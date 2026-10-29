use super::*;

impl DatasetRepository for SqlTransaction {
    fn schema_projection(
        &mut self,
        project: &str,
        dataset: &str,
        revision: i64,
    ) -> Result<crate::DatasetSchema> {
        match self.query_opt("SELECT definition FROM gl_schema_history WHERE project=$1 AND dataset=$2 AND revision<=$3 ORDER BY revision DESC LIMIT 1", &[&project,&dataset,&revision])? {
            Some(row)=>crate::codec::stored(&row.get::<_,String>(0usize)?),None=>Ok(crate::DatasetSchema{dynamic:true,..Default::default()})
        }
    }
    fn schema_at(
        &mut self,
        project: &str,
        dataset: &str,
        revision: i64,
    ) -> Result<crate::DatasetSchema> {
        self.read_schema_at(project, dataset, revision)
    }
    fn workspace_schemas(
        &mut self,
        project: &str,
        workspace: &str,
    ) -> Result<Vec<(String, crate::DatasetSchema)>> {
        self.read_workspace_schemas(project, workspace)
    }
    fn replace_workspace_schemas(
        &mut self,
        project: &str,
        workspace: &str,
        definitions: &[(String, crate::DatasetSchema, crate::DatasetSchema)],
    ) -> Result<()> {
        self.replace_schema_intent(project, workspace, definitions)
    }
    fn published_schema_changes(
        &mut self,
        project: &str,
        revision: i64,
    ) -> Result<Vec<(String, crate::DatasetSchema, crate::DatasetSchema)>> {
        let rows=self.query("SELECT dataset,definition FROM gl_schema_history WHERE project=$1 AND revision=$2 ORDER BY dataset", &[&project,&revision])?;
        let mut result = Vec::new();
        for row in rows {
            let dataset: String = row.get(0usize)?;
            result.push((
                dataset.clone(),
                self.read_schema_at(project, &dataset, revision - 1)?,
                crate::codec::stored(&row.get::<_, String>(1usize)?)?,
            ));
        }
        Ok(result)
    }
    fn schema_changed_since(&mut self, project: &str, base: i64, current: i64) -> Result<bool> {
        self.query_one("SELECT EXISTS(SELECT 1 FROM gl_schema_history WHERE project=$1 AND revision>$2 AND revision<=$3)", &[&project,&base,&current])?.get(0usize)
    }
    fn clear_workspace_schemas(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute(
            "DELETE FROM gl_workspace_schemas WHERE project=$1 AND workspace=$2",
            &[&project, &workspace],
        )
    }

    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>> {
        let key = (project.to_owned(), dataset.to_owned());
        if let Some(row) = self.1.datasets.get(&key) {
            return Ok(Some(row.clone()));
        }
        let row = self.query_opt(
            &self.lock_sql("SELECT geometry_type,name,coordinate_dimension,postgis_source,crs,storage_table,source_revision FROM gl_datasets WHERE project=$1 AND id=$2", "FOR KEY SHARE"),
            &[&project, &dataset],
        )?;
        if let Some(row) = &row {
            if self.1.datasets.len() >= 128 {
                self.1.datasets.clear();
            }
            self.1.datasets.insert(key, row.clone());
        }
        Ok(row)
    }

    fn insert_dataset(
        &mut self,
        project: &str,
        dataset: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: i32,
        crs: &str,
        managed: bool,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO gl_datasets(project,id,name,geometry_type,coordinate_dimension,crs) VALUES($1,$2,$3,$4,$5,$6)",
            &[
                &project,
                &dataset,
                &name,
                &geometry_type,
                &coordinate_dimension,
                &crs,
            ],
        )?;
        if managed {
            self.create_dataset_table(project, dataset)?;
        }
        Ok(())
    }

    fn rename_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()> {
        self.1.datasets.clear();
        self.execute(
            "UPDATE gl_datasets SET name=$3 WHERE project=$1 AND id=$2",
            &[&project, &dataset, &name],
        )
    }

    fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query(
            "SELECT id,name,geometry_type,coordinate_dimension,postgis_source,crs FROM gl_datasets WHERE project=$1 AND id>$2 ORDER BY id LIMIT $3",
            &[&project, &after, &limit],
        )
    }
}

