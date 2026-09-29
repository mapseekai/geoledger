use super::*;
use geoledger_core::schema::{COLUMN_ID, SchemaEdit, column_id};

// Accept type declarations, never arbitrary SQL expressions or USING clauses.
pub(super) fn type_sql(input: &str) -> Result<String> {
    let input = input.trim().to_ascii_lowercase();
    if let Some(rest) = input.strip_prefix("timestamp(")
        && let Some((precision, suffix)) = rest.split_once(')')
        && [" with time zone", " without time zone"].contains(&suffix)
        && precision.parse::<u32>().is_ok()
    {
        return Ok(input);
    }
    let (base, args) = match input.split_once('(') {
        Some((base, rest)) if rest.ends_with(')') => (base.trim(), Some(&rest[..rest.len() - 1])),
        Some(_) => return Err(Error::Invalid("invalid type declaration".into())),
        None => (input.as_str(), None),
    };
    let allowed = [
        "smallint",
        "integer",
        "bigint",
        "int2",
        "int4",
        "int8",
        "real",
        "double precision",
        "float4",
        "float8",
        "numeric",
        "decimal",
        "boolean",
        "bool",
        "text",
        "varchar",
        "character varying",
        "uuid",
        "date",
        "timestamp",
        "timestamp without time zone",
        "timestamp with time zone",
        "timestamptz",
        "jsonb",
        "bytea",
        "geometry",
    ];
    if !allowed.contains(&base) {
        return Err(Error::Unsupported(format!("type {input}")));
    }
    if let Some(args) = args {
        let valid = if base == "geometry" {
            let parts: Vec<_> = args.split(',').map(str::trim).collect();
            !parts.is_empty()
                && parts.len() <= 2
                && parts[0].chars().all(|c| c.is_ascii_alphabetic())
                && parts.get(1).is_none_or(|s| s.parse::<i32>().is_ok())
        } else {
            [
                "numeric",
                "decimal",
                "varchar",
                "character varying",
                "timestamp",
            ]
            .contains(&base)
                && args.split(',').all(|p| p.trim().parse::<i32>().is_ok())
        };
        if !valid {
            return Err(Error::Invalid("invalid type parameters".into()));
        }
    }
    Ok(input)
}

// Catalog defaults may contain functions; only constants are replayable here.
fn default_sql(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    if ["true", "false", "NULL"].contains(&input)
        || (input
            .chars()
            .all(|c| c.is_ascii_digit() || "+-.eE".contains(c))
            && input.parse::<f64>().is_ok())
    {
        return Ok(input.into());
    }
    if let Some(rest) = input.strip_prefix('\'') {
        let mut chars = rest.char_indices().peekable();
        let mut value = String::new();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().is_some_and(|(_, c)| *c == '\'') {
                    chars.next();
                    value.push('\'');
                    continue;
                }
                let suffix = rest[i + 1..].trim();
                return if suffix.is_empty() {
                    Ok(literal(&value))
                } else if let Some(typ) = suffix.strip_prefix("::") {
                    Ok(format!("{}::{}", literal(&value), type_sql(typ)?))
                } else {
                    Err(Error::Unsupported("nonconstant column default".into()))
                };
            }
            value.push(c);
        }
    }
    Err(Error::Unsupported(
        "adding or restoring columns with expression defaults; only literal defaults are supported"
            .into(),
    ))
}

pub(super) fn positions(
    session: &mut PostgisTransaction,
    binding: &Binding,
) -> Result<BTreeMap<i16, String>> {
    let oid = session.table_oid(&binding.schema_name, &binding.table_name)?;
    let by_name: BTreeMap<_, _> = binding
        .schema
        .fields
        .iter()
        .map(|f| (&f.name, column_id(f)))
        .collect();
    let rows = session.client.query("SELECT attnum,attname FROM pg_attribute WHERE attrelid=$1::bigint::oid AND attnum>0 AND NOT attisdropped", &[&oid]).map_err(pg_error)?;
    rows.into_iter()
        .map(|r| {
            let name: String = r.get(1);
            let id = by_name
                .get(&name)
                .ok_or_else(|| Error::Storage("column mapping mismatch".into()))?;
            Ok((r.get(0), id.clone()))
        })
        .collect()
}

