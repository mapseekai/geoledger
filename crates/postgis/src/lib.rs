//! PostGIS v1 working-copy adapter. Uses one database transaction and locks all
//! tracked tables in stable order. Triggers remain enabled during checkout.
mod schema_ops;
use fallible_iterator::FallibleIterator;
use geoledger_core::{adapter::*, *};
use postgres::{Client, Config, Row, types::ToSql};
use postgres_native_tls::MakeTlsConnector;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
    time::Duration,
};

pub struct PostgisProvider {
    connection_string: String,
    statement_timeout_ms: u32,
}
impl PostgisProvider {
    pub fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            statement_timeout_ms: 120_000,
        }
    }
    /// Set the transaction-local timeout for each SQL statement (default: 120 seconds).
    ///
    /// # Errors
    /// Rejects durations outside 1..=i32::MAX milliseconds and fractional milliseconds.
    /// Zero is rejected because PostgreSQL interprets it as disabling the timeout.
    pub fn with_statement_timeout(mut self, timeout: Duration) -> Result<Self> {
        let millis = timeout.as_millis();
        if millis == 0
            || millis > i32::MAX as u128
            || !timeout.subsec_nanos().is_multiple_of(1_000_000)
        {
            return Err(Error::Invalid(
                "statement timeout must be a whole number of milliseconds in 1..=2147483647".into(),
            ));
        }
        self.statement_timeout_ms = millis as u32;
        Ok(self)
    }

    fn start_transaction(&self, client: &mut Client) -> Result<()> {
        client.batch_execute("BEGIN; SET LOCAL lock_timeout='5s';
            SET LOCAL timezone='UTC'; SET LOCAL datestyle='ISO, YMD'; SET LOCAL intervalstyle='iso_8601';
            SET LOCAL bytea_output='hex'; SET LOCAL extra_float_digits=3;
            SET LOCAL standard_conforming_strings=on;").map_err(pg_error)?;
        client
            .query_one(
                "SELECT set_config('statement_timeout', $1, true)",
                &[&format!("{}ms", self.statement_timeout_ms)],
            )
            .map_err(pg_error)?;
        Ok(())
    }
    pub fn from_env(name: &str) -> Result<Self> {
        std::env::var(name)
            .map(Self::new)
            .map_err(|_| Error::Invalid(format!("set {name} to a PostgreSQL connection string")))
    }
}
// Intentionally no Debug: connection strings may contain passwords.
fn pg_error(error: postgres::Error) -> Error {
    let message = match error.as_db_error() {
        Some(db) => format!("SQLSTATE {}: {}", db.code().code(), db.message()),
        None => "connection or protocol failure; connection details are redacted".into(),
    };
    Error::database_source(message, error)
}
fn ident(value: &str) -> Result<String> {
    if value.is_empty() || value.len() > 63 || value.contains('\0') {
        return Err(Error::Invalid("invalid PostgreSQL identifier".into()));
    }
    Ok(format!("\"{}\"", value.replace('"', "\"\"")))
}
fn table(schema: &str, name: &str) -> Result<String> {
    Ok(format!("{}.{}", ident(schema)?, ident(name)?))
}
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn native(field: &Field) -> Result<&str> {
    field
        .metadata
        .get("postgres.type")
        .map(String::as_str)
        .ok_or_else(|| Error::Invalid("missing PostgreSQL type metadata".into()))
}
fn key_field(schema: &Schema) -> Result<&Field> {
    schema
        .fields
        .iter()
        .find(|f| f.name == schema.primary_key)
        .ok_or_else(|| Error::Invalid("primary-key field is missing".into()))
}
fn projection(schema: &Schema) -> Result<String> {
    schema
        .fields
        .iter()
        .map(|f| {
            let column = format!("r.{}", ident(&f.name)?);
            Ok(if f.geometry {
                format!("encode(ST_AsEWKB({column}, 'XDR'), 'hex')")
            } else {
                format!("{column}::text")
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(|v| v.join(","))
}
fn decode(schema: &Schema, row: &Row) -> Result<Record> {
    let mut fields = BTreeMap::new();
    let mut key = None;
    for (i, field) in schema.fields.iter().enumerate() {
        let value: Option<String> = row.try_get(i).map_err(pg_error)?;
        if field.name == schema.primary_key {
            key = value.clone();
        }
        let cell = match value {
            None => Cell::Null,
            Some(v) if field.geometry => Cell::Geometry(v),
            Some(v) => Cell::Text(v),
        };
        fields.insert(field.name.clone(), cell);
    }
    let record = Record {
        key: key.ok_or_else(|| Error::Database("NULL primary key".into()))?,
        fields,
    };
    schema.validate(&record)?;
    Ok(record)
}
struct PostgisTransaction {
    client: Client,
    repository_id: String,
    active: bool,
}
impl WorkingCopyProvider for PostgisProvider {
    fn name(&self) -> &'static str {
        "postgis"
    }
    fn begin(
        &self,
        repository_id: &str,
        initial_head: &ObjectId,
        bindings: &BTreeMap<String, Binding>,
        extra_table: Option<(&str, &str)>,
    ) -> Result<Box<dyn WorkingCopyTransaction>> {
        let mut config = Config::from_str(&self.connection_string).map_err(|_| {
            Error::Invalid("invalid PostgreSQL connection string (redacted)".into())
        })?;
        config
            .connect_timeout(Duration::from_secs(10))
            .application_name("geoledger");
        let tls = native_tls::TlsConnector::builder()
            .build()
            .map_err(|_| Error::Database("failed to initialize TLS".into()))?;
        let mut client = config
            .connect(MakeTlsConnector::new(tls))
            .map_err(pg_error)?;
        self.start_transaction(&mut client)?;
        let mut session = PostgisTransaction {
            client,
            repository_id: repository_id.into(),
            active: true,
        };
        let exists: bool = session
            .client
            .query_one(
                "SELECT to_regclass('_geoledger.repositories') IS NOT NULL",
                &[],
            )
            .map_err(pg_error)?
            .get(0);
        if !exists {
            session
                .client
                .query_one("SELECT pg_advisory_xact_lock(1196180531::bigint)", &[])
                .map_err(pg_error)?;
            session
                .client
                .batch_execute(include_str!("bootstrap.sql"))
                .map_err(pg_error)?;
        }
        let version: i32 = session
            .client
            .query_one("SELECT version FROM _geoledger.format", &[])
            .map_err(pg_error)?
            .get(0);
        if version != FORMAT_VERSION as i32 {
            return Err(Error::Unsupported("PostGIS tracking schema version".into()));
        }
        let ordered_index: bool = session
            .client
            .query_one(
                "SELECT to_regclass('_geoledger.dirty_pk_c_v3') IS NOT NULL",
                &[],
            )
            .map_err(pg_error)?
            .get(0);
        if !ordered_index {
            return Err(Error::Recovery(
                "GeoLedger tracking index is missing".into(),
            ));
        }
        let hash = blake3::hash(repository_id.as_bytes());
        let mut key_bytes = [0u8; 8];
        key_bytes.copy_from_slice(&hash.as_bytes()[..8]);
        let lock_key = i64::from_be_bytes(key_bytes);
        session
            .client
            .query_one("SELECT pg_advisory_xact_lock($1)", &[&lock_key])
            .map_err(pg_error)?;
        let marker = session
            .client
            .query_opt(
                "SELECT head FROM _geoledger.repositories WHERE id=$1",
                &[&repository_id],
            )
            .map_err(pg_error)?;
        if marker.is_none() {
            if !bindings.is_empty() {
                return Err(Error::Recovery(
                    "this database is not the registered working copy".into(),
                ));
            }
            session
                .client
                .execute(
                    "INSERT INTO _geoledger.repositories(id,head) VALUES($1,$2)",
                    &[&repository_id, &initial_head.as_str()],
                )
                .map_err(pg_error)?;
        }
        let mut tables: BTreeSet<(String, String)> = bindings
            .values()
            .map(|b| (b.schema_name.clone(), b.table_name.clone()))
            .collect();
        if let Some((schema, name)) = extra_table {
            tables.insert((schema.into(), name.into()));
        }
        for (schema, name) in tables {
            session
                .client
                .batch_execute(&format!(
                    "LOCK TABLE {} IN SHARE ROW EXCLUSIVE MODE",
                    table(&schema, &name)?
                ))
                .map_err(pg_error)?;
        }
        Ok(Box::new(session))
    }
}
impl PostgisTransaction {
    fn table_oid(&mut self, schema: &str, name: &str) -> Result<i64> {
        self.client.query_opt("SELECT c.oid::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2",&[&schema,&name]).map_err(pg_error)?
            .map(|r|r.get(0)).ok_or_else(||Error::NotFound(format!("table {schema}.{name}")))
    }
}
impl WorkingCopyTransaction for PostgisTransaction {
    fn column_ids(&mut self, binding: &Binding) -> Result<BTreeMap<i16, String>> {
        schema_ops::positions(self, binding)
    }
    fn current_schema(&mut self, binding: &Binding) -> Result<Schema> {
        schema_ops::current(self, binding)
    }
    fn edit_schema(
        &mut self,
        binding: &Binding,
        edit: &geoledger_core::schema::SchemaEdit,
    ) -> Result<Schema> {
        schema_ops::edit(self, binding, edit)
    }
    fn replace_schema(&mut self, binding: &Binding, target: &Schema) -> Result<()> {
        schema_ops::replace(self, binding, target)
    }
    fn marker(&mut self) -> Result<DatabaseMarker> {
        let r = self
            .client
            .query_one(
                "SELECT head,operation FROM _geoledger.repositories WHERE id=$1",
                &[&self.repository_id],
            )
            .map_err(pg_error)?;
        Ok(DatabaseMarker {
            head: r.get(0),
            operation: r.get(1),
        })
    }
    fn inspect(&mut self, schema_name: &str, table_name: &str) -> Result<Schema> {
        table(schema_name, table_name)?;
        if schema_name == "_geoledger"
            || schema_name.starts_with("pg_")
            || schema_name == "information_schema"
        {
            return Err(Error::Invalid(
                "system and tracking schemas cannot be imported".into(),
            ));
        }
        let oid = self.table_oid(schema_name, table_name)?;
        let flags = self.client.query_one("SELECT relkind::text,relrowsecurity,EXISTS(SELECT 1 FROM pg_inherits WHERE inhrelid=c.oid OR inhparent=c.oid) FROM pg_class c WHERE oid=$1::bigint::oid",&[&oid]).map_err(pg_error)?;
        if flags.get::<_, String>(0) != "r" || flags.get::<_, bool>(1) || flags.get::<_, bool>(2) {
            return Err(Error::Unsupported(
                "views, partitioned/inherited tables and row-level security".into(),
            ));
        }
        let foreign_keys: bool = self.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_constraint WHERE contype='f' AND (conrelid=$1::bigint::oid OR confrelid=$1::bigint::oid))",&[&oid]).map_err(pg_error)?.get(0);
        if foreign_keys {
            return Err(Error::Unsupported(
                "foreign-key relationships in tracked tables".into(),
            ));
        }
        let custom_triggers: bool = self.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid=$1::bigint::oid AND NOT tgisinternal AND tgname NOT IN ('gl_track_row_v3','gl_reject_truncate_v3'))",&[&oid]).map_err(pg_error)?.get(0);
        if custom_triggers {
            return Err(Error::Unsupported("user triggers on tracked tables".into()));
        }
        let keys = self.client.query("SELECT a.attname FROM pg_index i JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=ANY(i.indkey) WHERE i.indrelid=$1::bigint::oid AND i.indisprimary ORDER BY a.attnum",&[&oid]).map_err(pg_error)?;
        if keys.len() != 1 {
            return Err(Error::Unsupported(
                "exactly one primary-key column is required".into(),
            ));
        }
        let primary_key: String = keys[0].get(0);
        let columns=self.client.query("SELECT a.attname,t.typname,format_type(a.atttypid,a.atttypmod),NOT a.attnotnull,
            a.attidentity::text,a.attgenerated::text,COALESCE(pg_get_expr(d.adbin,d.adrelid),''),
            a.attcollation <> t.typcollation
            FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid
            LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
            WHERE a.attrelid=$1::bigint::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",&[&oid]).map_err(pg_error)?;
        let mut fields = Vec::new();
        let supported = [
            "int2",
            "int4",
            "int8",
            "float4",
            "float8",
            "numeric",
            "bool",
            "text",
            "varchar",
            "uuid",
            "date",
            "timestamp",
            "timestamptz",
            "jsonb",
            "bytea",
            "geometry",
        ];
        for r in columns {
            let name: String = r.get(0);
            let typ: String = r.get(1);
            let native_type: String = r.get(2);
            if !supported.contains(&typ.as_str()) {
                return Err(Error::Unsupported(format!(
                    "PostgreSQL type {typ} in field {name}"
                )));
            }
            // Schema v1/v2 do not encode column collations. Never accept a
            // definition that historical ADD COLUMN would silently change.
            if r.get::<_, bool>(7) {
                return Err(Error::Unsupported(format!(
                    "non-default collation in field {name}; schema v1/v2 cannot restore it"
                )));
            }
            if !r.get::<_, String>(4).is_empty() || !r.get::<_, String>(5).is_empty() {
                return Err(Error::Unsupported("identity/generated columns".into()));
            }
            if name == primary_key
                && !["int2", "int4", "int8", "text", "varchar", "uuid"].contains(&typ.as_str())
            {
                return Err(Error::Unsupported(
                    "primary key must be integer, text/varchar or UUID".into(),
                ));
            }
            let geometry = typ == "geometry";
            fields.push(Field {
                name,
                logical_type: typ,
                codec: if geometry {
                    "ewkb-xdr/v1".into()
                } else {
                    "postgres-text/v1".into()
                },
                nullable: r.get(3),
                geometry,
                metadata: BTreeMap::from([
                    ("postgres.type".into(), native_type),
                    ("postgres.default".into(), r.get(6)),
                ]),
            });
        }
        let constraints:Vec<String>=self.client.query("SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid=$1::bigint::oid ORDER BY contype,pg_get_constraintdef(oid)",&[&oid]).map_err(pg_error)?.into_iter().map(|r|r.get(0)).collect();
        Ok(Schema {
            version: FORMAT_VERSION,
            kind: if fields.iter().any(|f| f.geometry) {
                DatasetKind::Vector
            } else {
                DatasetKind::Table
            },
            primary_key,
            fields,
            metadata: BTreeMap::from([(
                "postgres.constraints".into(),
                serde_json::to_string(&constraints)?,
            )]),
        })
    }
    fn register(&mut self, dataset: &str, binding: &Binding) -> Result<()> {
        let oid = self.table_oid(&binding.schema_name, &binding.table_name)?;
        if self
            .client
            .query_opt(
                "SELECT repository_id FROM _geoledger.tracked WHERE table_oid=$1::bigint::oid",
                &[&oid],
            )
            .map_err(pg_error)?
            .is_some()
        {
            return Err(Error::Conflict(
                "table is already tracked by a repository".into(),
            ));
        }
        self.client.execute("INSERT INTO _geoledger.tracked(table_oid,repository_id,dataset) VALUES($1::bigint::oid,$2,$3)",&[&oid,&self.repository_id,&dataset]).map_err(pg_error)?;
        let table = table(&binding.schema_name, &binding.table_name)?;
        self.client
            .batch_execute(&format!(
                "CREATE TRIGGER gl_track_row_v3 AFTER INSERT OR UPDATE OR DELETE ON {table}
            FOR EACH ROW EXECUTE FUNCTION _geoledger.track_row_v3({},{},{});
            CREATE TRIGGER gl_reject_truncate_v3 BEFORE TRUNCATE ON {table}
            FOR EACH STATEMENT EXECUTE FUNCTION _geoledger.reject_truncate_v3();",
                literal(&self.repository_id),
                literal(dataset),
                literal(&binding.schema.primary_key)
            ))
            .map_err(pg_error)?;
        Ok(())
    }
    fn verify(&mut self, dataset: &str, binding: &Binding) -> Result<()> {
        geoledger_core::schema::validate_format(&binding.schema)?;
        self.inspect(&binding.schema_name, &binding.table_name)?;
        let oid = self.table_oid(&binding.schema_name, &binding.table_name)?;
        let tracked=self.client.query_opt("SELECT repository_id,dataset FROM _geoledger.tracked WHERE table_oid=$1::bigint::oid",&[&oid]).map_err(pg_error)?;
        if tracked.is_none_or(|r| {
            r.get::<_, String>(0) != self.repository_id || r.get::<_, String>(1) != dataset
        }) {
            return Err(Error::Recovery(format!(
                "missing or mismatched tracking for {dataset}"
            )));
        }
        let count:i64=self.client.query_one("SELECT count(*) FROM pg_trigger WHERE tgrelid=$1::bigint::oid AND tgenabled IN ('O','A') AND
            ((tgname='gl_track_row_v3' AND tgfoid='_geoledger.track_row_v3'::regproc AND tgtype=29) OR
             (tgname='gl_reject_truncate_v3' AND tgfoid='_geoledger.reject_truncate_v3'::regproc AND tgtype=34))",&[&oid]).map_err(pg_error)?.get(0);
        if count != 2 {
            return Err(Error::Recovery(format!(
                "tracking triggers missing/disabled for {dataset}"
            )));
        }
        Ok(())
    }
    fn scan(
        &mut self,
        binding: &Binding,
        visit: &mut dyn FnMut(Record) -> Result<()>,
    ) -> Result<()> {
        let query = format!(
            "SELECT {} FROM {} r ORDER BY r.{}::text COLLATE \"C\"",
            projection(&binding.schema)?,
            table(&binding.schema_name, &binding.table_name)?,
            ident(&binding.schema.primary_key)?
        );
        let mut rows = self
            .client
            .query_raw(&query, std::iter::empty::<&(dyn ToSql + Sync)>())
            .map_err(pg_error)?;
        while let Some(row) = rows.next().map_err(pg_error)? {
            visit(decode(&binding.schema, &row)?)?;
        }
        Ok(())
    }
    fn dirty_keys(&mut self, dataset: &str) -> Result<Vec<String>> {
        Ok(self.client.query("SELECT pk FROM _geoledger.dirty WHERE repository_id=$1 AND dataset=$2 ORDER BY pk COLLATE \"C\"",&[&self.repository_id,&dataset]).map_err(pg_error)?.into_iter().map(|r|r.get(0)).collect())
    }
    fn dirty_keys_page(
        &mut self,
        dataset: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let limit = i64::try_from(limit)
            .map_err(|_| Error::Invalid("dirty page limit too large".into()))?;
        self.client.query("SELECT pk FROM _geoledger.dirty WHERE repository_id=$1 AND dataset=$2 AND ($3::text IS NULL OR pk COLLATE \"C\" > $3 COLLATE \"C\") ORDER BY pk COLLATE \"C\" LIMIT $4", &[&self.repository_id, &dataset, &after, &limit])
            .map_err(pg_error).map(|rows| rows.into_iter().map(|r| r.get(0)).collect())
    }
    fn projection_defaults(
        &mut self,
        from: &Schema,
        to: &Schema,
    ) -> Result<BTreeMap<String, Cell>> {
        schema_ops::projection_defaults(self, from, to)
    }
    fn normalize_many(&mut self, binding: &Binding, records: &[Record]) -> Result<Vec<Record>> {
        let schema = &binding.schema;
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let mut values = Vec::with_capacity(records.len());
        for record in records {
            schema.validate(record)?;
            values.push(
                record
                    .fields
                    .iter()
                    .map(|(name, cell)| {
                        (
                            name.as_str(),
                            match cell {
                                Cell::Text(v) | Cell::Geometry(v) => Some(v.as_str()),
                                _ => None,
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>(),
            );
        }
        let expressions = schema
            .fields
            .iter()
            .map(|f| {
                let value = format!("v->>{}", literal(&f.name));
                let expr = if f.geometry {
                    format!("ST_GeomFromEWKB(decode({value},'hex'))::{}", native(f)?)
                } else {
                    format!("({value})::{}", native(f)?)
                };
                Ok(format!("{expr} AS {}", ident(&f.name)?))
            })
            .collect::<Result<Vec<_>>>()?;
        let query = format!(
            "SELECT {} FROM jsonb_array_elements($1::text::jsonb) WITH ORDINALITY AS input(v,ordinal) CROSS JOIN LATERAL (SELECT {}) r ORDER BY input.ordinal",
            projection(schema)?,
            expressions.join(",")
        );
        let rows = self
            .client
            .query(&query, &[&serde_json::to_string(&values)?])
            .map_err(pg_error)?;
        if rows.len() != records.len() {
            return Err(Error::Database("normalization row count mismatch".into()));
        }
        rows.iter()
            .zip(records)
            .map(|(row, original)| {
                let normalized = decode(schema, row)?;
                if normalized.key != original.key {
                    return Err(Error::Invalid(
                        "normalization changed record identity".into(),
                    ));
                }
                Ok(normalized)
            })
            .collect()
    }
    fn read(&mut self, binding: &Binding, key: &str) -> Result<Option<Record>> {
        let query = format!(
            "SELECT {} FROM {} r WHERE r.{}=($1::text)::{}",
            projection(&binding.schema)?,
            table(&binding.schema_name, &binding.table_name)?,
            ident(&binding.schema.primary_key)?,
            native(key_field(&binding.schema)?)?
        );
        self.client
            .query_opt(&query, &[&key])
            .map_err(pg_error)?
            .map(|r| decode(&binding.schema, &r))
            .transpose()
    }
    fn read_many(
        &mut self,
        binding: &Binding,
        keys: &[String],
    ) -> Result<BTreeMap<String, Record>> {
        let schema = &binding.schema;
        let query = format!(
            "SELECT {} FROM {} r WHERE r.{} IN (SELECT v::{} FROM unnest($1::text[]) AS v)",
            projection(schema)?,
            table(&binding.schema_name, &binding.table_name)?,
            ident(&schema.primary_key)?,
            native(key_field(schema)?)?
        );
        let mut records = BTreeMap::new();
        for keys in keys.chunks(1000) {
            for row in self.client.query(&query, &[&keys]).map_err(pg_error)? {
                let r = decode(schema, &row)?;
                records.insert(r.key.clone(), r);
            }
        }
        Ok(records)
    }
    fn read_many_bounded(
        &mut self,
        binding: &Binding,
        keys: &[String],
        byte_limit: usize,
    ) -> Result<Vec<(String, Option<Record>)>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let schema = &binding.schema;
        let pk_index = schema
            .fields
            .iter()
            .position(|f| f.name == schema.primary_key)
            .ok_or_else(|| Error::Invalid("primary-key field is missing".into()))?;
        let query = format!(
            "SELECT {} FROM unnest($1::text[]) WITH ORDINALITY AS requested(key,ordinal) LEFT JOIN {} r ON r.{} = requested.key::{} ORDER BY requested.ordinal",
            projection(schema)?,
            table(&binding.schema_name, &binding.table_name)?,
            ident(&schema.primary_key)?,
            native(key_field(schema)?)?
        );
        let params: [&(dyn ToSql + Sync); 1] = [&keys];
        let mut rows = self.client.query_raw(&query, params).map_err(pg_error)?;
        let mut batch = Vec::new();
        let mut bytes = 0;
        for key in keys {
            let row = rows
                .next()
                .map_err(pg_error)?
                .ok_or_else(|| Error::Database("bounded read row count mismatch".into()))?;
            let pk: Option<String> = row.try_get(pk_index).map_err(pg_error)?;
            let record = pk.map(|_| decode(schema, &row)).transpose()?;
            bytes += key.len() + record.as_ref().map_or(0, Record::payload_bytes);
            batch.push((key.clone(), record));
            if bytes >= byte_limit {
                break;
            }
        }
        Ok(batch)
    }
    fn write_many(&mut self, binding: &Binding, values: &[(&str, Option<&Record>)]) -> Result<()> {
        let schema = &binding.schema;
        let table = table(&binding.schema_name, &binding.table_name)?;
        let pk = ident(&schema.primary_key)?;
        let mut deletes = Vec::new();
        let mut inserts = Vec::new();
        for (key, value) in values {
            if let Some(record) = value {
                schema.validate(record)?;
                if record.key != *key {
                    return Err(Error::Invalid("record key mismatch".into()));
                }
                let fields: BTreeMap<&str, Option<&str>> = record
                    .fields
                    .iter()
                    .map(|(name, cell)| {
                        let value = match cell {
                            Cell::Text(v) | Cell::Geometry(v) => Some(v.as_str()),
                            _ => None,
                        };
                        (name.as_str(), value)
                    })
                    .collect();
                inserts.push(fields);
            } else {
                deletes.push(*key);
            }
        }
        if !deletes.is_empty() {
            self.client.execute(&format!("DELETE FROM {table} WHERE {pk} IN (SELECT v::{} FROM unnest($1::text[]) AS v)",native(key_field(schema)?)?),&[&deletes]).map_err(pg_error)?;
        }
        if !inserts.is_empty() {
            let cols = schema
                .fields
                .iter()
                .map(|f| ident(&f.name))
                .collect::<Result<Vec<_>>>()?;
            let exprs = schema
                .fields
                .iter()
                .map(|f| {
                    let value = format!("v->>{}", literal(&f.name));
                    Ok(if f.geometry {
                        format!("ST_GeomFromEWKB(decode({value},'hex'))::{}", native(f)?)
                    } else {
                        format!("({value})::{}", native(f)?)
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let updates = cols
                .iter()
                .filter(|c| **c != pk)
                .map(|c| format!("{c}=excluded.{c}"))
                .collect::<Vec<_>>();
            let conflict = if updates.is_empty() {
                "DO NOTHING".into()
            } else {
                format!("DO UPDATE SET {}", updates.join(","))
            };
            let query = format!(
                "INSERT INTO {table} ({}) SELECT {} FROM jsonb_array_elements($1::text::jsonb) AS v ON CONFLICT ({pk}) {conflict}",
                cols.join(","),
                exprs.join(",")
            );
            self.client
                .execute(&query, &[&serde_json::to_string(&inserts)?])
                .map_err(pg_error)?;
        }
        Ok(())
    }
    fn normalize(&mut self, binding: &Binding, record: &Record) -> Result<Record> {
        self.normalize_many(binding, std::slice::from_ref(record))?
            .pop()
            .ok_or_else(|| Error::Database("normalization row count mismatch".into()))
    }
    fn write(&mut self, binding: &Binding, key: &str, value: Option<&Record>) -> Result<()> {
        self.write_many(binding, &[(key, value)])
    }
    fn clear_dirty(&mut self) -> Result<()> {
        self.client
            .execute(
                "DELETE FROM _geoledger.dirty WHERE repository_id=$1",
                &[&self.repository_id],
            )
            .map_err(pg_error)?;
        Ok(())
    }
    fn mark(&mut self, operation: &str, head: &ObjectId) -> Result<()> {
        self.client
            .execute(
                "UPDATE _geoledger.repositories SET operation=$2,head=$3 WHERE id=$1",
                &[&self.repository_id, &operation, &head.as_str()],
            )
            .map_err(pg_error)?;
        Ok(())
    }
    fn commit(&mut self) -> Result<()> {
        self.client.batch_execute("COMMIT").map_err(pg_error)?;
        self.active = false;
        Ok(())
    }
}
impl Drop for PostgisTransaction {
    fn drop(&mut self) {
        if self.active {
            let _ = self.client.batch_execute("ROLLBACK");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn statement_timeout_cannot_be_disabled_or_overflow_postgres() {
        for duration in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_micros(1_001),
            Duration::from_millis(i32::MAX as u64 + 1),
            Duration::MAX,
        ] {
            assert!(
                PostgisProvider::new("")
                    .with_statement_timeout(duration)
                    .is_err()
            );
        }
        for duration in [
            Duration::from_millis(1),
            Duration::from_millis(i32::MAX as u64),
        ] {
            assert!(
                PostgisProvider::new("")
                    .with_statement_timeout(duration)
                    .is_ok()
            );
        }
    }

    #[test]
    #[ignore = "requires disposable GL_TEST_DATABASE_URL"]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn statement_timeout_is_enforced_and_transaction_local() {
        let dsn = std::env::var("GL_TEST_DATABASE_URL").expect("set disposable test database URL");
        let mut client = Client::connect(&dsn, postgres::NoTls).unwrap();
        let database: String = client
            .query_one("SELECT current_database()", &[])
            .unwrap()
            .get(0);
        assert_eq!(database, "geoledger_test", "refusing a non-test database");
        let setting = |client: &mut Client| -> String {
            client
                .query_one("SHOW statement_timeout", &[])
                .unwrap()
                .get(0)
        };
        let original = setting(&mut client);
        PostgisProvider::new(&dsn)
            .start_transaction(&mut client)
            .unwrap();
        assert_eq!(setting(&mut client), "2min");
        client.batch_execute("ROLLBACK").unwrap();
        let provider = PostgisProvider::new(&dsn)
            .with_statement_timeout(Duration::from_secs(900))
            .unwrap();
        provider.start_transaction(&mut client).unwrap();
        assert_eq!(setting(&mut client), "15min");
        client.batch_execute("COMMIT").unwrap();
        assert_eq!(setting(&mut client), original);
        let provider = PostgisProvider::new(&dsn)
            .with_statement_timeout(Duration::from_millis(20))
            .unwrap();
        provider.start_transaction(&mut client).unwrap();
        let error = client.query_one("SELECT pg_sleep(0.2)", &[]).unwrap_err();
        assert_eq!(
            error.code(),
            Some(&postgres::error::SqlState::QUERY_CANCELED)
        );
        client.batch_execute("ROLLBACK").unwrap();
        assert_eq!(setting(&mut client), original);
    }
    #[test]
    fn identifiers_are_quoted_not_interpolated() {
        assert_eq!(
            ident("roads\"; DROP TABLE x;--").ok(),
            Some("\"roads\"\"; DROP TABLE x;--\"".into())
        );
        assert!(ident("a\0b").is_err());
    }
}
