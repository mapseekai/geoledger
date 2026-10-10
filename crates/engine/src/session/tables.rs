//! Business-table access stays inside the publication transaction.
use super::{Backend, SqlTransaction};
use crate::{Error, PostgisTable, Result, Row};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct Source {
    table: PostgisTable,
    #[serde(skip)]
    oid: i64,
    signature: String,
    pub family: String,
    pub dimension: i32,
    columns: Vec<String>,
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn relation(source: &PostgisTable) -> String {
    format!("{}.{}", quote(&source.schema), quote(&source.table))
}
fn drift() -> Error {
    Error::new(
        409,
        "business table changed outside GeoLedger; reconcile the source before publishing",
    )
}
impl SqlTransaction {
    pub(super) fn inspect_source(&mut self, source: &PostgisTable) -> Result<Source> {
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
        self.batch_execute(&format!("LOCK TABLE {table} IN SHARE ROW EXCLUSIVE MODE"))?;
        let row = self.query_one("SELECT c.oid::bigint,c.relkind::text,c.relrowsecurity,c.relhasrules FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2", &[&source.schema, &source.table])?;
        let oid: i64 = row.get(0usize)?;
        if row.get::<_, String>(1usize)? != "r"
            || row.get::<_, bool>(2usize)?
            || row.get::<_, bool>(3usize)?
        {
            return Err(Error::new(
                400,
                "select an ordinary table with full-row visibility and no rewrite rules",
            ));
        }
        if self.query_one("SELECT EXISTS(SELECT 1 FROM pg_inherits WHERE inhrelid=$1::bigint::oid OR inhparent=$1::bigint::oid)", &[&oid])?.get::<_,bool>(0usize)? {
            return Err(Error::new(400, "select an independent table without inheritance"));
        }
        let columns = self.query("SELECT a.attname,t.typname,a.attgenerated::text,a.attidentity::text,a.atttypmod::bigint,a.attnum::bigint,a.attnotnull FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid WHERE a.attrelid=$1::bigint::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum", &[&oid])?;
        let mut properties = Vec::new();
        let mut signature = Vec::new();
        let mut key = None;
        let mut geometry = None;
        for row in columns {
            let name: String = row.get(0usize)?;
            let kind: String = row.get(1usize)?;
            let generated: String = row.get(2usize)?;
            let identity: String = row.get(3usize)?;
            let typmod: i64 = row.get(4usize)?;
            let attnum: i64 = row.get(5usize)?;
            let notnull: bool = row.get(6usize)?;
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
            } else if name == source.geometry_column {
                if kind != "geometry" {
                    return Err(Error::new(400, "select a PostGIS geometry column"));
                }
                geometry = Some(typmod);
            } else {
                if kind == "geometry" || kind == "geography" {
                    return Err(Error::new(400, "select a table with one geometry column"));
                }
                properties.push(name.clone());
            }
            signature.push((name, kind, generated, identity, typmod, notnull));
        }
        let key = key.ok_or_else(|| Error::new(400, "identity column not found"))?;
        let primary: bool = self.query_one("SELECT EXISTS(SELECT 1 FROM pg_index WHERE indrelid=$1::bigint::oid AND indisprimary AND indnkeyatts=1 AND indkey[0]=$2::bigint::smallint)", &[&oid,&key])?.get(0usize)?;
        if !primary {
            return Err(Error::new(
                400,
                "identity must be the table's single-column primary key",
            ));
        }
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
        if geometry.get::<_, i64>(1usize)? != 4326 || ![2, 3].contains(&dimension) {
            return Err(Error::new(
                400,
                "geometry column must use SRID 4326 and XY or XYZ coordinates",
            ));
        }
        Ok(Source {
            table: source.clone(),
            oid,
            signature: serde_json::to_string(&signature).map_err(Error::stored_json)?,
            family: family.into(),
            dimension,
            columns: properties,
        })
    }
    pub(super) fn source_page(&mut self, source: &PostgisTable, after: &str) -> Result<Vec<Row>> {
        self.query(&format!("SELECT t.{id}::text,(to_jsonb(t)-$2::text-$3::text)::text,ST_AsGeoJSON(t.{geom},17,0) FROM {table} t WHERE t.{id}::text COLLATE \"C\">$1 ORDER BY t.{id}::text COLLATE \"C\" LIMIT 1000", id=quote(&source.id_column),geom=quote(&source.geometry_column),table=relation(source)), &[&after,&source.id_column,&source.geometry_column])
    }
    // Catalog tuple changes invalidate the incremental proof after trigger DDL or restore.
    fn source_stamp(&mut self, source: &PostgisTable) -> Result<(String, String)> {
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
    pub(super) fn guard_source(&mut self, source: &PostgisTable) -> Result<()> {
        let table = relation(source);
        if self.query_one(&format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE btrim({id}::text)='' OR octet_length({id}::text)>256)",id=quote(&source.id_column)), &[])?.get::<_,bool>(0usize)? {
            return Err(Error::new(400,"business identifiers must be nonempty and at most 256 bytes"));
        }
        let key = source.id_column.replace('\'', "''");
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
        let (stamp, definition) = self.source_stamp(source)?;
        self.execute("INSERT INTO gl_source_state(schema_name,table_name,stamp,definition,binding) VALUES($1,$2,$3,$4,$5)", &[&source.schema,&source.table,&stamp,&definition,&binding])?;
        Ok(())
    }
    pub(super) fn release_sources(&mut self, project: &str, dataset: Option<&str>) -> Result<()> {
        if !matches!(self.0.backend, Backend::Postgis(_)) {
            return Ok(());
        }
        let rows=self.query("SELECT postgis_source FROM gl_datasets WHERE project=$1 AND ($2::text IS NULL OR id=$2) AND postgis_source IS NOT NULL", &[&project,&dataset])?;
        for row in rows {
            let source: Source = crate::codec::stored(&row.get::<_, String>(0usize)?)?;
            // Trigger comments survive logical restore and table/schema renames.
            let binding: String = self
                .query_one(
                    "SELECT binding FROM gl_source_state WHERE schema_name=$1 AND table_name=$2",
                    &[&source.table.schema, &source.table.table],
                )?
                .get(0usize)?;
            let triggers = self.query("SELECT n.nspname,c.relname,t.tgname,t.oid::bigint FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE obj_description(t.oid,'pg_trigger')=$1 AND t.tgname IN ('gl_versioned_write','gl_versioned_track','gl_versioned_truncate') ORDER BY c.oid,t.tgname", &[&binding])?;
            for trigger in triggers {
                let table = format!(
                    "{}.{}",
                    quote(&trigger.get::<_, String>(0usize)?),
                    quote(&trigger.get::<_, String>(1usize)?)
                );
                let name: String = trigger.get(2usize)?;
                let oid: i64 = trigger.get(3usize)?;
                self.batch_execute(&format!("LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"))?;
                if self.query_opt("SELECT 1 FROM pg_trigger WHERE oid=$1::bigint::oid AND obj_description(oid,'pg_trigger')=$2 AND tgrelid=$3::text::regclass AND tgname=$4", &[&oid,&binding,&table,&name])?.is_some() {
                    self.batch_execute(&format!("DROP TRIGGER {} ON {table}", quote(&name)))?;
                }
            }
            self.execute(
                "DELETE FROM gl_source_state WHERE schema_name=$1 AND table_name=$2",
                &[&source.table.schema, &source.table.table],
            )?;
        }
        Ok(())
    }
    fn check_source(
        &mut self,
        project: &str,
        dataset: &str,
        source: &Source,
        full: bool,
        published: bool,
    ) -> Result<()> {
        let table = relation(&source.table);
        let id = quote(&source.table.id_column);
        let geom = quote(&source.table.geometry_column);
        let mismatch = if full {
            let expected = if published {
                "SELECT feature_id,properties,geom FROM gl_history WHERE project=$1 AND dataset=$2 AND valid_to IS NULL AND properties IS NOT NULL AND feature_id NOT IN (SELECT feature_id FROM gl_merge_stage WHERE dataset=$2) UNION ALL SELECT feature_id,gl_json_field(after_value,'properties'),ST_SetSRID(ST_GeomFromGeoJSON(gl_json_field(after_value,'geometry')),4326) FROM gl_merge_stage WHERE dataset=$2 AND after_value IS NOT NULL"
            } else {
                "SELECT feature_id,properties,geom FROM gl_history WHERE project=$1 AND dataset=$2 AND valid_to IS NULL AND properties IS NOT NULL"
            };
            self.query_one(&format!("WITH expected AS ({expected}) SELECT EXISTS(SELECT 1 FROM {table} t FULL JOIN expected h ON t.{id}::text=h.feature_id WHERE t.{id} IS NULL OR h.feature_id IS NULL OR (to_jsonb(t)-$3::text-$4::text) IS DISTINCT FROM h.properties::jsonb OR ST_AsEWKB(t.{geom}) IS DISTINCT FROM ST_AsEWKB(h.geom))"), &[&project,&dataset,&source.table.id_column,&source.table.geometry_column])?.get::<_,bool>(0usize)?
        } else {
            // LIMIT keeps the candidate set driving native primary-key lookups, even before ANALYZE.
            self.query_one(&format!("SELECT EXISTS(SELECT 1 FROM gl_source_changes k LEFT JOIN LATERAL (SELECT t.{id}::text AS feature_id,(to_jsonb(t)-$3::text-$4::text) AS properties,t.{geom} AS geom FROM {table} t WHERE t.{id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($3::text,k.feature_id))).{id} LIMIT 1) t ON true LEFT JOIN LATERAL (SELECT properties,geom FROM gl_history WHERE project=$1 AND dataset=$2 AND feature_id=k.feature_id AND valid_to IS NULL LIMIT 1) h ON true LEFT JOIN gl_merge_stage s ON s.dataset=$2 AND s.feature_id=k.feature_id AND $7::boolean WHERE k.schema_name=$5 AND k.table_name=$6 AND (t.properties IS DISTINCT FROM (CASE WHEN s.feature_id IS NOT NULL THEN gl_json_field(s.after_value,'properties') ELSE h.properties END)::jsonb OR ST_AsEWKB(t.geom) IS DISTINCT FROM ST_AsEWKB(CASE WHEN s.feature_id IS NOT NULL THEN ST_SetSRID(ST_GeomFromGeoJSON(gl_json_field(s.after_value,'geometry')),4326) ELSE h.geom END) OR (t.feature_id IS NOT NULL AND (btrim(t.feature_id)='' OR octet_length(t.feature_id)>256))))"), &[&project,&dataset,&source.table.id_column,&source.table.geometry_column,&source.table.schema,&source.table.table,&published])?.get::<_,bool>(0usize)?
        };
        if mismatch {
            return Err(drift());
        }
        Ok(())
    }
    pub(super) fn sync_sources(&mut self, project: &str) -> Result<()> {
        if !matches!(self.0.backend, Backend::Postgis(_)) {
            return Ok(());
        }
        let sources = self.query("SELECT id,postgis_source FROM gl_datasets WHERE project=$1 AND postgis_source IS NOT NULL AND id IN (SELECT dataset FROM gl_merge_stage) ORDER BY postgis_source", &[&project])?;
        let mut publication_tables = Vec::new();
        for row in &sources {
            let dataset: String = row.get(0usize)?;
            let source: Source = crate::codec::stored(&row.get::<_, String>(1usize)?)?;
            let actual = self.inspect_source(&source.table)?;
            if source.signature != actual.signature {
                return Err(drift());
            }
            let (stamp, definition) = self.source_stamp(&source.table)?;
            let state = self.query_one("SELECT stamp,definition,dirty_all FROM gl_source_state WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table])?;
            if definition != state.get::<_, String>(1usize)? {
                return Err(drift());
            }
            let full = stamp != state.get::<_, String>(0usize)? || state.get::<_, bool>(2usize)?;
            self.check_source(project, &dataset, &source, full, false)?;
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
                        let props = serde_json::to_string(&stored.properties)
                            .map_err(Error::stored_json)?;
                        let names = source.columns.iter().map(|c| quote(c)).collect::<Vec<_>>();
                        let mut columns = vec![id.clone(), geom.clone()];
                        columns.extend(names.clone());
                        let mut values = vec![
                            format!("r.{id}"),
                            "ST_SetSRID(ST_GeomFromGeoJSON($3::text),4326)".into(),
                        ];
                        values.extend(names.iter().map(|c| format!("r.{c}")));
                        let mut assignments = vec![format!(
                            "{geom}=ST_SetSRID(ST_GeomFromGeoJSON($3::text),4326)"
                        )];
                        assignments.extend(names.iter().map(|c| format!("{c}=r.{c}")));
                        // The source lock makes update-then-insert atomic, including deferred primary keys.
                        let updated = self.query_opt(&format!("UPDATE {table} t SET {} FROM jsonb_populate_record(NULL::{table},$2::text::jsonb || jsonb_build_object($4::text,$1::text)) r WHERE t.{id}=r.{id} RETURNING 1",assignments.join(",")), &[&key,&props,&stored.geometry,&source.table.id_column])?;
                        if updated.is_none() {
                            self.execute(&format!("INSERT INTO {table} ({}) OVERRIDING SYSTEM VALUE SELECT {} FROM jsonb_populate_record(NULL::{table},$2::text::jsonb || jsonb_build_object($4::text,$1::text)) r",columns.join(","),values.join(",")), &[&key,&props,&stored.geometry,&source.table.id_column])?;
                        }
                        let check = self.query_opt(&format!("SELECT (to_jsonb(t)-$2::text-$3::text)::text,ST_AsGeoJSON(t.{geom},17,0),t.{id}::text FROM {table} t WHERE t.{id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($2::text,$1::text))).{id}"), &[&key,&source.table.id_column,&source.table.geometry_column])?.ok_or_else(drift)?;
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
            let full = stamp != state.get::<_, String>(0usize)? || state.get::<_, bool>(2usize)?;
            self.check_source(project, &dataset, &source, full, true)?;
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