pub(super) fn current(session: &mut PostgisTransaction, binding: &Binding) -> Result<Schema> {
    let mut actual = session.inspect(&binding.schema_name, &binding.table_name)?;
    geoledger_core::schema::validate_format(&binding.schema)?;
    actual.version = FORMAT_VERSION;
    let oid = session.table_oid(&binding.schema_name, &binding.table_name)?;
    let rows = session.client.query("SELECT attnum,attname FROM pg_attribute WHERE attrelid=$1::bigint::oid AND attnum>0 AND NOT attisdropped", &[&oid]).map_err(pg_error)?;
    let numbers: BTreeMap<String, i16> = rows.into_iter().map(|r| (r.get(1), r.get(0))).collect();
    for f in &mut actual.fields {
        let pos = numbers
            .get(&f.name)
            .ok_or_else(|| Error::Storage("column position missing".into()))?;
        let id = binding
            .column_ids
            .get(pos)
            .cloned()
            .unwrap_or_else(|| format!("pg:{}:{oid}:{pos}", session.repository_id));
        f.metadata.insert(COLUMN_ID.into(), id);
    }
    let indexes: Vec<String> = session.client.query("SELECT pg_get_indexdef(indexrelid) FROM pg_index WHERE indrelid=$1::bigint::oid ORDER BY pg_get_indexdef(indexrelid)", &[&oid]).map_err(pg_error)?.into_iter().map(|r|r.get(0)).collect();
    actual
        .metadata
        .insert("postgres.indexes".into(), serde_json::to_string(&indexes)?);
    actual.fields.sort_by(|a, b| a.name.cmp(&b.name));
    if !binding.column_ids.is_empty() {
        validate_transition(&binding.schema, &actual)?;
    }
    Ok(actual)
}

// Rewrite SQL identifier tokens only. String literals are left intact.
fn renamed_definition(sql: &str, renames: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut chars = sql.chars().peekable();
    // pg_get_indexdef's header identifies the index, table and access method,
    // never a column. Within expressions, qualified names and call/cast names
    // must retain their identity when a column happens to have the same name.
    let mut index_header = sql.starts_with("CREATE ");
    let mut previous = None;
    let mut after_collate = false;
    while let Some(c) = chars.next() {
        if c == '\'' || c == '"' {
            let quote = c;
            let mut token = String::new();
            while let Some(c) = chars.next() {
                if c == quote {
                    if chars.peek() == Some(&quote) {
                        chars.next();
                        token.push(quote);
                    } else {
                        break;
                    }
                } else {
                    token.push(c);
                }
            }
            let next = chars.clone().find(|c| !c.is_whitespace());
            let column = !index_header
                && !after_collate
                && !matches!(previous, Some('.' | ':'))
                && !matches!(next, Some('(' | '.'));
            let token = if quote == '"' && column {
                renames.get(&token).unwrap_or(&token).clone()
            } else {
                token
            };
            out.push(quote);
            out.push_str(&token.replace(quote, &format!("{quote}{quote}")));
            out.push(quote);
            previous = Some(quote);
            after_collate = false;
        } else if c.is_alphabetic() || c == '_' {
            let mut token = c.to_string();
            while chars
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            {
                if let Some(c) = chars.next() {
                    token.push(c);
                }
            }
            let next = chars.clone().find(|c| !c.is_whitespace());
            let column = !index_header
                && !after_collate
                && !matches!(previous, Some('.' | ':'))
                && !matches!(next, Some('(' | '.'));
            let name = if column {
                renames.get(&token).unwrap_or(&token)
            } else {
                &token
            };
            out.push('"');
            out.push_str(&name.replace('"', "\"\""));
            out.push('"');
            after_collate = token.eq_ignore_ascii_case("COLLATE");
            previous = Some('x');
        } else {
            if c == '(' {
                index_header = false;
            }
            if !c.is_whitespace() {
                previous = Some(c);
            }
            out.push(c);
        }
    }
    out
}

