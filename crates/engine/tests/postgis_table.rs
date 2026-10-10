use geoledger_engine::{Application, Policy, Storage};
use serde_json::{Value, json};
type TestResult = Result<(), Box<dyn std::error::Error>>;
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn existing_table_publication_is_atomic() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = postgres::Client::connect(&dsn, postgres::NoTls)?;
    assert_eq!(
        admin
            .query_one("SELECT current_database()", &[])?
            .get::<_, String>(0),
        "geoledger_test"
    );
    let name = format!("geoledger_binding_{}", uuid::Uuid::new_v4().simple());
    admin.batch_execute(&format!("CREATE DATABASE {name}"))?;
    let result = (|| -> TestResult {
        let dsn = dsn.replacen("/geoledger_test", &format!("/{name}"), 1);
        let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
        db.batch_execute("CREATE EXTENSION postgis; CREATE SCHEMA business; CREATE TABLE business.points(id bigint PRIMARY KEY,label text NOT NULL CHECK(label <> 'invalid'), n numeric(20,0), geom geometry(PointZ,4326)); INSERT INTO business.points VALUES(1,'original',18446744073709551615,ST_SetSRID(ST_MakePoint(1,2,3),4326))")?;
        let app = Application::new(Storage::Postgis(dsn)).with_policy(Policy {
            admins: ["alice".into()].into(),
            ..Policy::default()
        });
        app.migrate()?;
        let p = app.execute("alice", "create_project", json!({"name":"bound"}))?["project"].clone();
        let run = |op: &str, mut value: Value| {
            value["project"] = p.clone();
            app.execute("alice", op, value)
        };
        let source =
            json!({"schema":"business","table":"points","id_column":"id","geometry_column":"geom"});
        run("set_member", json!({"subject":"bob","role":"editor"}))?;
        let denied = app
            .execute(
                "bob",
                "create_dataset",
                json!({"project":p,"name":"unauthorized","postgis_table":source}),
            )
            .err()
            .ok_or("non-admin attached a table")?;
        assert_eq!(denied.status, 403);
        let created = run(
            "create_dataset",
            json!({"name":"existing","postgis_table":source}),
        )?;
        let d = created["dataset"].clone();
        assert_eq!(created["geometry_type"], "point");
        assert_eq!(created["coordinate_dimension"], 3);
        let features = run("features", json!({"dataset":d}))?;
        assert_eq!(
            features["features"][0]["properties"]["n"],
            18446744073709551615u64
        );
        assert!(
            run(
                "create_dataset",
                json!({"name":"duplicate","postgis_table":source})
            )
            .is_err()
        );
        assert!(
            db.batch_execute("UPDATE business.points SET label='outside'")
                .is_err()
        );
        let mut exported = Vec::new();
        app.export_data(&mut exported)?;
        assert!(!String::from_utf8_lossy(&exported).contains("postgis_source"));
        let temp = tempfile::tempdir()?;
        let imported = Application::new(Storage::Sqlite(temp.path().join("import.db")));
        imported.import_data(&mut std::io::Cursor::new(exported))?;
        assert!(
            imported.execute("alice", "list_datasets", json!({"project":p}))?[0]["postgis_table"]
                .is_null()
        );
        assert_eq!(
            imported.execute("alice", "features", json!({"project":p,"dataset":d}))?["features"],
            features["features"]
        );
        // A full PostgreSQL restore recreates relation OIDs; names/schema/data define the binding.
        db.batch_execute("ALTER TABLE business.points RENAME TO points_before_restore; CREATE TABLE business.points (LIKE business.points_before_restore INCLUDING ALL); INSERT INTO business.points SELECT * FROM business.points_before_restore; CREATE TRIGGER gl_versioned_write BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON business.points FOR EACH STATEMENT EXECUTE FUNCTION gl_source_guard(); CREATE TRIGGER gl_versioned_track AFTER INSERT OR UPDATE OR DELETE ON business.points FOR EACH ROW EXECUTE FUNCTION gl_source_track('id'); CREATE TRIGGER gl_versioned_truncate AFTER TRUNCATE ON business.points FOR EACH STATEMENT EXECUTE FUNCTION gl_source_track(); ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_track; ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_truncate; DROP TABLE business.points_before_restore")?;
        let binding: String = db.query_one("SELECT binding FROM gl_source_state WHERE schema_name='business' AND table_name='points'", &[])?.get(0);
        for trigger in [
            "gl_versioned_write",
            "gl_versioned_track",
            "gl_versioned_truncate",
        ] {
            db.batch_execute(&format!(
                "COMMENT ON TRIGGER {trigger} ON business.points IS '{binding}'"
            ))?;
        }
        let feature = |id: &str, label: &str| json!({"type":"Feature","id":id,"properties":{"label":label,"n":18446744073709551615u64},"geometry":{"type":"Point","coordinates":[10,20,30]}});
        let w = run("create_workspace", json!({}))?["workspace"].clone();
        run(
            "save",
            json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"1","feature":feature("1","edited")},{"dataset":d,"feature_id":"2","feature":feature("2","invalid")}]}),
        )?;
        let req = json!({"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"atomic"});
        assert!(run("publish", req).is_err());
        assert_eq!(
            db.query_one("SELECT label FROM business.points WHERE id=1", &[])?
                .get::<_, String>(0),
            "original"
        );
        assert_eq!(run("get_project", json!({}))?["head"], 1);
        run(
            "save",
            json!({"workspace":w,"expected_workspace_version":1,"edits":[{"dataset":d,"feature_id":"2","feature":feature("2","inserted")}]}),
        )?;
        let req = json!({"workspace":w,"expected_workspace_version":2,"request_id":uuid::Uuid::new_v4(),"message":"publish"});
        let result = run("publish", req.clone())?;
        assert_eq!(result, run("publish", req)?);
        assert_eq!(
            db.query_one("SELECT label FROM business.points WHERE id=1", &[])?
                .get::<_, String>(0),
            "edited"
        );
        assert_eq!(
            db.query_one("SELECT count(*) FROM business.points", &[])?
                .get::<_, i64>(0),
            2
        );
        let w = run("create_workspace", json!({}))?["workspace"].clone();
        run(
            "save",
            json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"1","feature":null}]}),
        )?;
        run(
            "publish",
            json!({"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"delete"}),
        )?;
        assert_eq!(
            db.query_one("SELECT count(*) FROM business.points", &[])?
                .get::<_, i64>(0),
            1
        );
        let undo = run("restore", json!({"revision":3}))?;
        run(
            "publish",
            json!({"workspace":undo["workspace"],"expected_workspace_version":undo["version"],"request_id":uuid::Uuid::new_v4(),"message":"undo"}),
        )?;
        assert_eq!(
            db.query_one("SELECT count(*) FROM business.points", &[])?
                .get::<_, i64>(0),
            2
        );
        let publication = |id: &str, label: &str| -> Result<Value, geoledger_engine::Error> {
            let w = run("create_workspace", json!({}))?["workspace"].clone();
            run(
                "save",
                json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":id,"feature":feature(id,label)}]}),
            )?;
            run(
                "publish",
                json!({"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"candidate validation"}),
            )
        };
        let rejected = |id: &str, label: &str, status: u16| -> TestResult {
            let error = publication(id, label)
                .err()
                .ok_or("invalid source publication accepted")?;
            assert_eq!(error.status, status);
            Ok(())
        };
        // Native key casts must not create a second version identity for the same business row.
        rejected("01", "alias", 400)?;
        // A business trigger can change a row absent from the submitted edits; both writes roll back.
        db.batch_execute("CREATE FUNCTION business.side_effect() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id=1 THEN UPDATE business.points SET label='side effect' WHERE id=2; END IF; RETURN NEW; END $$; CREATE TRIGGER side_effect AFTER UPDATE ON business.points FOR EACH ROW EXECUTE FUNCTION business.side_effect()")?;
        rejected("1", "candidate", 409)?;
        assert_eq!(
            db.query_one("SELECT label FROM business.points WHERE id=2", &[])?
                .get::<_, String>(0),
            "inserted"
        );
        assert_eq!(
            db.query_one("SELECT count(*) FROM gl_source_changes", &[])?
                .get::<_, i64>(0),
            0
        );
        db.batch_execute("DROP TRIGGER side_effect ON business.points; CREATE CONSTRAINT TRIGGER side_effect AFTER UPDATE ON business.points DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION business.side_effect()")?;
        rejected("1", "deferred candidate", 409)?;
        assert_eq!(
            db.query_one("SELECT label FROM business.points WHERE id=2", &[])?
                .get::<_, String>(0),
            "inserted"
        );
        db.batch_execute(
            "DROP TRIGGER side_effect ON business.points; DROP FUNCTION business.side_effect()",
        )?;
        // Disabling and restoring all tracking in one transaction invalidates the catalog proof.
        db.batch_execute("BEGIN; ALTER TABLE business.points DISABLE TRIGGER USER; UPDATE business.points SET label='outside' WHERE id=2; ALTER TABLE business.points ENABLE TRIGGER gl_versioned_write; ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_track; ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_truncate; COMMIT")?;
        rejected("1", "candidate", 409)?;
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); UPDATE business.points SET label='inserted' WHERE id=2; RESET geoledger.publication_table")?;
        // ALWAYS tracking also records privileged replica-mode writes while the statement guard is skipped.
        db.batch_execute("SET session_replication_role=replica; UPDATE business.points SET label='replica' WHERE id=2; RESET session_replication_role")?;
        rejected("1", "candidate", 409)?;
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); UPDATE business.points SET label='inserted' WHERE id=2; RESET geoledger.publication_table")?;
        // Both sides of an identity change are candidates.
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); UPDATE business.points SET id=3 WHERE id=2; RESET geoledger.publication_table")?;
        rejected("1", "candidate", 409)?;
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); UPDATE business.points SET id=2 WHERE id=3; RESET geoledger.publication_table")?;
        // TRUNCATE has no row events: its transactional flag forces full verification.
        db.batch_execute("CREATE TEMP TABLE source_backup AS SELECT * FROM business.points")?;
        db.batch_execute("BEGIN; SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,true); TRUNCATE business.points; ROLLBACK")?;
        assert!(
            !db.query_one("SELECT dirty_all FROM gl_source_state", &[])?
                .get::<_, bool>(0)
        );
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); TRUNCATE business.points; RESET geoledger.publication_table")?;
        rejected("1", "candidate", 409)?;
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); INSERT INTO business.points SELECT * FROM source_backup; RESET geoledger.publication_table; DROP TABLE source_backup")?;
        publication("1", "candidate")?;
        assert_eq!(
            db.query_one("SELECT count(*) FROM gl_source_changes", &[])?
                .get::<_, i64>(0),
            0
        );
        assert!(
            !db.query_one("SELECT dirty_all FROM gl_source_state", &[])?
                .get::<_, bool>(0)
        );
        // Deferred no-op updates on the first source retain its publication authorization.
        db.batch_execute("CREATE TABLE business.zzpoints (LIKE business.points INCLUDING ALL); INSERT INTO business.zzpoints SELECT * FROM business.points; CREATE FUNCTION business.deferred_noop() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id=1 THEN UPDATE business.points SET label=label WHERE id=2; END IF; RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER deferred_noop AFTER UPDATE ON business.points DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION business.deferred_noop()")?;
        let other = run("create_dataset",json!({"name":"second source","postgis_table":{"schema":"business","table":"zzpoints","id_column":"id","geometry_column":"geom"}}))?["dataset"].clone();
        let both = run("create_workspace", json!({}))?["workspace"].clone();
        run(
            "save",
            json!({"workspace":both,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"1","feature":feature("1","two tables")},{"dataset":other,"feature_id":"1","feature":feature("1","second table") }]}),
        )?;
        run(
            "publish",
            json!({"workspace":both,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"two sources with deferred trigger"}),
        )?;
        db.batch_execute(
            "DROP TRIGGER deferred_noop ON business.points; DROP FUNCTION business.deferred_noop()",
        )?;
        // Even privileged bypasses must be detected instead of overwritten.
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); UPDATE business.points SET label='outside' WHERE id=1; RESET geoledger.publication_table")?;
        let w = run("create_workspace", json!({}))?["workspace"].clone();
        run(
            "save",
            json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"1","feature":feature("1","overwrite")}]}),
        )?;
        let error=run("publish",json!({"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"drift"})).err().ok_or("drift accepted")?;
        assert_eq!(error.status, 409);
        run(
            "delete_dataset",
            json!({"dataset":d,"confirm_name":"existing"}),
        )?;
        db.batch_execute("UPDATE business.points SET label='released' WHERE id=1")?;
        assert_eq!(
            db.query_one("SELECT count(*) FROM business.points", &[])?
                .get::<_, i64>(0),
            2
        );
        Ok(())
    })();
    admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))?;
    if let Err(error) = &result {
        let mut source = error.source();
        while let Some(error) = source {
            eprintln!("cause: {error:?}");
            source = error.source();
        }
    }
    result
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn deferred_constraints_and_schema_drift() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = postgres::Client::connect(&dsn, postgres::NoTls)?;
    assert_eq!(
        admin
            .query_one("SELECT current_database()", &[])?
            .get::<_, String>(0),
        "geoledger_test"
    );
    let name = format!("geoledger_deferred_{}", uuid::Uuid::new_v4().simple());
    admin.batch_execute(&format!("CREATE DATABASE {name}"))?;
    let result = (|| -> TestResult {
        let dsn = dsn.replacen("/geoledger_test", &format!("/{name}"), 1);
        let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
        db.batch_execute("CREATE EXTENSION postgis; CREATE SCHEMA business; CREATE TABLE business.zz_parent(id bigint PRIMARY KEY,label text,geom geometry(Point,4326)); CREATE TABLE business.aa_child(id bigint PRIMARY KEY,parent_id bigint REFERENCES business.zz_parent(id) DEFERRABLE INITIALLY DEFERRED,geom geometry(Point,4326)); CREATE TABLE business.deferred_key(id bigint PRIMARY KEY DEFERRABLE INITIALLY DEFERRED,label text,geom geometry(Point,4326))")?;
        let app = Application::new(Storage::Postgis(dsn)).with_policy(Policy {
            admins: ["alice".into()].into(),
            ..Policy::default()
        });
        app.migrate()?;
        let p =
            app.execute("alice", "create_project", json!({"name":"deferred"}))?["project"].clone();
        let run = |op: &str, mut value: Value| {
            value["project"] = p.clone();
            app.execute("alice", op, value)
        };
        let attach = |table: &str| {
            run(
                "create_dataset",
                json!({"name":table,"postgis_table":{"schema":"business","table":table,"id_column":"id","geometry_column":"geom"}}),
            )
        };
        let parent = attach("zz_parent")?["dataset"].clone();
        let child = attach("aa_child")?["dataset"].clone();
        let deferred = attach("deferred_key")?["dataset"].clone();
        let feature = |id: &str, props: Value| json!({"type":"Feature","id":id,"properties":props,"geometry":{"type":"Point","coordinates":[1,2]}});
        let publish = |edits: Value| {
            let w = run("create_workspace", json!({}))?["workspace"].clone();
            run(
                "save",
                json!({"workspace":w,"expected_workspace_version":0,"edits":edits}),
            )?;
            run(
                "publish",
                json!({"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"deferred regression"}),
            )
        };
        // The child sorts before the parent; both requested rows must exist before FK checks.
        publish(json!([
            {"dataset":child,"feature_id":"1","feature":feature("1",json!({"parent_id":1}))},
            {"dataset":parent,"feature_id":"1","feature":feature("1",json!({"label":"parent"}))},
            {"dataset":deferred,"feature_id":"1","feature":feature("1",json!({"label":"inserted"}))}
        ]))?;
        publish(
            json!([{"dataset":deferred,"feature_id":"1","feature":feature("1",json!({"label":"updated"}))}]),
        )?;
        assert_eq!(
            db.query_one("SELECT label FROM business.deferred_key WHERE id=1", &[])?
                .get::<_, String>(0),
            "updated"
        );
        publish(json!([{"dataset":deferred,"feature_id":"1","feature":null}]))?;
        assert_eq!(
            db.query_one("SELECT count(*) FROM business.deferred_key", &[])?
                .get::<_, i64>(0),
            0
        );
        let head = run("get_project", json!({}))?["head"].clone();
        assert!(publish(json!([
            {"dataset":parent,"feature_id":"1","feature":feature("1",json!({"label":"must rollback"}))},
            {"dataset":child,"feature_id":"2","feature":feature("2",json!({"parent_id":999}))}
        ])).is_err());
        assert_eq!(run("get_project", json!({}))?["head"], head);
        assert_eq!(
            db.query_one("SELECT label FROM business.zz_parent WHERE id=1", &[])?
                .get::<_, String>(0),
            "parent"
        );
        assert_eq!(
            db.query_one("SELECT count(*) FROM business.aa_child", &[])?
                .get::<_, i64>(0),
            1
        );
        // Deferred cross-source side effects remain guarded and must match the requested result.
        db.batch_execute("CREATE FUNCTION business.side_effect() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN UPDATE business.zz_parent SET label='side effect' WHERE id=1; RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER side_effect AFTER INSERT ON business.aa_child DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION business.side_effect()")?;
        let error = publish(json!([
            {"dataset":parent,"feature_id":"1","feature":feature("1",json!({"label":"requested"}))},
            {"dataset":child,"feature_id":"2","feature":feature("2",json!({"parent_id":1}))}
        ]))
        .err()
        .ok_or("deferred side effect accepted")?;
        assert_eq!(error.status, 409);
        assert_eq!(
            db.query_one("SELECT label FROM business.zz_parent WHERE id=1", &[])?
                .get::<_, String>(0),
            "parent"
        );
        db.batch_execute("DROP TRIGGER side_effect ON business.aa_child; ALTER TABLE business.deferred_key ADD COLUMN extra text")?;
        run(
            "delete_dataset",
            json!({"dataset":deferred,"confirm_name":"deferred_key"}),
        )?;
        db.batch_execute("INSERT INTO business.deferred_key(id,label) VALUES(7,'released')")?;
        // A renamed source is released; an unrelated replacement keeps its same-named trigger.
        db.batch_execute("ALTER TABLE business.aa_child RENAME TO renamed_child; CREATE TABLE business.aa_child(id bigint PRIMARY KEY,geom geometry(Point,4326)); CREATE FUNCTION business.unrelated_guard() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$; CREATE TRIGGER gl_versioned_write BEFORE INSERT ON business.aa_child FOR EACH STATEMENT EXECUTE FUNCTION business.unrelated_guard()")?;
        run(
            "delete_dataset",
            json!({"dataset":child,"confirm_name":"aa_child"}),
        )?;
        assert_eq!(db.query_one("SELECT count(*) FROM pg_trigger WHERE tgrelid='business.aa_child'::regclass AND tgname='gl_versioned_write'",&[])?.get::<_,i64>(0),1);
        assert_eq!(db.query_one("SELECT count(*) FROM pg_trigger WHERE tgrelid='business.renamed_child'::regclass AND tgname LIKE 'gl_versioned_%'",&[])?.get::<_,i64>(0),0);
        db.batch_execute(
            "DELETE FROM business.renamed_child; DROP TABLE business.zz_parent CASCADE",
        )?;
        run(
            "delete_dataset",
            json!({"dataset":parent,"confirm_name":"zz_parent"}),
        )?;
        assert_eq!(
            db.query_one("SELECT count(*) FROM gl_source_state", &[])?
                .get::<_, i64>(0),
            0
        );
        Ok(())
    })();
    admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))?;
    result
}
