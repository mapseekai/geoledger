use super::*;

pub(super) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
pub(super) fn table(b: &TableBinding) -> String {
    format!("{}.{}", quote(&b.schema), quote(&b.table))
}
pub(super) fn validate_binding(b: &TableBinding) -> Result<()> {
    for value in [&b.schema, &b.table, &b.id_column, &b.geometry_column] {
        text(value, 63)?;
    }
    if b.id_column == b.geometry_column || b.srid <= 0 {
        return Err(bad());
    }
    Ok(())
}
pub(super) fn inspect_for_publish(
    t: &mut Transaction<'_>,
    b: &TableBinding,
) -> Result<Vec<Column>> {
    let name = table(b);
    let row=t.query_one("SELECT c.relkind::text, EXISTS(SELECT 1 FROM pg_index i JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=ANY(i.indkey) WHERE i.indrelid=c.oid AND i.indisprimary AND i.indnkeyatts=1 AND a.attname=$2), EXISTS(SELECT 1 FROM pg_attribute a WHERE a.attrelid=c.oid AND a.attname=$3 AND a.atttypid='geometry'::regtype AND postgis_typmod_srid(a.atttypmod)=$4) FROM pg_class c WHERE c.oid=$1::text::regclass",&[&name,&b.id_column,&b.geometry_column,&b.srid])?;
    if row.get::<_, String>(0) != "r" || !row.get::<_, bool>(1) || !row.get::<_, bool>(2) {
        return Err(Error::new(
            422,
            "only ordinary tables with one primary key and fixed geometry SRID are supported",
        ));
    }
    let rows=t.query("SELECT attname,format_type(atttypid,atttypmod),NOT attnotnull,attgenerated='' FROM pg_attribute WHERE attrelid=$1::text::regclass AND attnum>0 AND NOT attisdropped AND attname<>$2 ORDER BY attnum",&[&name,&b.geometry_column])?;
    let mut columns = Vec::new();
    for row in rows {
        let name: String = row.get(0);
        let kind: String = row.get(1);
        // These lossless text input/output types cover ordinary vector properties.
        // Domains, arrays and extension objects need explicit adapters, never implicit coercion.
        if !supported(&kind) {
            return Err(Error::new(
                422,
                &format!("unsupported managed column type: {kind}"),
            ));
        }
        columns.push(Column {
            editable: name != b.id_column && row.get::<_, bool>(3),
            name,
            kind,
            nullable: row.get(2),
        });
    }
    columns.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(columns)
}
fn supported(kind: &str) -> bool {
    matches!(
        kind,
        "text"
            | "smallint"
            | "integer"
            | "bigint"
            | "real"
            | "double precision"
            | "numeric"
            | "boolean"
            | "date"
            | "timestamp without time zone"
            | "timestamp with time zone"
            | "json"
            | "jsonb"
            | "uuid"
            | "bytea"
    ) || ["character varying(", "character(", "numeric(", "timestamp("]
        .iter()
        .any(|p| kind.starts_with(p))
}
pub(super) fn new_column(name: &str, kind: &str) -> Result<Column> {
    text(name, 63)?;
    let kind = match kind.to_lowercase().as_str() {
        "string" | "varchar" | "text" => "text",
        "int" | "int32" | "integer" => "integer",
        "int64" | "bigint" => "bigint",
        "float" | "float64" | "number" | "double" | "double precision" => "double precision",
        "bool" | "boolean" => "boolean",
        "date" => "date",
        "datetime" | "timestamp" | "timestamptz" => "timestamp with time zone",
        "numeric" => "numeric",
        "json" => "json",
        "jsonb" => "jsonb",
        _ => return Err(Error::new(422, "unsupported added column type")),
    };
    Ok(Column {
        name: name.into(),
        kind: kind.into(),
        nullable: true,
        editable: true,
    })
}
pub(super) fn validate_drop(t: &mut Transaction<'_>, d: &Dataset, name: &str) -> Result<()> {
    let dependent:bool=t.query_one("SELECT EXISTS(SELECT 1 FROM pg_attribute a WHERE a.attrelid=$1::text::regclass AND a.attname=$2 AND (EXISTS(SELECT 1 FROM pg_attrdef f WHERE f.adrelid=a.attrelid AND f.adnum=a.attnum) OR EXISTS(SELECT 1 FROM pg_constraint c WHERE c.conrelid=a.attrelid AND a.attnum=ANY(c.conkey)) OR EXISTS(SELECT 1 FROM pg_index i WHERE i.indrelid=a.attrelid AND a.attnum=ANY(i.indkey))))",&[&table(&d.binding),&name])?.get(0);
    if dependent {
        return Err(Error::new(
            422,
            "dropping a field with defaults, indexes or constraints requires a host schema adapter",
        ));
    }
    Ok(())
}
pub(super) fn finish_schema(
    t: &mut Transaction<'_>,
    d: &Dataset,
    before: &[Column],
    after: &[Column],
) -> Result<()> {
    for column in after
        .iter()
        .filter(|c| !c.nullable && !before.iter().any(|b| b.name == c.name))
    {
        t.batch_execute(&format!(
            "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
            table(&d.binding),
            quote(&column.name)
        ))?;
    }
    Ok(())
}
fn json_object(fields: &[String]) -> String {
    if fields.is_empty() {
        return "'{}'::jsonb".into();
    }
    fields
        .chunks(40)
        .map(|chunk| format!("jsonb_build_object({})", chunk.join(",")))
        .collect::<Vec<_>>()
        .join(" || ")
}
fn native_expression(columns: &[Column], alias: &str) -> String {
    let fields = columns
        .iter()
        .map(|c| format!("{},{}.{}::text", literal(&c.name), alias, quote(&c.name)))
        .collect::<Vec<_>>();
    json_object(&fields)
}
fn exact_number(kind: &str) -> bool {
    kind == "bigint" || kind == "numeric" || kind.starts_with("numeric(")
}
fn feature_expression(b: &TableBinding, alias: &str, columns: &[Column]) -> String {
    let id = quote(&b.id_column);
    let geom = quote(&b.geometry_column);
    let properties = columns
        .iter()
        .map(|c| {
            format!(
                "{},{alias}.{}{}",
                literal(&c.name),
                quote(&c.name),
                if exact_number(&c.kind) { "::text" } else { "" }
            )
        })
        .collect::<Vec<_>>();
    let properties = json_object(&properties);
    format!(
        "jsonb_build_object('type','Feature','id',{alias}.{id}::text,'properties',({properties}),'geometry',CASE WHEN {alias}.{geom} IS NULL THEN NULL ELSE ST_AsGeoJSON(ST_Transform({alias}.{geom},4326),17,0)::jsonb END)"
    )
}
pub(super) fn register(t: &mut Transaction<'_>, scope: &Scope, b: &TableBinding) -> Result<()> {
    // Registration locks the physical table before reading either metadata or rows;
    // the initial history is one INSERT SELECT snapshot with no offset pagination.
    t.batch_execute(&format!(
        "LOCK TABLE {} IN SHARE ROW EXCLUSIVE MODE",
        table(b)
    ))?;
    if t.query_opt("SELECT 1 FROM _geoledger_center.collab_datasets WHERE tenant=$1 AND project=$2 AND dataset=$3",&[&scope.tenant,&scope.project,&scope.dataset])?.is_some(){return Ok(());}
    let columns = inspect_for_publish(t, b)?;
    let invalid_ids:bool=t.query_one(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE {}::text='' OR octet_length({}::text)>256 OR {}::text=$1 OR {}::text~$2)",table(b),quote(&b.id_column),quote(&b.id_column),quote(&b.id_column),quote(&b.id_column)),&[&"$schema",&"[\u{1}-\u{1f}\u{7f}-\u{9f}]"])?.get(0);
    if invalid_ids {
        return Err(Error::new(
            422,
            "source feature IDs must be nonempty, at most 256 bytes, without controls or reserved $schema",
        ));
    }
    let epoch = Uuid::new_v4().to_string();
    t.execute("INSERT INTO _geoledger_center.collab_datasets(tenant,project,dataset,epoch,source_schema,source_table,id_column,geometry_column,srid) VALUES($1,$2,$3,$4::text::uuid,$5,$6,$7,$8,$9)",&[&scope.tenant,&scope.project,&scope.dataset,&epoch,&b.schema,&b.table,&b.id_column,&b.geometry_column,&b.srid])?;
    t.execute(
        "INSERT INTO _geoledger_center.collab_schemas VALUES($1::text::uuid,0,$2::text::jsonb)",
        &[&epoch, &json!(columns).to_string()],
    )?;
    t.execute(&format!("INSERT INTO _geoledger_center.collab_rows(epoch,id,revision,value,feature) SELECT $1::text::uuid,r.{}::text,0,jsonb_build_object('properties',{},'geometry',encode(ST_AsEWKB(r.{}),'hex')),{} FROM {} r",quote(&b.id_column),native_expression(&columns,"r"),quote(&b.geometry_column),feature_expression(b,"r",&columns),table(b)),&[&epoch])?;
    let oversized:bool=t.query_one("SELECT EXISTS(SELECT 1 FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND octet_length(feature::text)>$2::bigint)",&[&epoch,&(FEATURE_BYTES as i64)])?.get(0);
    if oversized {
        return Err(Error::new(413, "a source feature exceeds 4 MiB"));
    }
    Ok(())
}
pub(super) fn capture_schema_rows(
    t: &mut Transaction<'_>,
    d: &Dataset,
    revision: i64,
    columns: &[Column],
) -> Result<()> {
    let b = &d.binding;
    t.execute(&format!("INSERT INTO _geoledger_center.collab_rows(epoch,id,revision,value,feature) SELECT $1::text::uuid,r.{}::text,$2,jsonb_build_object('properties',{},'geometry',encode(ST_AsEWKB(r.{}),'hex')),{} FROM {} r WHERE NOT EXISTS(SELECT 1 FROM _geoledger_center.collab_rows h WHERE h.epoch=$1::text::uuid AND h.id=r.{}::text AND h.revision=$2)",quote(&b.id_column),native_expression(columns,"r"),quote(&b.geometry_column),feature_expression(b,"r",columns),table(b),quote(&b.id_column)),&[&d.epoch,&revision])?;
    Ok(())
}
pub(super) fn at(
    t: &mut Transaction<'_>,
    d: &Dataset,
    key: &str,
    rev: i64,
) -> Result<Option<Stored>> {
    t.query_opt("SELECT value::text FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND id=$2 AND revision<=$3 ORDER BY revision DESC LIMIT 1",&[&d.epoch,&key,&rev])?.and_then(|r|r.get::<_,Option<String>>(0)).map(|s|parse(&s)).transpose()
}
pub(super) fn restorable(
    t: &mut Transaction<'_>,
    d: &Dataset,
    key: &str,
    revision: i64,
) -> Result<Stored> {
    let row=t.query_opt("SELECT value::text FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND id=$2 AND revision<=$3 AND value IS NOT NULL ORDER BY revision DESC LIMIT 1",&[&d.epoch,&key,&revision])?.ok_or_else(missing)?;
    parse(&row.get::<_, String>(0))
}
pub(super) fn current(
    t: &mut Transaction<'_>,
    d: &Dataset,
    key: &str,
    columns: &[Column],
) -> Result<Option<Stored>> {
    let kind = &columns
        .iter()
        .find(|c| c.name == d.binding.id_column)
        .ok_or_else(bad)?
        .kind;
    t.query_opt(&format!("SELECT jsonb_build_object('properties',{},'geometry',encode(ST_AsEWKB(r.{}),'hex'))::text FROM {} r WHERE r.{}=$1::text::{kind}",native_expression(columns,"r"),quote(&d.binding.geometry_column),table(&d.binding),quote(&d.binding.id_column)),&[&key])?.map(|r|parse(&r.get::<_,String>(0))).transpose()
}
pub(super) fn outward(
    t: &mut Transaction<'_>,
    key: &str,
    value: Option<&Stored>,
    columns: &[Column],
) -> Result<Value> {
    let Some(value) = value else {
        return Ok(Value::Null);
    };
    let properties = columns
        .iter()
        .map(|c| {
            format!(
                "{},($1::text::jsonb->>{})::{}",
                literal(&c.name),
                literal(&c.name),
                if exact_number(&c.kind) {
                    "text"
                } else {
                    &c.kind
                }
            )
        })
        .collect::<Vec<_>>();
    let properties = json_object(&properties);
    let row=t.query_one(&format!("SELECT ({properties})::text,CASE WHEN $2::text IS NULL THEN NULL ELSE ST_AsGeoJSON(ST_Transform(ST_GeomFromEWKB(decode($2,'hex')),4326),17,0) END"),&[&json!(value.properties).to_string(),&value.geometry])?;
    let properties: Value = parse(&row.get::<_, String>(0))?;
    let geometry = row
        .get::<_, Option<String>>(1)
        .map(|s| parse::<Value>(&s))
        .transpose()?
        .unwrap_or(Value::Null);
    let feature = json!({"type":"Feature","id":key,"properties":properties,"geometry":geometry});
    if feature.to_string().len() > FEATURE_BYTES {
        return Err(Error::new(413, "feature exceeds 4 MiB"));
    }
    Ok(feature)
}
pub(super) fn normalize_patch(
    t: &mut Transaction<'_>,
    d: &Dataset,
    key: &str,
    mut value: Stored,
    body: &Value,
    columns: &[Column],
    full: bool,
) -> Result<Stored> {
    let object = body.as_object().ok_or_else(bad)?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "type" | "id" | "properties" | "geometry"))
    {
        return Err(bad());
    }
    if full && body.get("type").and_then(Value::as_str) != Some("Feature") {
        return Err(bad());
    }
    if let Some(properties) = body.get("properties") {
        let properties = properties.as_object().ok_or_else(bad)?;
        for (name, input) in properties {
            let column = columns
                .iter()
                .find(|c| c.name == *name)
                .ok_or_else(|| Error::new(422, "unknown property; add the field explicitly"))?;
            if !column.editable {
                if name == &d.binding.id_column
                    && (input.as_str() == Some(key)
                        || key
                            .parse::<i64>()
                            .ok()
                            .is_some_and(|id| input.as_i64() == Some(id)))
                {
                    continue;
                }
                return Err(Error::new(422, "property is not editable"));
            }
            let native = if input.is_null() {
                Value::Null
            } else {
                let text = if matches!(column.kind.as_str(), "json" | "jsonb") {
                    input.to_string()
                } else if let Some(s) = input.as_str() {
                    s.to_owned()
                } else {
                    input.to_string()
                };
                let row = t.query_one(
                    &format!("SELECT ($1::text::{})::text", column.kind),
                    &[&text],
                )?;
                json!(row.get::<_, String>(0))
            };
            value.properties.insert(name.clone(), native);
        }
    }
    if let Some(geometry) = body.get("geometry") {
        value.geometry = if geometry.is_null() {
            None
        } else {
            geometry_shape(geometry)?;
            let row=t.query_one("WITH g AS (SELECT ST_SetSRID(ST_GeomFromGeoJSON($1::text),4326) geom) SELECT encode(ST_AsEWKB(ST_Transform(geom,$2::integer)),'hex'), ST_IsValid(geom) AND ST_NDims(geom) IN(2,3) AND NOT EXISTS(SELECT 1 FROM ST_DumpPoints(geom) p WHERE NOT(ST_X(p.geom) BETWEEN -180 AND 180 AND ST_Y(p.geom) BETWEEN -90 AND 90)) FROM g",&[&geometry.to_string(),&d.binding.srid]).map_err(Error::invalid_geometry)?;
            if !row.get::<_, bool>(1) {
                return Err(Error::new(422, "invalid geometry"));
            }
            Some(row.get(0))
        };
    } else if full {
        return Err(Error::new(
            400,
            "feature must specify geometry including null",
        ));
    }
    Ok(value)
}
pub(super) fn allocate(
    t: &mut Transaction<'_>,
    d: &Dataset,
    w: &str,
    client: &str,
    columns: &[Column],
) -> Result<String> {
    text(client, 256)?;
    if let Some(row)=t.query_opt("SELECT id FROM _geoledger_center.collab_ids WHERE workspace=$1::text::uuid AND client_id=$2",&[&w,&client])? {return Ok(row.get(0));}
    let kind = &columns
        .iter()
        .find(|c| c.name == d.binding.id_column)
        .ok_or_else(bad)?
        .kind;
    let sequence: Option<String> = t
        .query_one(
            "SELECT pg_get_serial_sequence($1,$2)",
            &[&table(&d.binding), &d.binding.id_column],
        )?
        .get(0);
    let key = if let Some(sequence) = sequence {
        t.query_one("SELECT nextval($1::text::regclass)::text", &[&sequence])?
            .get::<_, String>(0)
    } else if matches!(kind.as_str(), "uuid" | "text") {
        Uuid::new_v4().to_string()
    } else {
        return Err(Error::new(
            422,
            "new features require a sequence, uuid or text primary key",
        ));
    };
    let occupied:bool=t.query_one(&format!("SELECT EXISTS(SELECT 1 FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND id=$2) OR EXISTS(SELECT 1 FROM {} WHERE {}=$2::text::{kind}) OR EXISTS(SELECT 1 FROM _geoledger_center.collab_ids i JOIN _geoledger_center.collab_workspaces w ON w.id=i.workspace WHERE w.epoch=$1::text::uuid AND i.id=$2)",table(&d.binding),quote(&d.binding.id_column)),&[&d.epoch,&key])?.get(0);
    if occupied {
        return Err(Error::new(
            409,
            "generated primary key is already allocated; repair the key generator and retry",
        ));
    }
    t.execute(
        "INSERT INTO _geoledger_center.collab_ids VALUES($1::text::uuid,$2,$3)",
        &[&w, &client, &key],
    )?;
    Ok(key)
}
pub(super) fn write_row(
    t: &mut Transaction<'_>,
    d: &Dataset,
    key: &str,
    value: Option<&Stored>,
    columns: &[Column],
    existing: bool,
) -> Result<()> {
    let table = table(&d.binding);
    let id = quote(&d.binding.id_column);
    let geom = quote(&d.binding.geometry_column);
    let key_type = &columns
        .iter()
        .find(|c| c.name == d.binding.id_column)
        .ok_or_else(bad)?
        .kind;
    let Some(value) = value else {
        t.execute(
            &format!("DELETE FROM {table} WHERE {id}=$1::text::{key_type}"),
            &[&key],
        )?;
        return Ok(());
    };
    let props = json!(value.properties).to_string();
    let fields = columns
        .iter()
        .filter(|c| c.editable || c.name == d.binding.id_column)
        .collect::<Vec<_>>();
    if existing {
        let assignments=fields.iter().filter(|c|c.editable).map(|c|format!("{}=($1::text::jsonb->>{})::{}",quote(&c.name),literal(&c.name),c.kind)).chain(std::iter::once(format!("{geom}=CASE WHEN $2::text IS NULL THEN NULL ELSE ST_GeomFromEWKB(decode($2,'hex')) END"))).collect::<Vec<_>>().join(",");
        t.execute(
            &format!("UPDATE {table} SET {assignments} WHERE {id}=$3::text::{key_type}"),
            &[&props, &value.geometry, &key],
        )?;
    } else {
        let names = fields
            .iter()
            .filter(|c| value.properties.contains_key(&c.name))
            .map(|c| quote(&c.name))
            .chain(std::iter::once(geom))
            .collect::<Vec<_>>()
            .join(",");
        let values = fields
            .iter()
            .filter(|c| value.properties.contains_key(&c.name))
            .map(|c| format!("($1::text::jsonb->>{})::{}", literal(&c.name), c.kind))
            .chain(std::iter::once(
                "CASE WHEN $2::text IS NULL THEN NULL ELSE ST_GeomFromEWKB(decode($2,'hex')) END"
                    .into(),
            ))
            .collect::<Vec<_>>()
            .join(",");
        t.execute(
            &format!("INSERT INTO {table}({names}) OVERRIDING SYSTEM VALUE VALUES({values})"),
            &[&props, &value.geometry],
        )?;
    }
    Ok(())
}
pub(super) fn restore_selected_columns(
    t: &mut Transaction<'_>,
    d: &Dataset,
    base: i64,
    before: &[Column],
    after: &[Column],
) -> Result<()> {
    let original = schema_at(t, d, base)?;
    for column in after.iter().filter(|c| {
        !before.iter().any(|b| b.name == c.name) && original.iter().any(|b| b.name == c.name)
    }) {
        t.execute(&format!("UPDATE {} r SET {}=(SELECT h.value->'properties'->>{} FROM _geoledger_center.collab_rows h WHERE h.epoch=$1::text::uuid AND h.id=r.{}::text AND h.revision<=$2 ORDER BY h.revision DESC LIMIT 1)::{}",table(&d.binding),quote(&column.name),literal(&column.name),quote(&d.binding.id_column),column.kind),&[&d.epoch,&base])?;
    }
    Ok(())
}
pub(super) fn apply_schema(
    t: &mut Transaction<'_>,
    d: &Dataset,
    before: &[Column],
    after: &[Column],
) -> Result<()> {
    if after
        .iter()
        .any(|a| before.iter().any(|b| a.name == b.name && a.kind != b.kind))
    {
        return Err(Error::new(
            422,
            "column type conversion requires a new field; choose the remote schema or revise the draft",
        ));
    }
    for col in before
        .iter()
        .filter(|c| !after.iter().any(|a| a.name == c.name))
    {
        if !col.editable {
            return Err(bad());
        }
        t.batch_execute(&format!(
            "ALTER TABLE {} DROP COLUMN {}",
            table(&d.binding),
            quote(&col.name)
        ))?;
    }
    for col in after
        .iter()
        .filter(|c| !before.iter().any(|a| a.name == c.name))
    {
        t.batch_execute(&format!(
            "ALTER TABLE {} ADD COLUMN {} {}",
            table(&d.binding),
            quote(&col.name),
            col.kind
        ))?;
    }
    Ok(())
}
