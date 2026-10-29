//! Business-table access stays inside the publication transaction.
use super::{Backend, SqlTransaction};
use crate::{Error, PostgisTable, Result, Row};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct Source {
    pub(super) table: PostgisTable,
    #[serde(skip)]
    oid: i64,
    pub(super) signature: String,
    pub family: String,
    pub dimension: i32,
    pub srid: i32,
    pub crs: String,
    columns: Vec<String>,
    pub(super) order_index: String,
    validate_id: bool,
}
pub(super) fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
pub(super) fn relation(source: &PostgisTable) -> String {
    format!("{}.{}", quote(&source.schema), quote(&source.table))
}
pub(super) fn source_properties(id: &str, geometry: &str) -> String {
    // Exclude geometry before JSON conversion to avoid serializing it only to discard it.
    format!(
        "(to_jsonb(jsonb_populate_record(t.*,jsonb_build_object({id}::text,NULL,{geometry}::text,NULL)))-{id}::text-{geometry}::text)"
    )
}
fn drift() -> Error {
    Error::new(
        409,
        "business table changed outside GeoLedger; reconcile the source before publishing",
    )
}
impl SqlTransaction {
    pub(super) fn rebind_restored_sources(&mut self) -> Result<()> {
        let rows = self.query("SELECT project,id,postgis_source FROM gl_datasets WHERE postgis_source IS NOT NULL ORDER BY project,id", &[])?;
        for row in rows {
            let project: String = row.get(0usize)?;
            let dataset: String = row.get(1usize)?;
            let source: Source = crate::codec::stored(&row.get::<_, String>(2usize)?)?;
            let actual = self.inspect_source(&source.table)?;
            if actual.signature != source.signature || !actual.order_index.is_empty() {
                return Err(drift());
            }
            let (stamp, definition) = self.source_stamp(&source.table)?;
            let state = self.query_one("SELECT definition,binding,dirty_all,EXISTS(SELECT 1 FROM gl_source_changes c WHERE c.schema_name=$1 AND c.table_name=$2) FROM gl_source_state WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table])?;
            let binding: String = state.get(1usize)?;
            if definition != state.get::<_, String>(0usize)?
                || state.get::<_, bool>(2usize)?
                || state.get::<_, bool>(3usize)?
                || !self.source_binding_valid(&source.table, &binding)?
            {
                return Err(drift());
            }
            self.execute(
                "UPDATE gl_source_state SET stamp=$3 WHERE schema_name=$1 AND table_name=$2",
                &[&source.table.schema, &source.table.table, &stamp],
            )?;
            self.1.verified_sources.remove(&(project, dataset));
        }
        Ok(())
    }

    pub(super) fn inspect_source(&mut self, source: &PostgisTable) -> Result<Source> {
        self.inspect_source_with_lock(source, true)
    }
    fn inspect_source_with_lock(&mut self, source: &PostgisTable, lock: bool) -> Result<Source> {
        if !matches!(self.0.backend, Backend::Postgis(_)) {
            return Err(Error::new(
                400,
                "business tables require the PostGIS backend",
            ));
        }
        for name in [
            &source.schema,
            &source.table,
            &source.id_column,
            &source.geometry_column,
        ] {
            crate::text(name, 63)?;
        }
        if source.schema.starts_with("pg_")
            || source.schema == "information_schema"
            || source.table.starts_with("gl_")
            || source.id_column == source.geometry_column
        {
            return Err(Error::new(
                400,
                "select a business table with distinct identity and geometry columns",
            ));
        }
        let table = relation(source);
        // Locks also prevent external DDL and writes until commit, including initial enrollment.
        if lock {
            self.batch_execute(&format!("LOCK TABLE {table} IN SHARE ROW EXCLUSIVE MODE"))?;
        }
        let row = self.query_one("SELECT c.oid::bigint,c.relkind::text,c.relrowsecurity,c.relhasrules,c.relpersistence::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2", &[&source.schema, &source.table])?;
        let oid: i64 = row.get(0usize)?;
        if row.get::<_, String>(1usize)? != "r"
            || row.get::<_, bool>(2usize)?
            || row.get::<_, bool>(3usize)?
            || row.get::<_, String>(4usize)? != "p"
        {
            return Err(Error::new(
                400,
                "select a durable ordinary table with full-row visibility and no rewrite rules",
            ));
        }
        if self.query_one("SELECT EXISTS(SELECT 1 FROM pg_inherits WHERE inhrelid=$1::bigint::oid OR inhparent=$1::bigint::oid)", &[&oid])?.get::<_,bool>(0usize)? {
            return Err(Error::new(400, "select an independent table without inheritance"));
        }
        let columns = self.query("SELECT a.attname,t.typname,a.attgenerated::text,a.attidentity::text,a.atttypmod::bigint,a.attnum::bigint,a.attnotnull,CASE WHEN a.attcollation=0 THEN '' ELSE a.attcollation::regcollation::text END FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid WHERE a.attrelid=$1::bigint::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum", &[&oid])?;
        let mut properties = Vec::new();
        let mut signature = Vec::new();
        let mut key = None;
        let mut text_key = false;
        let mut geometry = None;
        for row in columns {
            let name: String = row.get(0usize)?;
            let kind: String = row.get(1usize)?;
            let generated: String = row.get(2usize)?;
            let identity: String = row.get(3usize)?;
            let typmod: i64 = row.get(4usize)?;
            let attnum: i64 = row.get(5usize)?;
            let notnull: bool = row.get(6usize)?;
            let collation: String = row.get(7usize)?;
            if !generated.is_empty() || (!identity.is_empty() && name != source.id_column) {
                return Err(Error::new(
                    400,
                    "business properties must be writable columns",
                ));
            }
            if name == source.id_column {
                if !["int2", "int4", "int8", "text", "varchar", "uuid"].contains(&kind.as_str()) {
                    return Err(Error::new(
                        400,
                        "identity must be an integer, text or UUID primary key",
                    ));
                }
                key = Some(attnum);
                text_key = kind == "text" || kind == "varchar";
            } else if name == source.geometry_column {
                if kind != "geometry" {
                    return Err(Error::new(400, "select a PostGIS geometry column"));
                }
                geometry = Some(typmod);
            } else {
                if kind == "geometry" || kind == "geography" {
                    return Err(Error::new(400, "select a table with one geometry column"));
                }
                crate::text(&name, 256)?;
                if matches!(
                    name.as_str(),
                    "$serde_json::private::RawValue" | "$serde_json::private::Number"
                ) {
                    return Err(Error::new(400, "reserved JSON object key"));
                }
                if properties.len() == 256 {
                    return Err(Error::new(
                        400,
                        "business tables support at most 256 properties",
                    ));
                }
                properties.push(name.clone());
            }
            signature.push((name, kind, generated, identity, typmod, notnull, collation));
        }
        let key = key.ok_or_else(|| Error::new(400, "identity column not found"))?;
        let primary: bool = self.query_one("SELECT EXISTS(SELECT 1 FROM pg_index WHERE indrelid=$1::bigint::oid AND indisprimary AND indisvalid AND indisready AND indislive AND indnkeyatts=1 AND indkey[0]=$2::bigint::smallint)", &[&oid,&key])?.get(0usize)?;
        if !primary {
            return Err(Error::new(
                400,
                "identity must be the table's single-column primary key",
            ));
        }
        let ordered: bool = self.query_one("SELECT EXISTS(SELECT 1 FROM pg_index x JOIN pg_class i ON i.oid=x.indexrelid JOIN pg_am a ON a.oid=i.relam JOIN pg_opclass o ON o.oid=x.indclass[0] WHERE x.indrelid=$1::bigint::oid AND x.indisvalid AND x.indisready AND x.indislive AND x.indnkeyatts>=1 AND x.indpred IS NULL AND a.amname='btree' AND o.opcname='text_ops' AND x.indcollation[0]='\"C\"'::regcollation AND (($3 AND x.indkey[0]=$2::bigint::smallint) OR (x.indkey[0]=0 AND pg_get_indexdef(x.indexrelid,1,true)=format('(%I::text)',$4::text))))", &[&oid,&key,&text_key,&source.id_column])?.get(0usize)?;
        let typmod = geometry.ok_or_else(|| Error::new(400, "geometry column not found"))?;
        let geometry = self.query_one("SELECT postgis_typmod_type($1::bigint::integer),postgis_typmod_srid($1::bigint::integer)::bigint,postgis_typmod_dims($1::bigint::integer)::bigint", &[&typmod])?;
        let kind: String = geometry.get(0usize)?;
        let family = match kind.trim_end_matches('Z') {
            "Point" | "MultiPoint" => "point",
            "LineString" | "MultiLineString" => "line",
            "Polygon" | "MultiPolygon" => "polygon",
            _ => {
                return Err(Error::new(
                    400,
                    "geometry column must declare a point, line or polygon type with XY or XYZ coordinates",
                ));
            }
        };
        let dimension: i32 = geometry.get(2usize)?;
        if ![2, 3].contains(&dimension) {
            return Err(Error::new(
                400,
                "geometry column must use XY or XYZ coordinates",
            ));
        }
        let srid = geometry.get::<_, i64>(1usize)? as i32;
        let definition = self.query_opt("SELECT auth_name,auth_srid::bigint,srtext FROM spatial_ref_sys WHERE srid=$1::bigint::integer", &[&srid])?;
        let crs = if let Some(row) = definition {
            let authority: Option<String> = row.get(0usize)?;
            let code: Option<i64> = row.get(1usize)?;
            match (authority.as_deref(), code) {
                (Some("EPSG"), Some(code)) => format!("EPSG:{code}"),
                _ => row
                    .get::<_, Option<String>>(2usize)?
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| format!("POSTGIS:{srid}")),
            }
        } else {
            format!("POSTGIS:{srid}")
        };
        Ok(Source {
            table: source.clone(),
            oid,
            signature: serde_json::to_string(&(&signature, &crs)).map_err(Error::stored_json)?,
            family: family.into(),
            dimension,
            srid,
            crs,
            columns: properties,
            order_index: if ordered {
                String::new()
            } else {
                format!("gl_order_{}", uuid::Uuid::new_v4().simple())
            },
            validate_id: text_key,
        })
    }
    pub(super) fn source_page(&mut self, source: &PostgisTable, after: &str) -> Result<Vec<Row>> {
        let id = quote(&source.id_column);
        let table = relation(source);
        // Preserve the source primary-key index, including its type and collation.
        let scope = if after.is_empty() {
            "$1::text=''".to_owned()
        } else {
            format!(
                "t.{id}>(jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,$1::text))).{id}"
            )
        };
        let properties = source_properties("$2", "$3");
        self.query(&format!("SELECT t.{id}::text,{properties}::text,ST_AsGeoJSON(t.{geom},17,0) FROM {table} t WHERE {scope} ORDER BY t.{id} LIMIT 1000", geom=quote(&source.geometry_column)), &[&after,&source.id_column,&source.geometry_column])
    }
    // Catalog tuple changes invalidate the incremental proof after trigger DDL or restore.
    pub(super) fn source_stamp(&mut self, source: &PostgisTable) -> Result<(String, String)> {
        let oid: i64 = self
            .query_one(
                "SELECT $1::text::regclass::oid::bigint",
                &[&relation(source)],
            )?
            .get(0usize)?;
        let rows = self.query("SELECT t.tgname,t.tgenabled::text,t.oid::text,t.xmin::text,p.xmin::text,pg_get_triggerdef(t.oid),pg_get_functiondef(p.oid),c.relfilenode::text FROM pg_trigger t JOIN pg_proc p ON p.oid=t.tgfoid JOIN pg_class c ON c.oid=t.tgrelid WHERE t.tgrelid=$1::bigint::oid AND t.tgname IN ('gl_versioned_write','gl_versioned_track','gl_versioned_truncate') ORDER BY t.tgname", &[&oid])?;
        if rows.len() != 3 {
            return Err(drift());
        }
        let mut stamp = vec![oid.to_string()];
        let mut definition = Vec::new();
        for row in rows {
            let name: String = row.get(0usize)?;
            let enabled: String = row.get(1usize)?;
            if enabled
                != if name == "gl_versioned_write" {
                    "O"
                } else {
                    "A"
                }
            {
                return Err(drift());
            }
            for n in [2usize, 3, 4, 7] {
                stamp.push(row.get::<_, String>(n)?);
            }
            for n in [5usize, 6] {
                definition.push(row.get::<_, String>(n)?);
            }
        }
        Ok((
            serde_json::to_string(&stamp).map_err(Error::stored_json)?,
            serde_json::to_string(&definition).map_err(Error::stored_json)?,
        ))
    }
    fn source_binding_valid(&mut self, source: &PostgisTable, binding: &str) -> Result<bool> {
        let count: i64 = self.query_one("SELECT count(*) FROM pg_trigger WHERE tgrelid=$1::text::regclass AND tgname IN ('gl_versioned_write','gl_versioned_track','gl_versioned_truncate') AND obj_description(oid,'pg_trigger')=$2", &[&relation(source),&binding])?.get(0usize)?;
        Ok(count == 3)
    }
    pub(super) fn ensure_source_clean(
        &mut self,
        project: &str,
        dataset: &str,
        source: &Source,
    ) -> Result<()> {
        let key = (project.to_owned(), dataset.to_owned());
        if self.1.verified_sources.contains(&key) {
            return Ok(());
        }
        self.batch_execute(&format!(
            "LOCK TABLE {} IN ACCESS SHARE MODE",
            relation(&source.table)
        ))?;
        let actual = self.inspect_source_with_lock(&source.table, false)?;
        if actual.signature != source.signature || !actual.order_index.is_empty() {
            return Err(drift());
        }
        let (stamp, definition) = self.source_stamp(&source.table)?;
        let state=self.query_one("SELECT stamp,definition,binding,dirty_all,EXISTS(SELECT 1 FROM gl_source_changes c WHERE c.schema_name=$1 AND c.table_name=$2) FROM gl_source_state WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table])?;
        if state.get::<_, bool>(3usize)?
            || state.get::<_, bool>(4usize)?
            || definition != state.get::<_, String>(1usize)?
            || stamp != state.get::<_, String>(0usize)?
            || !self.source_binding_valid(&source.table, &state.get::<_, String>(2usize)?)?
        {
            return Err(drift());
        }
        self.1.verified_sources.insert(key);
        Ok(())
    }
    pub(super) fn guard_source(&mut self, source: &Source) -> Result<()> {
        let srid = source.srid;
        let table_source = &source.table;
        let table = relation(table_source);
        let mut invalid = Vec::new();
        if source.validate_id {
            let id = quote(&table_source.id_column);
            // Match Rust's char::is_control; PostgreSQL text already rejects U+0000.
            invalid.push(format!(
                r#"btrim({id}::text)='' OR octet_length(convert_to({id}::text,'UTF8'))>256 OR {id}::text COLLATE "C" ~ U&'[\0001-\001F\007F-\009F]'"#
            ));
        }
        if srid == 0 {
            invalid.push(format!(
                "ST_SRID({})<>0",
                quote(&table_source.geometry_column)
            ));
        }
        if !invalid.is_empty()
            && self
                .query_one(
                    &format!(
                        "SELECT EXISTS(SELECT 1 FROM {table} WHERE {})",
                        invalid.join(" OR ")
                    ),
                    &[],
                )?
                .get::<_, bool>(0usize)?
        {
            return Err(Error::new(
                400,
                "business identifiers must be nonempty, contain no control characters and be at most 256 bytes; geometries must match the column SRID",
            ));
        }
        // Typmods allow empty single geometries and non-finite coordinates.
        self.batch_execute(&format!(
            "DECLARE gl_source_validation NO SCROLL CURSOR FOR SELECT ST_AsGeoJSON({},17,0) FROM {table}",
            quote(&table_source.geometry_column)
        ))?;
        loop {
            let rows = self.query("FETCH FORWARD 32 FROM gl_source_validation", &[])?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                if let Some(raw) = row.get::<_, Option<String>>(0usize)? {
                    let geometry = serde_json::from_str(&raw).map_err(|_| crate::bad())?;
                    crate::geometry::normalize(&geometry)?;
                }
            }
        }
        self.batch_execute("CLOSE gl_source_validation")?;
        let key = table_source.id_column.replace('\'', "''");
        if !source.order_index.is_empty() {
            self.batch_execute(&format!(
                "CREATE INDEX {} ON {table} (({}::text COLLATE \"C\"))",
                quote(&source.order_index),
                quote(&table_source.id_column)
            ))?;
        }
        self.batch_execute(&format!("CREATE TRIGGER gl_versioned_write BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON {table} FOR EACH STATEMENT EXECUTE FUNCTION public.gl_source_guard(); CREATE TRIGGER gl_versioned_track AFTER INSERT OR UPDATE OR DELETE ON {table} FOR EACH ROW EXECUTE FUNCTION public.gl_source_track('{key}'); CREATE TRIGGER gl_versioned_truncate AFTER TRUNCATE ON {table} FOR EACH STATEMENT EXECUTE FUNCTION public.gl_source_track(); ALTER TABLE {table} ENABLE ALWAYS TRIGGER gl_versioned_track; ALTER TABLE {table} ENABLE ALWAYS TRIGGER gl_versioned_truncate"))?;
        let binding = format!("geoledger:{}", uuid::Uuid::new_v4());
        for name in [
            "gl_versioned_write",
            "gl_versioned_track",
            "gl_versioned_truncate",
        ] {
            self.batch_execute(&format!(
                "COMMENT ON TRIGGER {name} ON {table} IS '{binding}'"
            ))?;
        }
        let (stamp, definition) = self.source_stamp(table_source)?;
        self.execute("INSERT INTO gl_source_state(schema_name,table_name,stamp,definition,binding) VALUES($1,$2,$3,$4,$5)", &[&table_source.schema,&table_source.table,&stamp,&definition,&binding])?;
        Ok(())
    }
    pub(super) fn release_sources(&mut self, project: &str, dataset: Option<&str>) -> Result<()> {
        if !matches!(self.0.backend, Backend::Postgis(_)) {
            return Ok(());
        }
        let rows=self.query("SELECT postgis_source FROM gl_datasets WHERE project=$1 AND ($2::text IS NULL OR id=$2) AND postgis_source IS NOT NULL", &[&project,&dataset])?;
        for row in rows {
            let source: Source = crate::codec::stored(&row.get::<_, String>(0usize)?)?;
            let state = self.query_one("SELECT binding,(stamp::jsonb->>0)::bigint FROM gl_source_state WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table])?;
            let binding: String = state.get(0usize)?;
            let source_oid: i64 = state.get(1usize)?;
            // The stamped relation survives renames; a dropped source already lost its triggers and indexes.
            if let Some(table_row) = self.query_opt("SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=$1::bigint::oid AND c.relkind='r'", &[&source_oid])? {
                let table = format!(
                    "{}.{}",
                    quote(&table_row.get::<_, String>(0usize)?),
                    quote(&table_row.get::<_, String>(1usize)?)
                );
                self.batch_execute(&format!("LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"))?;
                let locked_oid: i64 = self.query_one("SELECT $1::text::regclass::oid::bigint", &[&table])?.get(0usize)?;
                if locked_oid != source_oid {
                    return Err(drift());
                }
                let triggers = self.query("SELECT tgname,obj_description(oid,'pg_trigger') FROM pg_trigger WHERE tgrelid=$1::bigint::oid AND tgname IN ('gl_versioned_write','gl_versioned_track','gl_versioned_truncate') ORDER BY tgname", &[&source_oid])?;
                if triggers.len() != 3 {
                    return Err(drift());
                }
                for trigger in &triggers {
                    if trigger.get::<_,Option<String>>(1usize)?.as_deref() != Some(binding.as_str()) {
                        return Err(drift());
                    }
                }
                for trigger in triggers {
                    let name: String = trigger.get(0usize)?;
                    self.batch_execute(&format!("DROP TRIGGER {} ON {table}", quote(&name)))?;
                }
                if !source.order_index.is_empty()
                    && let Some(index) = self.query_opt("SELECT n.nspname FROM pg_index x JOIN pg_class i ON i.oid=x.indexrelid JOIN pg_namespace n ON n.oid=i.relnamespace WHERE x.indrelid=$1::bigint::oid AND i.relname=$2 AND i.relkind='i'", &[&source_oid,&source.order_index])? {
                    self.batch_execute(&format!("DROP INDEX {}.{}",quote(&index.get::<_,String>(0usize)?),quote(&source.order_index)))?;
                }
            }
            self.execute(
                "DELETE FROM gl_source_state WHERE schema_name=$1 AND table_name=$2",
                &[&source.table.schema, &source.table.table],
            )?;
        }
        Ok(())
    }
    pub(super) fn check_source(
        &mut self,
        dataset: &str,
        source: &Source,
        published: bool,
    ) -> Result<()> {
        let table = relation(&source.table);
        let id = quote(&source.table.id_column);
        let geom = quote(&source.table.geometry_column);
        let srid = source.srid;
        if !published && self.query_one("SELECT EXISTS(SELECT 1 FROM gl_source_changes WHERE schema_name=$1 AND table_name=$2)", &[&source.table.schema,&source.table.table])?.get::<_,bool>(0usize)? {
            return Err(drift());
        }
        let keys = if published {
            "SELECT feature_id FROM gl_merge_stage WHERE dataset=$1 UNION SELECT feature_id FROM gl_source_changes WHERE schema_name=$4 AND table_name=$5"
        } else {
            "SELECT feature_id FROM gl_merge_stage WHERE dataset=$1 AND CAST($4 AS text) IS NOT NULL AND CAST($5 AS text) IS NOT NULL"
        };
        let value = if published {
            "s.after_value"
        } else {
            "s.before_value"
        };
        let properties = source_properties("$2", "$3");
        let mismatch: bool = self.query_one(&format!("SELECT EXISTS(SELECT 1 FROM ({keys}) k LEFT JOIN gl_merge_stage s ON s.dataset=$1 AND s.feature_id=k.feature_id LEFT JOIN LATERAL (SELECT t.{id}::text AS feature_id,{properties} AS properties,t.{geom} AS geom FROM {table} t WHERE t.{id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,k.feature_id))).{id} LIMIT 1) t ON true WHERE s.feature_id IS NULL OR t.properties IS DISTINCT FROM gl_json_field({value},'properties')::jsonb OR ST_SRID(t.geom)<>{srid} OR ST_AsEWKB(ST_SetSRID(t.geom,0)) IS DISTINCT FROM ST_AsEWKB(ST_SetSRID(ST_GeomFromGeoJSON(gl_json_field({value},'geometry')),0)) OR (t.feature_id IS NOT NULL AND (btrim(t.feature_id)='' OR octet_length(t.feature_id)>256)))"), &[&dataset,&source.table.id_column,&source.table.geometry_column,&source.table.schema,&source.table.table])?.get(0usize)?;
        if mismatch {
            return Err(drift());
        }
        Ok(())
    }
    pub(super) fn sync_sources(
        &mut self,
        project: &str,
        schema_prepared: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        if !matches!(self.0.backend, Backend::Postgis(_)) {
            return Ok(());
        }
        let sources = self.query("SELECT id,postgis_source FROM gl_datasets WHERE project=$1 AND postgis_source IS NOT NULL AND id IN (SELECT dataset FROM gl_merge_stage) ORDER BY postgis_source", &[&project])?;
        let mut publication_tables = Vec::new();
        for row in &sources {
            let dataset: String = row.get(0usize)?;
            let source: Source = crate::codec::stored(&row.get::<_, String>(1usize)?)?;
            let actual = self.inspect_source(&source.table)?;
            if source.signature != actual.signature || !actual.order_index.is_empty() {
                return Err(drift());
            }
            let (stamp, definition) = self.source_stamp(&source.table)?;
            let state = self.query_one("SELECT stamp,definition,dirty_all,binding FROM gl_source_state WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table])?;
            if definition != state.get::<_, String>(1usize)? {
                return Err(drift());
            }
            if state.get::<_, bool>(2usize)?
                || stamp != state.get::<_, String>(0usize)?
                || !self.source_binding_valid(&source.table, &state.get::<_, String>(3usize)?)?
            {
                return Err(drift());
            }
            if !schema_prepared.contains(&dataset) {
                self.check_source(&dataset, &source, false)?;
            }
            self.execute(
                "UPDATE gl_source_state SET stamp=$3 WHERE schema_name=$1 AND table_name=$2",
                &[&source.table.schema, &source.table.table, &stamp],
            )?;
            publication_tables.push(actual.oid.to_string());
        }
        for (row, oid) in sources.iter().zip(&publication_tables) {
            let dataset: String = row.get(0usize)?;
            let source: Source = crate::codec::stored(&row.get::<_, String>(1usize)?)?;
            let table = relation(&source.table);
            let id = quote(&source.table.id_column);
            let geom = quote(&source.table.geometry_column);
            if self.query_one(&format!("SELECT EXISTS(SELECT 1 FROM gl_merge_stage WHERE dataset=$1 AND (jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,feature_id))).{id}::text IS DISTINCT FROM feature_id)"), &[&dataset,&source.table.id_column])?.get::<_,bool>(0usize)? {
                return Err(Error::new(400,"feature identifiers must use the business primary key's canonical text"));
            }
            self.query_one(
                "SELECT set_config('geoledger.publication_table',$1,true)",
                &[oid],
            )?;
            // Keep row ordering for business triggers; ordinary tables can write the staged batch at once.
            let batch: bool = self.query_one("SELECT NOT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid=$1::text::oid AND NOT tgisinternal AND tgname NOT IN ('gl_versioned_write','gl_versioned_track','gl_versioned_truncate'))", &[oid])?.get(0usize)?;
            let mut after = String::new();
            loop {
                let rows = self.query("SELECT feature_id,after_value FROM gl_merge_stage WHERE dataset=$1 AND feature_id COLLATE \"C\">$2 ORDER BY feature_id COLLATE \"C\" LIMIT 1000", &[&dataset,&after])?;
                if rows.is_empty() {
                    break;
                }
                for row in rows {
                    let key: String = row.get(0usize)?;
                    let value: Option<String> = row.get(1usize)?;
                    if let Some(value) = value {
                        let stored: crate::feature::Stored = crate::codec::stored(&value)?;
                        if stored.properties.len() != source.columns.len()
                            || source
                                .columns
                                .iter()
                                .any(|c| !stored.properties.contains_key(c))
                        {
                            return Err(Error::new(
                                400,
                                "feature properties must match the business table columns",
                            ));
                        }
                        if batch {
                            after = key;
                            continue;
                        }
                        let props = serde_json::to_string(&stored.properties)
                            .map_err(Error::stored_json)?;
                        let names = source.columns.iter().map(|c| quote(c)).collect::<Vec<_>>();
                        let mut columns = vec![id.clone(), geom.clone()];
                        columns.extend(names.clone());
                        let mut values = vec![
                            format!("r.{id}"),
                            format!("ST_SetSRID(ST_GeomFromGeoJSON($3::text),{})", source.srid),
                        ];
                        values.extend(names.iter().map(|c| format!("r.{c}")));
                        let mut assignments = vec![format!(
                            "{geom}=ST_SetSRID(ST_GeomFromGeoJSON($3::text),{})",
                            source.srid
                        )];
                        assignments.extend(names.iter().map(|c| format!("{c}=r.{c}")));
                        // The source lock makes update-then-insert atomic, including deferred primary keys.
                        let updated = self.query_opt(&format!("UPDATE {table} t SET {} FROM jsonb_populate_record(NULL::{table},$2::text::jsonb || jsonb_build_object($4::text,$1::text)) r WHERE t.{id}=r.{id} RETURNING 1",assignments.join(",")), &[&key,&props,&stored.geometry,&source.table.id_column])?;
                        if updated.is_none() {
                            self.execute(&format!("INSERT INTO {table} ({}) OVERRIDING SYSTEM VALUE SELECT {} FROM jsonb_populate_record(NULL::{table},$2::text::jsonb || jsonb_build_object($4::text,$1::text)) r",columns.join(","),values.join(",")), &[&key,&props,&stored.geometry,&source.table.id_column])?;
                        }
                        let properties_sql = source_properties("$2", "$3");
                        let check = self.query_opt(&format!("SELECT {properties_sql}::text,ST_AsGeoJSON(t.{geom},17,0),t.{id}::text FROM {table} t WHERE t.{id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,$1::text))).{id}"), &[&key,&source.table.id_column,&source.table.geometry_column])?.ok_or_else(drift)?;
                        if check.get::<_, String>(2usize)? != key {
                            return Err(Error::new(
                                400,
                                "feature identifiers must exactly match the business primary key",
                            ));
                        }
                        let properties: serde_json::Value =
                            crate::codec::stored(&check.get::<_, String>(0usize)?)?;
                        let geometry: Option<String> = check.get(1usize)?;
                        let actual_geometry = geometry
                            .map(|g| crate::geometry::normalize(&crate::codec::stored(&g)?))
                            .transpose()?
                            .flatten();
                        if properties
                            != serde_json::to_value(&stored.properties)
                                .map_err(Error::stored_json)?
                            || actual_geometry != stored.geometry
                        {
                            return Err(Error::new(
                                409,
                                "business table constraints or triggers changed the published feature",
                            ));
                        }
                    } else {
                        if batch {
                            after = key;
                            continue;
                        }
                        self.execute(&format!("DELETE FROM {table} WHERE {id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,$1::text))).{id}"), &[&key,&source.table.id_column])?;
                        if self
                            .query_opt(
                                &format!("SELECT 1 FROM {table} WHERE {id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,$1::text))).{id}"),
                                &[&key,&source.table.id_column],
                            )?
                            .is_some()
                        {
                            return Err(drift());
                        }
                    }
                    after = key;
                }
            }
            if batch {
                let names = source.columns.iter().map(|c| quote(c)).collect::<Vec<_>>();
                let mut columns = vec![id.clone(), geom.clone()];
                columns.extend(names.clone());
                let geometry = format!(
                    "ST_SetSRID(ST_GeomFromGeoJSON(gl_json_field(s.after_value,'geometry')),{})",
                    source.srid
                );
                let mut values = vec![format!("r.{id}"), geometry.to_owned()];
                values.extend(names.iter().map(|c| format!("r.{c}")));
                let mut assignments = vec![format!("{geom}={geometry}")];
                assignments.extend(names.iter().map(|c| format!("{c}=r.{c}")));
                let records = format!(
                    "gl_merge_stage s CROSS JOIN LATERAL jsonb_populate_record(NULL::{table},gl_json_field(s.after_value,'properties')::jsonb || jsonb_build_object($2::text,s.feature_id)) r"
                );
                self.execute(&format!("UPDATE {table} t SET {} FROM {records} WHERE s.dataset=$1 AND s.after_value IS NOT NULL AND t.{id}=r.{id}",assignments.join(",")), &[&dataset,&source.table.id_column])?;
                self.execute(&format!("INSERT INTO {table} ({}) OVERRIDING SYSTEM VALUE SELECT {} FROM {records} WHERE s.dataset=$1 AND s.after_value IS NOT NULL AND NOT EXISTS(SELECT 1 FROM {table} t WHERE t.{id}=r.{id}) ORDER BY s.feature_id COLLATE \"C\"",columns.join(","),values.join(",")), &[&dataset,&source.table.id_column])?;
                self.execute(&format!("DELETE FROM {table} t USING gl_merge_stage s WHERE s.dataset=$1 AND s.after_value IS NULL AND t.{id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,s.feature_id))).{id}"), &[&dataset,&source.table.id_column])?;
            }
        }
        // Resolve cross-table constraints only after every source has its final requested rows.
        self.query_one(
            "SELECT set_config('geoledger.publication_table',$1,true)",
            &[&publication_tables.join(",")],
        )?;
        self.batch_execute("SET CONSTRAINTS ALL IMMEDIATE")?;
        // Validate the final table image too: a business trigger may affect another row.
        for row in &sources {
            let dataset: String = row.get(0usize)?;
            let source: Source = crate::codec::stored(&row.get::<_, String>(1usize)?)?;
            let (stamp, definition) = self.source_stamp(&source.table)?;
            let state = self.query_one("SELECT stamp,definition,dirty_all FROM gl_source_state WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table])?;
            if definition != state.get::<_, String>(1usize)? {
                return Err(drift());
            }
            if stamp != state.get::<_, String>(0usize)? || state.get::<_, bool>(2usize)? {
                return Err(drift());
            }
            self.check_source(&dataset, &source, true)?;
            self.execute(
                "DELETE FROM gl_source_changes WHERE schema_name=$1 AND table_name=$2",
                &[&source.table.schema, &source.table.table],
            )?;
            self.execute("UPDATE gl_source_state SET stamp=$3,dirty_all=false WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table,&stamp])?;
        }
        self.batch_execute("SET LOCAL geoledger.publication_table = ''")?;
        Ok(())
    }
}

use super::Cell;
use crate::repository::PostgisTables;

impl PostgisTables for SqlTransaction {
    fn inspect_postgis_table(&mut self, source: &crate::PostgisTable) -> Result<Row> {
        let metadata = self.inspect_source(source)?;
        Ok(Row::new(vec![
            Cell::Text(metadata.family),
            Cell::Integer(metadata.dimension.into()),
            Cell::Text(metadata.crs),
        ]))
    }

    fn postgis_table_page(
        &mut self,
        source: &crate::PostgisTable,
        after: &str,
    ) -> Result<Vec<Row>> {
        self.source_page(source, after)
    }

    fn bind_postgis_table(
        &mut self,
        project: &str,
        dataset: &str,
        source: &crate::PostgisTable,
        revision: i64,
    ) -> Result<()> {
        let source_metadata = self.inspect_source(source)?;
        let metadata = serde_json::to_string(&source_metadata).map_err(Error::stored_json)?;
        self.execute(
            "UPDATE gl_datasets SET postgis_source=$3,source_revision=$4 WHERE project=$1 AND id=$2",
            &[&project, &dataset, &metadata, &revision],
        )?;
        self.1.datasets.clear();
        self.guard_source(&source_metadata)
    }
}