impl SqlTransaction {
    pub(super) fn dataset_table(&mut self, project: &str, dataset: &str) -> Result<Option<String>> {
        let table = self
            .dataset_exists(project, dataset)?
            .map(|row| row.get::<_, Option<String>>(5usize))
            .transpose()?
            .flatten();
        if let Some(table) = &table {
            check_table_name(table)?;
        }
        Ok(table)
    }

    pub(super) fn create_dataset_table(&mut self, project: &str, dataset: &str) -> Result<()> {
        let table = format!("gl_data_{}", uuid::Uuid::new_v4().simple());
        let (collation, object, geometry) = match self.0.backend {
            Backend::Postgis(_) => (
                "\"C\"",
                "jsonb_typeof(properties::jsonb)",
                "geom geometry CHECK(geom IS NULL OR ST_SRID(geom)=0)",
            ),
            Backend::Sqlite(_) => ("BINARY", "json_type(properties)", "geom BLOB,geom_z BLOB"),
        };
        self.batch_execute(&format!(
            "CREATE TABLE {table}(feature_id text COLLATE {collation} PRIMARY KEY NOT NULL,valid_from bigint NOT NULL CHECK(valid_from>0),properties text NOT NULL CHECK({object}='object'),geometry_json text,{geometry})"
        ))?;
        self.execute(
            "UPDATE gl_datasets SET storage_table=$3 WHERE project=$1 AND id=$2",
            &[&project, &dataset, &table],
        )?;
        self.1.datasets.clear();
        match self.0.backend {
            Backend::Postgis(_) => self.batch_execute(&format!(
                "CREATE INDEX {table}_geom ON {table} USING gist(geom); CREATE TRIGGER gl_dataset_write BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON {table} FOR EACH STATEMENT EXECUTE FUNCTION public.gl_source_guard()"
            ))?,
            Backend::Sqlite(_) => {
                for (column, dimension) in [("geom", "XY"), ("geom_z", "XYZ")] {
                    for sql in [
                        format!("SELECT RecoverGeometryColumn('{table}','{column}',0,'GEOMETRY','{dimension}')"),
                        format!("SELECT CreateSpatialIndex('{table}','{column}')"),
                    ] {
                        if self.query_one(&sql, &[])?.get::<_, i32>(0usize)? != 1 {
                            return Err(Error::new(500, "cannot register dataset geometry/index"));
                        }
                    }
                }
                self.batch_execute(&format!(
                    "CREATE TRIGGER {table}_insert BEFORE INSERT ON {table} WHEN NOT gl_dataset_write_allowed() BEGIN SELECT RAISE(ABORT,'write this dataset through GeoLedger'); END;
                     CREATE TRIGGER {table}_update BEFORE UPDATE ON {table} WHEN OLD.feature_id<>NEW.feature_id OR NOT gl_dataset_write_allowed() BEGIN SELECT RAISE(ABORT,'write this dataset through GeoLedger'); END;
                     CREATE TRIGGER {table}_delete BEFORE DELETE ON {table} WHEN NOT gl_dataset_write_allowed() BEGIN SELECT RAISE(ABORT,'write this dataset through GeoLedger'); END;"
                ))?;
            }
        }
        Ok(())
    }