pub(super) fn validate_transition(before: &Schema, after: &Schema) -> Result<()> {
    if before.primary_key != after.primary_key || key_field(before)? != key_field(after)? {
        return Err(Error::Unsupported("primary-key schema changes".into()));
    }
    let by_id: BTreeMap<_, _> = after.fields.iter().map(|f| (column_id(f), f)).collect();
    let mut renames = BTreeMap::new();
    let before_ids: BTreeSet<_> = before.fields.iter().map(column_id).collect();
    for field in before
        .fields
        .iter()
        .filter(|f| !by_id.contains_key(&column_id(f)))
        .chain(
            after
                .fields
                .iter()
                .filter(|f| !before_ids.contains(&column_id(f))),
        )
    {
        default_sql(
            field
                .metadata
                .get("postgres.default")
                .map_or("", String::as_str),
        )?;
    }
    for old in &before.fields {
        if let Some(new) = by_id.get(&column_id(old)) {
            if old.nullable != new.nullable
                || old.metadata.get("postgres.default") != new.metadata.get("postgres.default")
            {
                return Err(Error::Unsupported(
                    "changing existing defaults or nullability".into(),
                ));
            }
            if old.name != new.name {
                renames.insert(old.name.clone(), new.name.clone());
            }
        }
    }
    for key in ["postgres.constraints", "postgres.indexes"] {
        if let (Some(old), Some(new)) = (before.metadata.get(key), after.metadata.get(key)) {
            let mut old: Vec<String> = serde_json::from_str(old)?;
            let mut new: Vec<String> = serde_json::from_str(new)?;
            old = old
                .into_iter()
                .map(|s| renamed_definition(&s, &renames))
                .collect();
            new = new
                .into_iter()
                .map(|s| renamed_definition(&s, &BTreeMap::new()))
                .collect();
            old.sort();
            new.sort();
            if old != new {
                return Err(Error::Unsupported("schema edit changes dependent constraints or indexes; no dependencies are silently discarded".into()));
            }
        }
    }
    Ok(())
}

pub(super) fn edit(
    session: &mut PostgisTransaction,
    binding: &Binding,
    edit: &SchemaEdit,
) -> Result<Schema> {
    let table = table(&binding.schema_name, &binding.table_name)?;
    let name = match edit {
        SchemaEdit::Add { name, .. }
        | SchemaEdit::Drop { name, .. }
        | SchemaEdit::Rename { name, .. }
        | SchemaEdit::AlterType { name, .. } => name,
    };
    if name == &binding.schema.primary_key {
        return Err(Error::Unsupported("primary-key schema changes".into()));
    }
    let col = ident(name)?;
    let statement = match edit {
        SchemaEdit::Add { data_type, .. } => format!(
            "ALTER TABLE {table} ADD COLUMN {col} {}",
            type_sql(data_type)?
        ),
        SchemaEdit::Drop { discard: false, .. } => {
            return Err(Error::Invalid(
                "drop field requires discard=true / --discard".into(),
            ));
        }
        SchemaEdit::Drop { .. } => format!("ALTER TABLE {table} DROP COLUMN {col} RESTRICT"),
        SchemaEdit::Rename { new_name, .. } => format!(
            "ALTER TABLE {table} RENAME COLUMN {col} TO {}",
            ident(new_name)?
        ),
        SchemaEdit::AlterType { data_type, .. } => {
            let typ = type_sql(data_type)?;
            format!("ALTER TABLE {table} ALTER COLUMN {col} TYPE {typ} USING {col}::{typ}")
        }
    };
    session.client.batch_execute(&statement).map_err(pg_error)?;
    let after = current(session, binding)?;
    validate_transition(&binding.schema, &after)?;
    Ok(after)
}

