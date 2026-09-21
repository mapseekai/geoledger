#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Opt-in integration tests; fixtures remain inside the disposable test database.
use postgres::{Client, NoTls};
use serde_json::{Value, json};
use spatial_version::{Application, Command};
use spatial_version_postgis::PostgisProvider;
use std::sync::Arc;

struct Fixture {
    app: Application,
    db: Client,
    schema: String,
    _directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dsn = std::env::var("SV_TEST_DATABASE_URL").expect("set disposable test database URL");
        let mut db = Client::connect(&dsn, NoTls).unwrap();
        let database: String = db
            .query_one("SELECT current_database()", &[])
            .unwrap()
            .get(0);
        assert_eq!(
            database, "spatial_version_test",
            "refusing a non-test database"
        );
        db.batch_execute("CREATE EXTENSION IF NOT EXISTS postgis")
            .unwrap();
        let schema = format!("svtest_{}", uuid::Uuid::new_v4().simple());
        db.batch_execute(&format!(r#"
            CREATE SCHEMA "{schema}";
            CREATE TABLE "{schema}".roads (
                id bigint PRIMARY KEY, name text UNIQUE, width numeric,
                geom geometry(LineStringZ, 4326)
            );
            INSERT INTO "{schema}".roads VALUES
                (1,'Original',10.12345678901234567890,ST_GeomFromEWKT('SRID=4326;LINESTRING Z(0 0 1,1 1 2)')),
                (2,'Second',20,NULL);
        "#)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let app =
            Application::new(directory.path()).with_provider(Arc::new(PostgisProvider::new(dsn)));
        let f = Self {
            app,
            db,
            schema,
            _directory: directory,
        };
        f.run(json!({"op":"init","author":"integration"}));
        f.run(json!({"op":"import","dataset":"roads","schema":f.schema,"table":"roads","author":"integration"}));
        f
    }
    fn run(&self, value: Value) -> Value {
        let command: Command = serde_json::from_value(value).unwrap();
        self.app.execute(command).unwrap()
    }
    fn error(&self, value: Value) {
        let command: Command = serde_json::from_value(value).unwrap();
        assert!(self.app.execute(command).is_err());
    }
    fn sql(&mut self, text: &str) {
        self.db
            .batch_execute(&text.replace("$roads", &format!("\"{}\".roads", self.schema)))
            .unwrap();
    }
    fn status(&self) -> Value {
        self.run(json!({"op":"status"}))
    }
    fn head(&self) -> String {
        self.run(json!({"op":"show"}))["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn commit(&self, message: &str) -> String {
        self.run(json!({"op":"commit","author":"integration","message":message}))["commit"]
            .as_str()
            .unwrap()
            .into()
    }
    fn name(&mut self) -> String {
        self.db
            .query_one(
                &format!("SELECT name FROM \"{}\".roads WHERE id=1", self.schema),
                &[],
            )
            .unwrap()
            .get(0)
    }
}

#[test]
#[ignore = "requires disposable SV_TEST_DATABASE_URL"]
fn import_tracks_edits_and_exact_numeric_geometry_encoding() {
    let mut f = Fixture::new();
    let record = f.run(json!({"op":"show","dataset":"roads","key":"1"}))["record"].clone();
    assert_eq!(
        record["fields"]["width"]["value"],
        "10.12345678901234567890"
    );
    let expected: String =
        f.db.query_one(
            &format!(
                "SELECT encode(ST_AsEWKB(geom,'XDR'),'hex') FROM \"{}\".roads WHERE id=1",
                f.schema
            ),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(record["fields"]["geom"]["value"], expected);
    f.sql("UPDATE $roads SET name=name WHERE id=1");
    assert!(f.status()["diff"]["changes"].as_array().unwrap().is_empty());
    f.sql("UPDATE $roads SET id=3,name='Moved' WHERE id=1");
    assert_eq!(f.status()["diff"]["changes"].as_array().unwrap().len(), 2);
    f.commit("rename primary key");
    assert!(f.status()["diff"]["changes"].as_array().unwrap().is_empty());
    assert!(f.run(json!({"op":"show","dataset":"roads","key":"1"}))["record"].is_null());
    assert_eq!(
        f.run(json!({"op":"show","dataset":"roads","key":"3"}))["record"]["fields"]["name"]["value"],
        "Moved"
    );
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable SV_TEST_DATABASE_URL"]
fn independent_fields_merge_and_same_field_conflicts_are_resolved() {
    let mut f = Fixture::new();
    f.run(json!({"op":"branch","name":"draft"}));
    f.run(json!({"op":"switch","branch":"draft"}));
    f.sql("UPDATE $roads SET width=42 WHERE id=1");
    f.commit("draft width");
    f.run(json!({"op":"switch","branch":"main"}));
    f.sql("UPDATE $roads SET name='Main' WHERE id=1");
    f.commit("main name");
    f.run(json!({"op":"merge","source":"draft","author":"integration"}));
    assert_eq!(f.name(), "Main");
    assert_eq!(
        f.run(json!({"op":"show"}))["commit"]["parents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        f.run(json!({"op":"show","dataset":"roads","key":"1"}))["record"]["fields"]["width"]["value"],
        "42"
    );
    f.run(json!({"op":"branch","name":"conflict"}));
    f.run(json!({"op":"switch","branch":"conflict"}));
    f.sql("UPDATE $roads SET name='Theirs' WHERE id=1");
    f.commit("theirs");
    f.run(json!({"op":"switch","branch":"main"}));
    f.sql("UPDATE $roads SET name='Ours' WHERE id=1");
    let ours = f.commit("ours");
    f.run(json!({"op":"merge","source":"conflict","author":"integration"}));
    assert_eq!(f.head(), ours);
    assert_eq!(f.name(), "Ours");
    assert_eq!(
        f.run(json!({"op":"conflicts"}))["conflicts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.error(json!({"op":"merge_continue"}));
    f.run(json!({"op":"resolve","dataset":"roads","key":"1","choice":"theirs"}));
    assert_eq!(f.name(), "Ours");
    f.run(json!({"op":"merge_continue"}));
    assert_eq!(f.name(), "Theirs");
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable SV_TEST_DATABASE_URL"]
fn restores_test_fixture_and_reverts_by_creating_a_new_commit() {
    // Fixture refuses any database whose name is not spatial_version_test.
    let mut f = Fixture::new();
    let initial = f.head();
    f.run(json!({"op":"branch","name":"draft"}));
    f.sql("UPDATE $roads SET name='Dirty' WHERE id=1");
    f.error(json!({"op":"switch","branch":"draft"}));
    f.error(json!({"op":"restore","discard":false}));
    f.run(json!({"op":"restore","discard":true}));
    assert_eq!(f.name(), "Original");
    f.sql("UPDATE $roads SET name='Committed' WHERE id=1");
    let edited = f.commit("edit name");
    f.run(json!({"op":"revert","target":edited,"author":"integration"}));
    assert_eq!(f.name(), "Original");
    assert_ne!(f.head(), initial);
    f.run(json!({"op":"reset","target":edited,"hard":true}));
    assert_eq!(f.head(), edited);
    assert_eq!(f.name(), "Committed");
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable SV_TEST_DATABASE_URL"]
fn geometry_conflicts_abort_without_changing_working_copy() {
    let mut f = Fixture::new();
    f.run(json!({"op":"branch","name":"draft"}));
    f.run(json!({"op":"switch","branch":"draft"}));
    f.sql(
        "UPDATE $roads SET geom=ST_GeomFromEWKT('SRID=4326;LINESTRING Z(0 0 1,2 2 2)') WHERE id=1",
    );
    f.commit("theirs geometry");
    f.run(json!({"op":"switch","branch":"main"}));
    f.sql(
        "UPDATE $roads SET geom=ST_GeomFromEWKT('SRID=4326;LINESTRING Z(0 0 1,3 3 3)') WHERE id=1",
    );
    f.commit("ours geometry");
    let before = f.run(json!({"op":"show","dataset":"roads","key":"1"}));
    f.run(json!({"op":"merge","source":"draft","author":"integration"}));
    assert_eq!(
        f.run(json!({"op":"conflicts"}))["conflicts"][0]["fields"][0],
        "geom"
    );
    f.run(json!({"op":"merge_abort"}));
    assert_eq!(
        f.run(json!({"op":"show","dataset":"roads","key":"1"})),
        before
    );
    assert!(f.status()["diff"]["changes"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "requires disposable SV_TEST_DATABASE_URL"]
fn constraint_failure_keeps_head_and_database_unchanged() {
    let mut f = Fixture::new();
    f.run(json!({"op":"branch","name":"draft"}));
    f.run(json!({"op":"switch","branch":"draft"}));
    f.sql("UPDATE $roads SET name='Collision' WHERE id=1");
    f.commit("draft row one");
    f.run(json!({"op":"switch","branch":"main"}));
    f.sql("UPDATE $roads SET name='Collision' WHERE id=2");
    let head = f.commit("main row two");
    f.error(json!({"op":"merge","source":"draft","author":"integration"}));
    assert_eq!(f.head(), head);
    assert_eq!(f.name(), "Original");
    assert!(f.status()["diff"]["changes"].as_array().unwrap().is_empty());
    assert_eq!(f.run(json!({"op":"recover"}))["recovered"], false);
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable SV_TEST_DATABASE_URL"]
fn schema_drift_is_rejected() {
    let mut f = Fixture::new();
    f.sql("ALTER TABLE $roads ADD COLUMN extra text");
    f.error(json!({"op":"commit","author":"integration","message":"reject DDL drift"}));
}