    pub(super) fn drop_dataset_table(&mut self, project: &str, dataset: &str) -> Result<()> {
        let Some(table) = self.dataset_table(project, dataset)? else {
            return Ok(());
        };
        // Keep the physical relation alive while a reader holds its metadata snapshot.
        self.query_one(
            &self.lock_sql(
                "SELECT id FROM gl_datasets WHERE project=$1 AND id=$2",
                "FOR UPDATE",
            ),
            &[&project, &dataset],
        )?;
        if matches!(self.0.backend, Backend::Sqlite(_)) {
            for column in ["geom", "geom_z"] {
                for sql in [
                    format!("SELECT DisableSpatialIndex('{table}','{column}')"),
                    format!("SELECT DiscardGeometryColumn('{table}','{column}')"),
                ] {
                    if self.query_one(&sql, &[])?.get::<_, i32>(0usize)? != 1 {
                        return Err(Error::new(500, "cannot remove dataset geometry/index"));
                    }
                }
                self.batch_execute(&format!("DROP TABLE idx_{table}_{column}"))?;
            }
        }
        self.batch_execute(&format!("DROP TABLE {table}"))?;
        self.execute(
            "UPDATE gl_datasets SET storage_table=NULL WHERE project=$1 AND id=$2",
            &[&project, &dataset],
        )?;
        self.1.datasets.clear();
        Ok(())
    }

    pub(super) fn write_dataset_table(
        &mut self,
        project: &str,
        dataset: &str,
        revision: i64,
    ) -> Result<()> {
        let Some(table) = self.dataset_table(project, dataset)? else {
            return Ok(());
        };
        if matches!(self.0.backend, Backend::Postgis(_)) {
            self.query_one("SELECT set_config('geoledger.publication_table',($1::text::regclass::oid)::text,true)", &[&table])?;
        }
        self.with_dataset_writes(|t| {
            t.execute(&format!("DELETE FROM {table} WHERE feature_id IN (SELECT feature_id FROM gl_merge_stage WHERE dataset=$1 AND after_value IS NULL)"), &[&dataset])?;
            let columns = t.geometry_columns();
            let assignments = columns.split(',').map(|c| format!("{c}=excluded.{c}")).collect::<Vec<_>>().join(",");
            t.execute(&format!("INSERT INTO {table}(feature_id,valid_from,properties,geometry_json,{columns}) SELECT feature_id,$2,gl_json_field(after_value,'properties'),gl_json_field(after_value,'geometry'),{values} FROM gl_merge_stage WHERE dataset=$1 AND after_value IS NOT NULL ON CONFLICT(feature_id) DO UPDATE SET valid_from=excluded.valid_from,properties=excluded.properties,geometry_json=excluded.geometry_json,{assignments}", values=t.geometry_values("gl_json_field(after_value,'geometry')")), &[&dataset,&revision])
        })?;
        if matches!(self.0.backend, Backend::Postgis(_)) {
            self.batch_execute("SET LOCAL geoledger.publication_table = ''")?;
        }
        Ok(())
    }

    pub(super) fn with_dataset_writes<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        struct Reset(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Reset {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Relaxed);
            }
        }
        self.0.dataset_write_allowed.store(true, Ordering::Relaxed);
        let _reset = Reset(self.0.dataset_write_allowed.clone());
        action(self)
    }
}

