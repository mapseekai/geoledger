mod support;
use geoledger_engine::{Application, Storage, StorageOptions};
use serde_json::{Value, json};
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn registration_state(
    db: &mut postgres::Client,
    project: &str,
    table: &str,
) -> Result<Vec<i64>, Box<dyn std::error::Error>> {
    Ok(db.query_one(
        "SELECT ARRAY[(SELECT head FROM gl_projects WHERE id=$1),(SELECT count(*) FROM gl_datasets),(SELECT count(*) FROM gl_workspaces),(SELECT count(*) FROM gl_commits),(SELECT count(*) FROM gl_source_state),(SELECT count(*) FROM gl_source_versions),(SELECT count(*) FROM gl_history),(SELECT count(*) FROM gl_commit_changes),(SELECT count(*) FROM gl_audit_events),(SELECT count(*) FROM pg_index WHERE indrelid=$2::text::regclass),(SELECT count(*) FROM pg_trigger WHERE tgrelid=$2::text::regclass AND NOT tgisinternal)]",
        &[&project, &table],
    )?.get(0))
}

#[test]
#[ignore = "explicit million-row PostGIS enrollment capacity probe"]
fn million_row_enrollment_keeps_only_source_rows() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    support::reset_database(&mut db)?;
    let result = (|| -> TestResult {
        db.batch_execute("CREATE SCHEMA business; CREATE TABLE business.capacity_source(id text COLLATE \"C\" PRIMARY KEY, payload text NOT NULL, geom geometry(Point,4326)); INSERT INTO business.capacity_source SELECT lpad(i::text,7,'0'),repeat('x',512),ST_SetSRID(ST_MakePoint(i%180,0),4326) FROM generate_series(1,1000000) i; CREATE INDEX capacity_source_geom_idx ON business.capacity_source USING gist(geom)")?;
        let app = Application::with_options(
            Storage::Postgis(dsn),
            StorageOptions {
                statement_timeout: std::time::Duration::from_secs(300),
                ..Default::default()
            },
        )
        .with_timeout(std::time::Duration::from_secs(300));
        app.migrate()?;
        app.bootstrap_admin("alice", "initial-test-password")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":"initial-test-password"}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let project =
            app.execute(alice, "create_project", json!({"name":"capacity"}))?["project"].clone();
        let started = std::time::Instant::now();
        let dataset=app.execute(alice,"create_dataset",json!({"project":project,"name":"source","postgis_table":{"schema":"business","table":"capacity_source","id_column":"id","geometry_column":"geom"}}))?["dataset"].clone();
        let registration = started.elapsed();
        let counts=db.query_one("SELECT (SELECT count(*) FROM business.capacity_source),(SELECT count(*) FROM gl_history),(SELECT count(*) FROM gl_commit_changes),(SELECT count(*) FROM gl_source_versions)",&[])?;
        assert_eq!(
            (
                counts.get::<_, i64>(0),
                counts.get::<_, i64>(1),
                counts.get::<_, i64>(2),
                counts.get::<_, i64>(3)
            ),
            (1_000_000, 0, 0, 0)
        );
        let order_index: String = db.query_one("SELECT postgis_source::jsonb->>'order_index' FROM gl_datasets WHERE project=$1 AND id=$2", &[&project.as_str().ok_or("project")?,&dataset.as_str().ok_or("dataset")?])?.get(0);
        assert!(
            order_index.is_empty(),
            "C-collated text primary key already supplies ID order"
        );
        let plan=db.query("EXPLAIN (COSTS OFF) SELECT id::text COLLATE \"C\" FROM business.capacity_source WHERE id::text COLLATE \"C\">'0999900' ORDER BY id::text COLLATE \"C\" LIMIT 100",&[])?.iter().map(|row|row.get::<_,String>(0)).collect::<Vec<_>>().join("\n");
        assert!(
            plan.contains("capacity_source_pkey"),
            "tail page must reuse source primary key: {plan}"
        );
        let spatial_plan=db.query("EXPLAIN (COSTS OFF) SELECT id FROM business.capacity_source WHERE ST_Intersects(geom,ST_MakeEnvelope(89.9,-0.1,90.1,0.1,4326))",&[])?.iter().map(|row|row.get::<_,String>(0)).collect::<Vec<_>>().join("\n");
        assert!(
            spatial_plan.contains("capacity_source_geom_idx"),
            "bbox must reuse the business GiST index: {spatial_plan}"
        );
        let page = app.execute(
            alice,
            "features",
            json!({"project":project,"dataset":dataset,"after":"0999900","limit":100}),
        )?;
        assert_eq!(page["features"].as_array().ok_or("features")?.len(), 100);
        assert_eq!(page["features"][99]["id"], "1000000");
        let w = app.execute(alice, "create_workspace", json!({"project":project}))?["workspace"]
            .clone();
        let feature = json!({"type":"Feature","id":"0000001","properties":{"payload":"changed"},"geometry":{"type":"Point","coordinates":[1,0]}});
        app.execute(alice,"save",json!({"project":project,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":dataset,"feature_id":"0000001","feature":feature}]}))?;
        let edit = std::time::Instant::now();
        app.execute(alice,"publish",json!({"project":project,"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"one edit"}))?;
        let publish = edit.elapsed();
        let counts=db.query_one("SELECT (SELECT count(*) FROM business.capacity_source),(SELECT count(*) FROM gl_history),(SELECT count(*) FROM gl_commit_changes),(SELECT count(*) FROM gl_source_versions)",&[])?;
        assert_eq!(
            (
                counts.get::<_, i64>(0),
                counts.get::<_, i64>(1),
                counts.get::<_, i64>(2),
                counts.get::<_, i64>(3)
            ),
            (1_000_000, 1, 1, 1)
        );
        let old = app.execute(
            alice,
            "features",
            json!({"project":project,"dataset":dataset,"feature_id":"0000001","revision":1}),
        )?;
        assert_eq!(
            old["properties"]["payload"].as_str().map(str::len),
            Some(512)
        );
        let new = app.execute(
            alice,
            "features",
            json!({"project":project,"dataset":dataset,"feature_id":"0000001"}),
        )?;
        assert_eq!(new["properties"]["payload"], "changed");
        eprintln!(
            "million_enrollment rows=1000000 registration_secs={:.3} single_publish_secs={:.3} history_rows=1 commit_keys=1 source_versions=1",
            registration.as_secs_f64(),
            publish.as_secs_f64()
        );
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn registration_pages_native_primary_keys_without_losing_rows() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    support::reset_database(&mut db)?;
    let result = (|| -> TestResult {
        db.batch_execute("CREATE SCHEMA business")?;
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        app.bootstrap_admin("alice", "initial-test-password")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":"initial-test-password"}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let project =
            app.execute(alice, "create_project", json!({"name":"paging"}))?["project"].clone();
        for (table, kind, value) in [
            ("small_keys", "smallint", "i"),
            ("integer_keys", "integer", "i"),
            ("big_keys", "bigint", "i"),
            ("uuid_keys", "uuid", "md5(i::text)::uuid"),
            ("text_keys", "text COLLATE \"POSIX\"", "'道路-' || i::text"),
            (
                "varchar_keys",
                "varchar(64) COLLATE \"POSIX\"",
                "'道路-' || i::text",
            ),
            (
                "varchar_c_keys",
                "varchar(64) COLLATE \"C\"",
                "'道路-' || i::text",
            ),
        ] {
            db.batch_execute(&format!(
                "CREATE TABLE business.{table}(id {kind} PRIMARY KEY, n integer NOT NULL, geom geometry(Point,4326)); INSERT INTO business.{table} SELECT {value},i,ST_SetSRID(ST_MakePoint(1,2),4326) FROM generate_series(-1250,1250) i"
            ))?;
            if table == "big_keys" {
                db.batch_execute("CREATE INDEX existing_big_keys_order ON business.big_keys ((id::text COLLATE \"C\"),(lower(n::text)))")?;
            }
            let expected: std::collections::BTreeMap<String, i64> = db
                .query(
                    &format!("SELECT id::text,n::bigint FROM business.{table}"),
                    &[],
                )?
                .iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect();
            let dataset = app.execute(
                alice,
                "create_dataset",
                json!({"project":project,"name":table,"postgis_table":{"schema":"business","table":table,"id_column":"id","geometry_column":"geom"}}),
            )?["dataset"]
                .clone();
            if table == "big_keys" || table == "varchar_c_keys" {
                let own: String = db.query_one("SELECT postgis_source::jsonb->>'order_index' FROM gl_datasets WHERE project=$1 AND id=$2", &[&project.as_str().ok_or("project")?,&dataset.as_str().ok_or("dataset")?])?.get(0);
                assert!(
                    own.is_empty(),
                    "{table} already has a usable identity index"
                );
            }
            let mut actual = std::collections::BTreeMap::new();
            let mut after = String::new();
            loop {
                let page = app.execute(
                    alice,
                    "features",
                    json!({"project":project,"dataset":dataset,"after":after,"limit":1000}),
                )?;
                for feature in page["features"].as_array().ok_or("features")? {
                    let id = feature["id"].as_str().ok_or("feature id")?.to_owned();
                    let n = feature["properties"]["n"].as_i64().ok_or("property n")?;
                    assert!(actual.insert(id, n).is_none(), "duplicate in {table}");
                }
                let Some(next) = page["next_after"].as_str() else {
                    break;
                };
                assert_ne!(after, next, "stalled page in {table}");
                after = next.to_owned();
            }
            assert_eq!(actual.len(), 2501, "{table}");
            assert_eq!(actual, expected, "{table}");
            assert_eq!(
                db.query_one(&format!("SELECT count(*) FROM business.{table}"), &[])?
                    .get::<_, i64>(0),
                2501
            );
        }
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn registration_rejects_invalid_source_geometries_atomically() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    support::reset_database(&mut db)?;
    let result = (|| -> TestResult {
        db.batch_execute("CREATE SCHEMA business")?;
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        let password = uuid::Uuid::new_v4().to_string();
        app.bootstrap_admin("alice", &password)?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":password}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let project = app.execute(
            alice,
            "create_project",
            json!({"name":"geometry validation"}),
        )?["project"]
            .as_str()
            .ok_or("project")?
            .to_owned();
        for (index, (kind, invalid, valid)) in [
            (
                "Point",
                "ST_GeomFromText('POINT(NaN 2)',4326)",
                "POINT(1 2)",
            ),
            (
                "Point",
                "ST_SetSRID(ST_MakePoint('Infinity'::float8,2),4326)",
                "POINT(1 2)",
            ),
            (
                "PointZ",
                "ST_GeomFromText('POINT Z(1 2 NaN)',4326)",
                "POINT Z(1 2 3)",
            ),
            ("Point", "ST_GeomFromText('POINT EMPTY',4326)", "POINT(1 2)"),
            (
                "LineString",
                "ST_GeomFromText('LINESTRING EMPTY',4326)",
                "LINESTRING(1 2,3 4)",
            ),
            (
                "Polygon",
                "ST_GeomFromText('POLYGON EMPTY',4326)",
                "POLYGON((0 0,1 1,1 0,0 0))",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("geometry_case_{index}");
            let table = format!("business.{name}");
            db.batch_execute(&format!("CREATE TABLE {table}(id bigint PRIMARY KEY,label text,geom geometry({kind},4326)); INSERT INTO {table} SELECT i,'original',ST_GeomFromText('{valid}',4326) FROM generate_series(1,33) i; INSERT INTO {table} VALUES(34,'invalid',{invalid})"))?;
            let before = registration_state(&mut db, &project, &table)?;
            let register = || {
                app.execute(alice,"create_dataset",json!({"project":project,"name":name,"postgis_table":{"schema":"business","table":name,"id_column":"id","geometry_column":"geom"}}))
            };
            assert_eq!(
                register().err().map(|error| error.status),
                Some(400),
                "{invalid}"
            );
            assert_eq!(
                registration_state(&mut db, &project, &table)?,
                before,
                "{invalid} changed enrollment state"
            );
            assert_eq!(
                db.execute(&format!("DELETE FROM {table} WHERE id=34"), &[])?,
                1
            );
            register()?;
        }
        db.batch_execute("CREATE TABLE business.nullable_multi(id bigint PRIMARY KEY,label text,geom geometry(MultiPoint,4326)); INSERT INTO business.nullable_multi VALUES(1,'null',NULL),(2,'empty',ST_GeomFromText('MULTIPOINT EMPTY',4326))")?;
        let dataset=app.execute(alice,"create_dataset",json!({"project":project,"name":"nullable multi","postgis_table":{"schema":"business","table":"nullable_multi","id_column":"id","geometry_column":"geom"}}))?["dataset"].clone();
        let page = app.execute(
            alice,
            "features",
            json!({"project":project,"dataset":dataset}),
        )?;
        assert!(page["features"][0]["geometry"].is_null());
        assert_eq!(
            page["features"][1]["geometry"],
            json!({"type":"MultiPoint","coordinates":[]})
        );
        assert_eq!(
            db.query_one("SELECT count(*) FROM gl_history", &[])?
                .get::<_, i64>(0),
            0
        );
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn incremental_fixture_seeding_keeps_source_readable_and_other_drift() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    support::reset_database(&mut db)?;
    let result = (|| -> TestResult {
        db.batch_execute("CREATE SCHEMA business; CREATE TABLE business.source(id text PRIMARY KEY,attributes jsonb,geom geometry(Point,4326)); CREATE TABLE business.other(LIKE business.source INCLUDING ALL)")?;
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        let password = uuid::Uuid::new_v4().to_string();
        app.bootstrap_admin("alice", &password)?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":password}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let project = app.execute(
            alice,
            "create_project",
            json!({"name":"incremental fixture"}),
        )?["project"]
            .as_str()
            .ok_or("project")?
            .to_owned();
        let mut dataset = String::new();
        for table in ["source", "other"] {
            let created=app.execute(alice,"create_dataset",json!({"project":project,"name":table,"postgis_table":{"schema":"business","table":table,"id_column":"id","geometry_column":"geom"}}))?;
            if table == "source" {
                dataset = created["dataset"].as_str().ok_or("dataset")?.to_owned();
            }
        }
        db.batch_execute("BEGIN; SELECT set_config('geoledger.publication_table','business.other'::regclass::oid::text,true); INSERT INTO business.other VALUES('outside','{}',NULL); COMMIT")?;
        let generated=std::process::Command::new("python3").current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).args(["-c","import sys; sys.path.insert(0,'scripts'); from benchmark_incremental import postgis_seed_sql; print(postgis_seed_sql(sys.argv[1],sys.argv[2],1,4326))",&project,&dataset]).output()?;
        assert!(generated.status.success(), "SQL generator failed");
        let feature = json!({"type":"Feature","id":"000000000001","properties":{"attributes":{"n":18446744073709551615_u64}},"geometry":{"type":"Point","coordinates":[1,2]}});
        let payload = feature.to_string().replace('\'', "''");
        // Supply one payload without requiring server-side access to the test host.
        let sql = String::from_utf8(generated.stdout)?
            .lines()
            .map(|line| {
                if line.starts_with("COPY fixture_payload") {
                    format!("INSERT INTO fixture_payload VALUES('{payload}');")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        db.batch_execute(&sql)?;
        assert_eq!(db.query_one("SELECT count(*) FROM gl_source_changes WHERE schema_name='business' AND table_name='source'",&[])?.get::<_,i64>(0),0);
        assert_eq!(db.query_one("SELECT count(*) FROM gl_source_changes WHERE schema_name='business' AND table_name='other'",&[])?.get::<_,i64>(0),1);
        let page = app.execute(
            alice,
            "features",
            json!({"project":project,"dataset":dataset}),
        )?;
        assert_eq!(page["features"][0]["properties"], feature["properties"]);
        assert_eq!(page["features"][0]["id"], feature["id"]);
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn registration_rejects_invalid_feature_ids_atomically() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    support::reset_database(&mut db)?;
    let result = (|| -> TestResult {
        db.batch_execute("CREATE SCHEMA business; CREATE TABLE business.text_keys(id text COLLATE \"POSIX\" PRIMARY KEY,label text NOT NULL,geom geometry(Point,4326))")?;
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        app.bootstrap_admin("alice", "initial-test-password")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":"initial-test-password"}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let project = app.execute(
            alice,
            "create_project",
            json!({"name":"identity validation"}),
        )?["project"]
            .as_str()
            .ok_or("project")?
            .to_owned();
        let register = || {
            app.execute(
            alice,
            "create_dataset",
            json!({"project":project,"name":"keys","postgis_table":{"schema":"business","table":"text_keys","id_column":"id","geometry_column":"geom"}}),
        )
        };
        let before = registration_state(&mut db, &project, "business.text_keys")?;
        for id in [
            String::new(),
            "é".repeat(128) + "a",
            "line\nkey".into(),
            "key-\u{0001}".into(),
            "key-\u{001f}".into(),
            "key-\u{007f}".into(),
            "key-\u{0085}".into(),
            "key-\u{009f}".into(),
        ] {
            db.execute(
                "INSERT INTO business.text_keys(id,label) VALUES($1,'original')",
                &[&id],
            )?;
            assert_eq!(register().err().map(|e| e.status), Some(400), "{id:?}");
            assert_eq!(
                registration_state(&mut db, &project, "business.text_keys")?,
                before,
                "rejected identifier {id:?} changed enrollment state"
            );
            assert_eq!(db.execute("DELETE FROM business.text_keys", &[])?, 1);
        }
        let valid_ids = ["é".repeat(128), "key- ~\u{00a0}".into()];
        for id in &valid_ids {
            db.execute(
                "INSERT INTO business.text_keys(id,label) VALUES($1,'original')",
                &[id],
            )?;
        }
        let dataset = register()?["dataset"].clone();
        assert_eq!(
            db.query_one("SELECT count(*) FROM gl_history", &[])?
                .get::<_, i64>(0),
            0,
            "enrollment must not copy feature bodies into history"
        );
        let workspace =
            app.execute(alice, "create_workspace", json!({"project":project}))?["workspace"]
                .clone();
        let mut edits = Vec::new();
        for id in &valid_ids {
            let mut feature = app.execute(
                alice,
                "features",
                json!({"project":project,"dataset":dataset,"feature_id":id}),
            )?;
            assert_eq!(feature["id"], *id);
            feature["properties"]["label"] = json!("changed");
            edits.push(json!({"dataset":dataset,"feature_id":id,"feature":{"type":"Feature","id":id,"properties":feature["properties"],"geometry":feature["geometry"]}}));
        }
        app.execute(alice, "save", json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"edits":edits}))?;
        app.execute(alice, "publish", json!({"project":project,"workspace":workspace,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"valid identities"}))?;
        assert_eq!(
            db.query_one(
                "SELECT count(*) FROM business.text_keys WHERE label='changed'",
                &[]
            )?
            .get::<_, i64>(0),
            2
        );
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn registration_rejects_invalid_property_columns_atomically() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    support::reset_database(&mut db)?;
    let result = (|| -> TestResult {
        let columns = (0..257)
            .map(|i| format!("n{i:03} integer"))
            .collect::<Vec<_>>()
            .join(",");
        db.batch_execute(&format!("CREATE SCHEMA business; CREATE TABLE business.wide(id bigint PRIMARY KEY,{columns},geom geometry(Point,4326)); INSERT INTO business.wide(id) VALUES(1); CREATE TABLE business.bad_name(id bigint PRIMARY KEY,\"bad\nproperty\" text,geom geometry(Point,4326)); INSERT INTO business.bad_name(id) VALUES(1)"))?;
        for (table, column) in [
            ("reserved_raw", "$serde_json::private::RawValue"),
            ("reserved_number", "$serde_json::private::Number"),
        ] {
            db.batch_execute(&format!(
                "CREATE TABLE business.{table}(id bigint PRIMARY KEY,\"{column}\" text,geom geometry(Point,4326)); INSERT INTO business.{table}(id) VALUES(1)"
            ))?;
        }
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        app.bootstrap_admin("alice", "initial-test-password")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":"initial-test-password"}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let project = app.execute(
            alice,
            "create_project",
            json!({"name":"property validation"}),
        )?["project"]
            .as_str()
            .ok_or("project")?
            .to_owned();
        let register = |table: &str| {
            app.execute(
            alice,
            "create_dataset",
            json!({"project":project,"name":table,"postgis_table":{"schema":"business","table":table,"id_column":"id","geometry_column":"geom"}}),
        )
        };
        for table in ["wide", "bad_name", "reserved_raw", "reserved_number"] {
            let relation = format!("business.{table}");
            let before = registration_state(&mut db, &project, &relation)?;
            assert_eq!(
                register(table).err().map(|e| e.status),
                Some(400),
                "{table}"
            );
            assert_eq!(
                registration_state(&mut db, &project, &relation)?,
                before,
                "{table}"
            );
            assert_eq!(db.execute(&format!("UPDATE {relation} SET id=2"), &[])?, 1);
        }
        db.batch_execute("ALTER TABLE business.wide DROP COLUMN n256")?;
        let dataset = register("wide")?["dataset"].clone();
        assert_eq!(
            db.query_one("SELECT count(*) FROM gl_history", &[])?
                .get::<_, i64>(0),
            0
        );
        let mut feature = app.execute(
            alice,
            "features",
            json!({"project":project,"dataset":dataset,"feature_id":"2"}),
        )?;
        assert_eq!(
            feature["properties"].as_object().ok_or("properties")?.len(),
            256
        );
        feature["properties"]["n000"] = json!(42);
        let feature = json!({"type":"Feature","id":"2","properties":feature["properties"],"geometry":feature["geometry"]});
        let workspace =
            app.execute(alice, "create_workspace", json!({"project":project}))?["workspace"]
                .clone();
        app.execute(alice, "save", json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"edits":[{"dataset":dataset,"feature_id":"2","feature":feature}]}))?;
        app.execute(alice, "publish", json!({"project":project,"workspace":workspace,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"valid properties"}))?;
        assert_eq!(
            db.query_one("SELECT n000 FROM business.wide WHERE id=2", &[])?
                .get::<_, i32>(0),
            42
        );
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

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
    support::reset_database(&mut admin)?;
    let result = (|| -> TestResult {
        let (dsn, mut db) = support::connect_isolated(&dsn, &name)?;
        db.batch_execute("CREATE EXTENSION IF NOT EXISTS postgis; CREATE SCHEMA business; CREATE TABLE business.points(id bigint PRIMARY KEY,label text NOT NULL CHECK(label <> 'invalid'), n numeric(20,0), geom geometry(PointZ,3857)); INSERT INTO business.points VALUES(1,'original',18446744073709551615,ST_SetSRID(ST_MakePoint(500000,3500000,3),3857))")?;
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        app.bootstrap_admin("alice", "initial-test-password")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":"initial-test-password"}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let p = app.execute(alice, "create_project", json!({"name":"bound"}))?["project"].clone();
        let run = |op: &str, mut value: Value| {
            value["project"] = p.clone();
            app.execute(alice, op, value)
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
        assert_eq!(
            denied.body["error"]["message"],
            "registering a business table requires a platform administrator"
        );
        let created = run(
            "create_dataset",
            json!({"name":"existing","postgis_table":source}),
        )?;
        let d = created["dataset"].clone();
        assert_eq!(created["geometry_type"], "point");
        assert_eq!(created["coordinate_dimension"], 3);
        assert_eq!(created["crs"], "EPSG:3857");
        assert!(
            db.query_one(
                "SELECT storage_table FROM gl_datasets WHERE project=$1 AND id=$2",
                &[&p.as_str().ok_or("project")?, &d.as_str().ok_or("dataset")?]
            )?
            .get::<_, Option<String>>(0)
            .is_none()
        );
        assert_eq!(
            db.query_one(
                "SELECT count(*) FROM pg_class WHERE relkind='r' AND left(relname,8)='gl_data_'",
                &[]
            )?
            .get::<_, i64>(0),
            0
        );
        assert_eq!(
            run("history", json!({}))?["commits"][0]["message"],
            "Register table"
        );
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
            imported.execute(alice, "list_datasets", json!({"project":p}))?[0]["postgis_table"]
                .is_null()
        );
        assert_eq!(
            imported.execute(alice, "features", json!({"project":p,"dataset":d}))?["features"],
            features["features"]
        );
        imported.execute(
            alice,
            "delete_dataset",
            json!({"project":p,"dataset":d,"confirm_name":"existing"}),
        )?;
        let imported_db = rusqlite::Connection::open(temp.path().join("import.db"))?;
        for table in ["gl_datasets", "gl_commits", "gl_workspaces"] {
            assert_eq!(
                imported_db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))?,
                0,
                "{table} should not retain registration state"
            );
        }
        let feature = |id: &str, label: &str| json!({"type":"Feature","id":id,"properties":{"label":label,"n":18446744073709551615u64},"geometry":{"type":"Point","coordinates":[500010,3500020,30]}});
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
        let published_workspace = w.clone();
        let published_features = run("features", json!({"dataset":d,"workspace":w}))?;
        assert_eq!(published_features["revision"], 2);
        assert_eq!(
            published_features["features"],
            json!([feature("1", "edited"), feature("2", "inserted")])
        );
        let published_summary = run("workspace_summary", json!({"workspace":w}))?;
        assert_eq!(
            published_summary["total"],
            json!({"added":1,"deleted":0,"modified":1})
        );
        let native = db.query_one(
            "SELECT ST_SRID(geom),ST_X(geom),ST_Y(geom),ST_Z(geom) FROM business.points WHERE id=1",
            &[],
        )?;
        assert_eq!(native.get::<_, i32>(0), 3857);
        assert_eq!(
            (
                native.get::<_, f64>(1),
                native.get::<_, f64>(2),
                native.get::<_, f64>(3)
            ),
            (500010.0, 3500020.0, 30.0)
        );
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
        assert_eq!(
            run(
                "features",
                json!({"dataset":d,"workspace":published_workspace})
            )?,
            published_features
        );
        assert_eq!(
            run(
                "workspace_summary",
                json!({"workspace":published_workspace})
            )?,
            published_summary
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
        // An out-of-band write cannot be reconciled without a retained old value.
        db.batch_execute("BEGIN; ALTER TABLE business.points DISABLE TRIGGER USER; UPDATE business.points SET label='outside' WHERE id=2; ALTER TABLE business.points ENABLE TRIGGER gl_versioned_write; ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_track; ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_truncate; COMMIT")?;
        rejected("1", "candidate", 409)?;
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.points'::regclass::oid::text,false); UPDATE business.points SET label='inserted' WHERE id=2; RESET geoledger.publication_table")?;
        rejected("1", "candidate", 409)?;
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
        // SRID 0 typmods permit per-row SRIDs; enrollment must not silently mix them.
        db.batch_execute("CREATE TABLE business.local_points(id bigint PRIMARY KEY,geom geometry(Point,0)); INSERT INTO business.local_points VALUES(1,ST_SetSRID(ST_MakePoint(500000,3500000),3857))")?;
        let local_request = json!({"name":"local","postgis_table":{"schema":"business","table":"local_points","id_column":"id","geometry_column":"geom"}});
        assert_eq!(
            run("create_dataset", local_request.clone())
                .err()
                .ok_or("mixed SRID enrolled")?
                .status,
            400
        );
        db.batch_execute("UPDATE business.local_points SET geom=ST_SetSRID(geom,0)")?;
        let local = run("create_dataset", local_request)?;
        assert_eq!(local["crs"], "POSTGIS:0");
        let local_id = local["dataset"].clone();
        let w = run("create_workspace", json!({}))?["workspace"].clone();
        run(
            "save",
            json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":local_id,"feature_id":"1","feature":{"type":"Feature","id":"1","properties":{},"geometry":{"type":"Point","coordinates":[500001,3500001]}}}]}),
        )?;
        db.batch_execute("SELECT set_config('geoledger.publication_table','business.local_points'::regclass::oid::text,false); UPDATE business.local_points SET geom=ST_SetSRID(geom,3857); RESET geoledger.publication_table")?;
        assert_eq!(run("publish",json!({"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"reject SRID drift"})).err().ok_or("SRID drift overwritten")?.status,409);
        Ok(())
    })();
    support::reset_database(&mut admin)?;
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
fn same_database_source_replacement_is_rejected() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    support::reset_database(&mut db)?;
    let result = (|| -> TestResult {
        db.batch_execute("CREATE SCHEMA business; CREATE TABLE business.points(id text COLLATE \"C\" PRIMARY KEY,geom geometry(Point,4326)); INSERT INTO business.points VALUES('one',ST_SetSRID(ST_MakePoint(1,2),4326)); CREATE UNLOGGED TABLE business.volatile(id text PRIMARY KEY,geom geometry(Point,4326))")?;
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        app.bootstrap_admin("alice", "initial-test-password")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":"initial-test-password"}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let project =
            app.execute(alice, "create_project", json!({"name":"source binding"}))?["project"]
                .clone();
        let register = |table: &str| {
            app.execute(alice,"create_dataset",json!({"project":project,"name":table,"postgis_table":{"schema":"business","table":table,"id_column":"id","geometry_column":"geom"}}))
        };
        assert_eq!(
            register("volatile")
                .err()
                .ok_or("unlogged source accepted")?
                .status,
            400
        );
        let dataset = register("points")?["dataset"].clone();
        let binding: String = db.query_one("SELECT binding FROM gl_source_state WHERE schema_name='business' AND table_name='points'", &[])?.get(0);
        db.batch_execute("COMMENT ON TRIGGER gl_versioned_write ON business.points IS NULL")?;
        assert_eq!(
            app.execute(
                alice,
                "features",
                json!({"project":project,"dataset":dataset})
            )
            .err()
            .ok_or("unbound trigger accepted")?
            .status,
            409
        );
        assert_eq!(
            app.execute(
                alice,
                "delete_dataset",
                json!({"project":project,"dataset":dataset,"confirm_name":"points"})
            )
            .err()
            .ok_or("incomplete trigger cleanup accepted")?
            .status,
            409
        );
        db.batch_execute(&format!(
            "COMMENT ON TRIGGER gl_versioned_write ON business.points IS '{binding}'"
        ))?;
        db.batch_execute("ALTER TABLE business.points RENAME TO old_points; CREATE TABLE business.points (LIKE business.old_points INCLUDING ALL); INSERT INTO business.points SELECT * FROM business.old_points; CREATE TRIGGER gl_versioned_write BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON business.points FOR EACH STATEMENT EXECUTE FUNCTION gl_source_guard(); CREATE TRIGGER gl_versioned_track AFTER INSERT OR UPDATE OR DELETE ON business.points FOR EACH ROW EXECUTE FUNCTION gl_source_track('id'); CREATE TRIGGER gl_versioned_truncate AFTER TRUNCATE ON business.points FOR EACH STATEMENT EXECUTE FUNCTION gl_source_track(); ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_track; ALTER TABLE business.points ENABLE ALWAYS TRIGGER gl_versioned_truncate; DROP TABLE business.old_points")?;
        for trigger in [
            "gl_versioned_write",
            "gl_versioned_track",
            "gl_versioned_truncate",
        ] {
            db.batch_execute(&format!(
                "COMMENT ON TRIGGER {trigger} ON business.points IS '{binding}'"
            ))?;
        }
        assert_eq!(
            app.execute(
                alice,
                "features",
                json!({"project":project,"dataset":dataset})
            )
            .err()
            .ok_or("replaced table accepted")?
            .status,
            409
        );
        Ok(())
    })();
    support::reset_database(&mut db)?;
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
    support::reset_database(&mut admin)?;
    let result = (|| -> TestResult {
        let (dsn, mut db) = support::connect_isolated(&dsn, &name)?;
        db.batch_execute("CREATE EXTENSION IF NOT EXISTS postgis; CREATE SCHEMA business; CREATE TABLE business.zz_parent(id bigint PRIMARY KEY,label text,geom geometry(Point,3857)); CREATE TABLE business.aa_child(id bigint PRIMARY KEY,parent_id bigint REFERENCES business.zz_parent(id) DEFERRABLE INITIALLY DEFERRED,geom geometry(Point,3857)); CREATE TABLE business.deferred_key(id bigint PRIMARY KEY DEFERRABLE INITIALLY DEFERRED,label text,geom geometry(Point,3857))")?;
        let app = Application::new(Storage::Postgis(dsn));
        app.migrate()?;
        app.bootstrap_admin("alice", "initial-test-password")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"alice","password":"initial-test-password"}),
        )?;
        let alice = login["user"]["id"].as_str().ok_or("admin id")?;
        let p =
            app.execute(alice, "create_project", json!({"name":"deferred"}))?["project"].clone();
        let run = |op: &str, mut value: Value| {
            value["project"] = p.clone();
            app.execute(alice, op, value)
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
        let owned_order: String = db.query_one("SELECT postgis_source::jsonb->>'order_index' FROM gl_datasets WHERE project=$1 AND id=$2", &[&p.as_str().ok_or("project")?,&child.as_str().ok_or("dataset")?])?.get(0);
        assert!(!owned_order.is_empty());
        db.batch_execute("ALTER TABLE business.aa_child RENAME TO renamed_child; CREATE TABLE business.aa_child(id bigint PRIMARY KEY,geom geometry(Point,3857)); CREATE FUNCTION business.unrelated_guard() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$; CREATE TRIGGER gl_versioned_write BEFORE INSERT ON business.aa_child FOR EACH STATEMENT EXECUTE FUNCTION business.unrelated_guard()")?;
        run(
            "delete_dataset",
            json!({"dataset":child,"confirm_name":"aa_child"}),
        )?;
        assert_eq!(db.query_one("SELECT count(*) FROM pg_trigger WHERE tgrelid='business.aa_child'::regclass AND tgname='gl_versioned_write'",&[])?.get::<_,i64>(0),1);
        assert_eq!(db.query_one("SELECT count(*) FROM pg_trigger WHERE tgrelid='business.renamed_child'::regclass AND tgname LIKE 'gl_versioned_%'",&[])?.get::<_,i64>(0),0);
        assert_eq!(
            db.query_one(
                "SELECT count(*) FROM pg_class WHERE relname=$1",
                &[&owned_order]
            )?
            .get::<_, i64>(0),
            0
        );
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
    support::reset_database(&mut admin)?;
    if let Err(error) = &result {
        let mut source = error.source();
        while let Some(error) = source {
            eprintln!("cause: {error:?}");
            source = error.source();
        }
    }
    result
}