pub(super) fn replace(
    session: &mut PostgisTransaction,
    binding: &Binding,
    target: &Schema,
) -> Result<()> {
    // Historic rows are restored from objects, never obtained by reverse-casting
    // lossy converted values. DELETE keeps all working-copy triggers active.
    let table = table(&binding.schema_name, &binding.table_name)?;
    geoledger_core::schema::validate_format(&binding.schema)?;
    geoledger_core::schema::validate_format(target)?;
    let before = binding.schema.clone();
    let target_ids = target.clone();
    validate_transition(&before, &target_ids)?;
    session
        .client
        .batch_execute(&format!("DELETE FROM {table}"))
        .map_err(pg_error)?;
    let desired: BTreeMap<_, _> = target_ids
        .fields
        .iter()
        .map(|f| (column_id(f), f))
        .collect();
    let mut existing = BTreeMap::new();
    for f in &before.fields {
        let id = column_id(f);
        if !desired.contains_key(&id) {
            session
                .client
                .batch_execute(&format!(
                    "ALTER TABLE {table} DROP COLUMN {} RESTRICT",
                    ident(&f.name)?
                ))
                .map_err(pg_error)?;
        } else {
            existing.insert(id, f.clone());
        }
    }
    for (id, f) in &mut existing {
        if desired.get(id).is_some_and(|target| target.name != f.name) {
            let temp = format!("gl_rename_{}", blake3::hash(id.as_bytes()).to_hex());
            let temp = &temp[..60];
            session
                .client
                .batch_execute(&format!(
                    "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                    ident(&f.name)?,
                    ident(temp)?
                ))
                .map_err(pg_error)?;
            f.name = temp.into();
        }
    }
    for f in &target_ids.fields {
        let name = ident(&f.name)?;
        let typ = type_sql(native(f)?)?;
        if let Some(old) = existing.get(&column_id(f)) {
            if old.name != f.name {
                session
                    .client
                    .batch_execute(&format!(
                        "ALTER TABLE {table} RENAME COLUMN {} TO {name}",
                        ident(&old.name)?
                    ))
                    .map_err(pg_error)?;
            }
            if native(old)? != native(f)? {
                session
                    .client
                    .batch_execute(&format!(
                        "ALTER TABLE {table} ALTER COLUMN {name} TYPE {typ} USING {name}::{typ}"
                    ))
                    .map_err(pg_error)?;
            }
        } else {
            let default = default_sql(
                f.metadata
                    .get("postgres.default")
                    .map_or("", String::as_str),
            )?;
            let default = if default.is_empty() {
                default
            } else {
                format!(" DEFAULT {default}")
            };
            session
                .client
                .batch_execute(&format!(
                    "ALTER TABLE {table} ADD COLUMN {name} {typ} {}{default}",
                    if f.nullable { "" } else { "NOT NULL" }
                ))
                .map_err(pg_error)?;
        }
    }
    let mut target_binding = binding.clone();
    target_binding.schema = target.clone();
    target_binding.column_ids = positions(session, &target_binding)?;
    let actual = current(session, &target_binding)?;
    if !geoledger_core::schema::equivalent(&actual, target) {
        return Err(Error::Unsupported(
            "historical schema cannot be reproduced exactly".into(),
        ));
    }
    Ok(())
}

/// Defaults are evaluated only for newly introduced columns, once per projection.
pub(super) fn projection_defaults(
    session: &mut PostgisTransaction,
    from: &Schema,
    to: &Schema,
) -> Result<BTreeMap<String, Cell>> {
    let present: BTreeSet<_> = from.fields.iter().map(column_id).collect();
    let fields: Vec<_> = to
        .fields
        .iter()
        .filter(|f| !present.contains(&column_id(f)))
        .cloned()
        .collect();
    if fields.is_empty() {
        return Ok(BTreeMap::new());
    }
    let expressions = fields
        .iter()
        .map(|f| {
            let default = default_sql(
                f.metadata
                    .get("postgres.default")
                    .map_or("", String::as_str),
            )?;
            Ok(format!(
                "({})::{} AS {}",
                if default.is_empty() { "NULL" } else { &default },
                type_sql(native(f)?)?,
                ident(&f.name)?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let schema = Schema {
        fields,
        ..to.clone()
    };
    let query = format!(
        "SELECT {} FROM (SELECT {}) r",
        projection(&schema)?,
        expressions.join(",")
    );
    let row = session.client.query_one(&query, &[]).map_err(pg_error)?;
    schema
        .fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let value: Option<String> = row.try_get(i).map_err(pg_error)?;
            Ok((
                f.name.clone(),
                match value {
                    None => Cell::Null,
                    Some(v) if f.geometry => Cell::Geometry(v),
                    Some(v) => Cell::Text(v),
                },
            ))
        })
        .collect()
}