pub(super) fn check_table_name(table: &str) -> Result<()> {
    if table.len() != 40
        || !table.starts_with("gl_data_")
        || !table[8..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::new(500, "invalid dataset storage table"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Application;
    use serde_json::{Value, json};

    fn dataset_tables(storage: Storage) -> Result<()> {
        let app = Application::new(storage.clone());
        app.migrate()?;
        let mut db = Client::open(
            &storage,
            &Arc::new(postgres::Pool::default()),
            Duration::from_secs(30),
        )?;
        let project =
            app.execute("alice", "create_project", json!({"name":"separate tables"}))?["project"]
                .as_str()
                .ok_or_else(crate::bad)?
                .to_owned();
        let call = |op: &str, mut value: Value| {
            value["project"] = json!(project);
            app.execute("alice", op, value)
        };
        let mut datasets = Vec::new();
        let mut tables = Vec::new();
        for (name, family) in [
            ("points", "point"),
            ("roads", "line"),
            ("buildings", "polygon"),
            ("points '; DROP TABLE gl_projects; --", "point"),
        ] {
            let dataset = call(
                "create_dataset",
                json!({"name":name,"geometry_type":family}),
            )?["dataset"]
                .as_str()
                .ok_or_else(crate::bad)?
                .to_owned();
            let table: String = db
                .query_one(
                    "SELECT storage_table FROM gl_datasets WHERE project=$1 AND id=$2",
                    &[&project, &dataset],
                )?
                .get(0usize)?;
            check_table_name(&table)?;
            assert!(!tables.contains(&table));
            assert_eq!(
                db.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
                    .get::<_, i64>(0usize)?,
                0
            );
            datasets.push(dataset);
            tables.push(table);
        }
        let geometries = [
            json!({"type":"Point","coordinates":[1,2]}),
            json!({"type":"LineString","coordinates":[[1,2],[3,4]]}),
            json!({"type":"Polygon","coordinates":[[[0,0],[5,0],[5,5],[0,0]]]}),
            Value::Null,
        ];
        let edits = datasets.iter().zip(&geometries).map(|(dataset, geometry)| json!({"dataset":dataset,"feature_id":"same-key","feature":{"type":"Feature","id":"same-key","properties":{"exact":18446744073709551615u64},"geometry":geometry}})).collect::<Vec<_>>();
        let workspace = call("create_workspace", json!({}))?["workspace"].clone();
        call(
            "save",
            json!({"workspace":workspace,"expected_workspace_version":0,"edits":edits}),
        )?;
        for table in &tables {
            assert_eq!(
                db.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
                    .get::<_, i64>(0usize)?,
                0
            );
        }
        let request = json!({"workspace":workspace,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"all tables"});
        // A failure after the table writes must roll back data, history and the receipt together.
        match db.backend {
            Backend::Sqlite(_) => db.batch_execute("CREATE TRIGGER fail_table_publish BEFORE INSERT ON gl_idempotency BEGIN SELECT RAISE(ABORT,'publication rollback'); END")?,
            Backend::Postgis(_) => db.batch_execute("CREATE FUNCTION fail_table_publish() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'publication rollback'; END $$; CREATE TRIGGER fail_table_publish BEFORE INSERT ON gl_idempotency FOR EACH ROW EXECUTE FUNCTION fail_table_publish()")?,
        }
        assert!(call("publish", request.clone()).is_err());
        for table in &tables {
            assert_eq!(
                db.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
                    .get::<_, i64>(0usize)?,
                0
            );
        }
        assert_eq!(
            db.query_one("SELECT head FROM gl_projects WHERE id=$1", &[&project])?
                .get::<_, i64>(0usize)?,
            0
        );
        assert_eq!(
            db.query_one(
                "SELECT count(*) FROM gl_history WHERE project=$1",
                &[&project]
            )?
            .get::<_, i64>(0usize)?,
            0
        );
        match db.backend {
            Backend::Sqlite(_) => db.batch_execute("DROP TRIGGER fail_table_publish")?,
            Backend::Postgis(_) => db.batch_execute("DROP TRIGGER fail_table_publish ON gl_idempotency; DROP FUNCTION fail_table_publish()")?,
        }
        let receipt = call("publish", request.clone())?;
        assert_eq!(receipt, call("publish", request)?);
        if matches!(storage, Storage::Postgis(_)) {
            let mut reader = Client::open(
                &storage,
                &Arc::new(postgres::Pool::default()),
                Duration::from_secs(30),
            )?
            .transaction(true)?;
            reader.dataset_exists(&project, &datasets[0])?;
            assert!(
                app.clone()
                    .with_timeout(Duration::from_millis(100))
                    .execute(
                        "alice",
                        "delete_dataset",
                        json!({"project":project,"dataset":datasets[0],"confirm_name":"points"})
                    )
                    .is_err()
            );
            assert_eq!(
                reader
                    .query_one(&format!("SELECT count(*) FROM {}", tables[0]), &[])?
                    .get::<_, i64>(0usize)?,
                1
            );
            reader.commit()?;
        }
        for (i, table) in tables.iter().enumerate() {
            let row = db.query_one(
                &format!(
                    "SELECT properties,geometry_json FROM {table} WHERE feature_id='same-key'"
                ),
                &[],
            )?;
            assert_eq!(
                crate::codec::stored::<Value>(&row.get::<_, String>(0usize)?)?["exact"],
                18446744073709551615u64
            );
            assert_eq!(
                row.get::<_, Option<String>>(1usize)?
                    .map(|v| crate::codec::stored::<Value>(&v))
                    .transpose()?
                    .unwrap_or(Value::Null),
                geometries[i]
            );
            assert!(db.execute(&format!("DELETE FROM {table}"), &[]).is_err());
            assert!(
                db.execute(&format!("UPDATE {table} SET properties='{{}}'"), &[])
                    .is_err()
            );
            assert_eq!(
                call("features", json!({"dataset":datasets[i],"limit":1}))?["features"][0]["geometry"],
                geometries[i]
            );
        }
        call(
            "rename_dataset",
            json!({"dataset":datasets[0],"name":"renamed points"}),
        )?;
        assert_eq!(
            db.query_one(
                "SELECT storage_table FROM gl_datasets WHERE project=$1 AND id=$2",
                &[&project, &datasets[0]]
            )?
            .get::<_, String>(0usize)?,
            tables[0]
        );
        let next = call("create_workspace", json!({}))?["workspace"].clone();
        call(
            "save",
            json!({"workspace":next,"expected_workspace_version":0,"edits":[{"dataset":datasets[0],"feature_id":"same-key","feature":null}]}),
        )?;
        call(
            "publish",
            json!({"workspace":next,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"delete point"}),
        )?;
        assert_eq!(
            db.query_one(&format!("SELECT count(*) FROM {}", tables[0]), &[])?
                .get::<_, i64>(0usize)?,
            0
        );
        assert_eq!(
            call("features", json!({"dataset":datasets[0]}))?["features"],
            json!([])
        );
        assert_eq!(
            call("features", json!({"dataset":datasets[0],"revision":1}))?["features"][0]["geometry"],
            geometries[0]
        );
        call(
            "delete_dataset",
            json!({"dataset":datasets[0],"confirm_name":"renamed points"}),
        )?;
        assert!(
            db.query(&format!("SELECT * FROM {}", tables[0]), &[])
                .is_err()
        );
        for table in &tables[1..] {
            assert_eq!(
                db.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
                    .get::<_, i64>(0usize)?,
                1
            );
        }
        call("delete_project", json!({"confirm_name":"separate tables"}))?;
        for table in &tables {
            let exists = match db.backend {
                Backend::Sqlite(_) => db
                    .query_opt(
                        "SELECT 1 FROM sqlite_master WHERE name=$1 OR name=$2",
                        &[table, &format!("idx_{table}_geom")],
                    )?
                    .is_some(),
                Backend::Postgis(_) => db
                    .query_one("SELECT to_regclass($1::text) IS NOT NULL", &[table])?
                    .get::<_, bool>(0usize)?,
            };
            assert!(!exists, "dataset table and indexes must be removed");
        }
        Ok(())
    }

    #[test]
    fn sqlite_dataset_tables_are_isolated_and_atomic() -> Result<()> {
        let dir =
            tempfile::tempdir().map_err(|e| Error::new(500, "temporary directory").caused_by(e))?;
        dataset_tables(Storage::Sqlite(dir.path().join("tables.db")))
    }

    #[test]
    #[ignore = "requires isolated geoledger_test PostGIS database"]
    fn postgis_dataset_tables_are_isolated_and_atomic() -> Result<()> {
        let dsn = std::env::var("GL_TEST_DATABASE_URL")
            .map_err(|e| Error::new(500, "test database").caused_by(e))?;
        let mut db = Client::open(
            &Storage::Postgis(dsn.clone()),
            &Arc::new(postgres::Pool::default()),
            Duration::from_secs(30),
        )?;
        assert_eq!(
            db.query_one("SELECT current_database()::text", &[])?
                .get::<_, String>(0usize)?,
            "geoledger_test"
        );
        dataset_tables(Storage::Postgis(dsn))
    }
}
