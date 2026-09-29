#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Opt-in integration tests; fixtures remain inside the disposable test database.
use geoledger::{Application, Command};
use geoledger_postgis::PostgisProvider;
use postgres::{Client, NoTls};
use serde_json::{Value, json};
use std::sync::Arc;

struct Fixture {
    app: Application,
    db: Client,
    schema: String,
    _directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dsn = std::env::var("GL_TEST_DATABASE_URL").expect("set disposable test database URL");
        let mut db = Client::connect(&dsn, NoTls).unwrap();
        let database: String = db
            .query_one("SELECT current_database()", &[])
            .unwrap()
            .get(0);
        assert_eq!(database, "geoledger_test", "refusing a non-test database");
        db.batch_execute("CREATE EXTENSION IF NOT EXISTS postgis")
            .unwrap();
        let schema = format!("gltest_{}", uuid::Uuid::new_v4().simple());
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
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn regression_nondefault_collations_are_rejected_before_import() {
    let mut f = Fixture::new();
    let head = f.head();
    f.db.batch_execute(&format!(
        "CREATE COLLATION {}.ci (provider=icu, locale='und-u-ks-level2', deterministic=false)",
        f.schema
    ))
    .unwrap();
    for (name, collation) in [
        ("ordered", "\"C\"".into()),
        ("folded", format!("{}.ci", f.schema)),
    ] {
        f.db.batch_execute(&format!(
            "CREATE TABLE {schema}.{name}(id integer PRIMARY KEY, value text COLLATE {collation});
             INSERT INTO {schema}.{name} VALUES(1,'One')",
            schema = f.schema
        ))
        .unwrap();
        let result = f.app.execute(
            serde_json::from_value(json!({
                "op":"import","dataset":name,"schema":f.schema,"table":name
            }))
            .unwrap(),
        );
        assert!(
            matches!(result, Err(geoledger::core::Error::Unsupported(_))),
            "{result:?}"
        );
        let count: i64 =
            f.db.query_one(
                "SELECT count(*) FROM _geoledger.tracked WHERE table_oid=$1::text::regclass",
                &[&format!("{}.{name}", f.schema)],
            )
            .unwrap()
            .get(0);
        assert_eq!(count, 0);
        assert_eq!(f.head(), head);
    }
    assert!(
        f.db.query_one(&format!("SELECT value='one' FROM {}.folded", f.schema), &[])
            .unwrap()
            .get::<_, bool>(0)
    );
    assert_eq!(f.status()["clean"], true);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn regression_collation_drift_blocks_destructive_commands() {
    let mut f = Fixture::new();
    f.sql("ALTER TABLE $roads ADD COLUMN note text; UPDATE $roads SET note='One'");
    let head = f.commit("add note");
    f.db.batch_execute(&format!(
        "CREATE COLLATION {}.ci (provider=icu, locale='und-u-ks-level2', deterministic=false)",
        f.schema
    ))
    .unwrap();
    f.sql(&format!(
        "ALTER TABLE $roads ALTER COLUMN note TYPE text COLLATE {}.ci",
        f.schema
    ));
    for command in [
        json!({"op":"status"}),
        json!({"op":"commit","message":"unsupported collation"}),
        json!({"op":"restore","discard":true}),
        json!({"op":"reset","target":head,"hard":true}),
        json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"note","discard":true}}),
    ] {
        let result = f.app.execute(serde_json::from_value(command).unwrap());
        assert!(
            matches!(result, Err(geoledger::core::Error::Unsupported(_))),
            "{result:?}"
        );
        assert_eq!(f.head(), head);
        assert!(
            f.db.query_one(
                &format!("SELECT bool_and(note='one') FROM {}.roads", f.schema),
                &[]
            )
            .unwrap()
            .get::<_, bool>(0)
        );
    }
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn regression_unique_value_swaps_restore_across_batches() {
    for rows in [2, 1002] {
        let mut f = Fixture::new();
        f.sql(&format!(
            "UPDATE $roads SET name=id::text;
            INSERT INTO $roads(id,name) SELECT i,i::text FROM generate_series(3,{rows}) i"
        ));
        let original = f.commit("number unique values");
        f.run(json!({"op":"branch","name":"original"}));
        let rotate = format!(
            "UPDATE $roads SET name='temp-'||id;
            UPDATE $roads SET name=((id % {rows})+1)::text"
        );
        f.sql(&rotate);
        let rotated = f.commit("rotate unique values");
        let assert_values = |f: &mut Fixture, rotated: bool| {
            let expected = if rotated {
                format!("((id % {rows})+1)::text")
            } else {
                "id::text".into()
            };
            assert!(
                f.db.query_one(
                    &format!(
                        "SELECT count(*)={rows} AND bool_and(name={expected}) FROM {}.roads",
                        f.schema
                    ),
                    &[]
                )
                .unwrap()
                .get::<_, bool>(0)
            );
            assert_eq!(f.status()["clean"], true);
        };
        f.run(json!({"op":"switch","branch":"original"}));
        assert_values(&mut f, false);
        f.run(json!({"op":"switch","branch":"main"}));
        assert_values(&mut f, true);
        f.sql("UPDATE $roads SET name='temp-'||id; UPDATE $roads SET name=id::text");
        assert_eq!(
            f.run(json!({"op":"restore","discard":true}))["restored_records"],
            rows
        );
        assert_values(&mut f, true);
        f.sql("UPDATE $roads SET name='dirty-'||id");
        f.run(json!({"op":"reset","target":original,"hard":true}));
        assert_values(&mut f, false);
        f.run(json!({"op":"reset","target":rotated,"hard":true}));
        assert_values(&mut f, true);
        f.run(json!({"op":"revert","target":rotated}));
        assert_values(&mut f, false);
        f.run(json!({"op":"fsck"}));
    }
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn regression_historical_working_diff_includes_schema_changes() {
    let mut f = Fixture::new();
    let original = f.head();
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"alter_type","name":"width","data_type":"text"}}));
    let compare = |f: &Fixture, from: &str| {
        let working = f.run(json!({"op":"diff","from":from}));
        let committed = f.run(json!({"op":"diff","from":from,"to":"HEAD"}));
        assert_eq!(working, committed);
        assert_eq!(working["total"], 0);
        assert_eq!(working["schema_changes"].as_array().unwrap().len(), 1);
    };
    compare(&f, &original);
    f.sql("DELETE FROM $roads");
    let empty = f.commit("empty table");
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"note","data_type":"text"}}));
    compare(&f, &empty);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn regression_long_primary_key_import_rolls_back() {
    let mut f = Fixture::new();
    let head = f.head();
    f.db.batch_execute(&format!(
        "CREATE TABLE {}.long_keys(id text PRIMARY KEY, value text);
         INSERT INTO {}.long_keys VALUES(repeat('a',9000),'one')",
        f.schema, f.schema
    ))
    .unwrap();
    let result = f.app.execute(
        serde_json::from_value(json!({
            "op":"import","dataset":"long","schema":f.schema,"table":"long_keys"
        }))
        .unwrap(),
    );
    assert!(
        matches!(result, Err(geoledger::core::Error::Invalid(_))),
        "{result:?}"
    );
    assert_eq!(f.head(), head);
    let tracking: i64 =
        f.db.query_one(
            "SELECT count(*) FROM _geoledger.tracked WHERE table_oid=$1::text::regclass",
            &[&format!("{}.long_keys", f.schema)],
        )
        .unwrap()
        .get(0);
    assert_eq!(tracking, 0);
    assert_eq!(f.run(json!({"op":"recover"}))["recovered"], false);
    assert_eq!(f.status()["clean"], true);
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn schema_merge_refuses_lossy_values_and_accepts_exact_target_values() {
    let mut f = Fixture::new();
    f.run(json!({"op":"branch","name":"edit"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"alter_type","name":"name","data_type":"varchar(8)"},"author":"test"}));
    let before = f.head();
    f.run(json!({"op":"switch","branch":"edit"}));
    f.sql("UPDATE $roads SET name='abcdefghi' WHERE id=1");
    f.commit("long text");
    f.run(json!({"op":"switch","branch":"main"}));
    f.error(json!({"op":"merge","source":"edit","author":"test"}));
    assert_eq!(f.head(), before);
    assert_eq!(f.name(), "Original");
    assert_eq!(f.status()["clean"], true);
    f.run(json!({"op":"switch","branch":"edit"}));
    f.sql("UPDATE $roads SET name='Exact' WHERE id=1");
    f.commit("exact text");
    f.run(json!({"op":"switch","branch":"main"}));
    f.run(json!({"op":"merge","source":"edit","author":"test"}));
    assert_eq!(f.name(), "Exact");
    assert_eq!(
        f.run(json!({"op":"show","dataset":"roads","key":"1"}))["record"]["fields"]["name"]["value"],
        "Exact"
    );
    assert_eq!(f.run(json!({"op":"fsck"}))["ok"], true);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn deleted_column_conflicts_with_new_row_values_in_both_merge_directions() {
    for reverse in [false, true] {
        let mut f = Fixture::new();
        f.run(json!({"op":"branch","name":"edit"}));
        f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"width","discard":true},"author":"test"}));
        f.run(json!({"op":"switch","branch":"edit"}));
        f.sql("INSERT INTO $roads(id,name,width) VALUES(3,'third',30)");
        f.commit("new row");
        if !reverse {
            f.run(json!({"op":"switch","branch":"main"}));
        }
        let before = f.head();
        f.error(json!({"op":"merge","source":if reverse {"main"} else {"edit"},"author":"test"}));
        assert_eq!(f.head(), before);
        assert_eq!(f.status()["clean"], true);
    }
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn defaulted_column_is_filled_for_rows_from_the_older_schema() {
    let mut f = Fixture::new();
    f.run(json!({"op":"branch","name":"edit"}));
    f.sql("ALTER TABLE $roads ADD COLUMN flag integer NOT NULL DEFAULT 7");
    f.commit("default");
    f.run(json!({"op":"switch","branch":"edit"}));
    f.sql("INSERT INTO $roads(id,name,width) VALUES(3,'third',30)");
    f.commit("new row");
    f.run(json!({"op":"switch","branch":"main"}));
    f.run(json!({"op":"merge","source":"edit","author":"test"}));
    assert_eq!(
        f.run(json!({"op":"show","dataset":"roads","key":"3"}))["record"]["fields"]["flag"]["value"],
        "7"
    );
    let flag: i32 =
        f.db.query_one(
            &format!("SELECT flag FROM \"{}\".roads WHERE id=3", f.schema),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(flag, 7);
    assert_eq!(f.status()["clean"], true);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn previews_have_a_byte_budget_and_keyset_restore_visits_every_dirty_row() {
    let mut f = Fixture::new();
    f.sql("INSERT INTO $roads(id,name,width) SELECT i,repeat('a',8192)||i,0 FROM generate_series(3,1502) i");
    f.commit("large baseline");
    let baseline = f.head();
    f.sql("UPDATE $roads SET name=repeat('b',8192)||id,width=1 WHERE id>=3");
    let preview = f.run(json!({"op":"status","limit":1000}));
    let count = preview["diff"]["changes"].as_array().unwrap().len();
    assert!(count > 0 && count < 1000);
    assert_eq!(preview["diff"]["total"], 1500);
    assert_eq!(preview["diff"]["truncated"], true);
    let diff = f.run(json!({"op":"diff","from":baseline,"limit":1000}));
    assert_eq!(diff["total"], 1500);
    assert!(diff["changes"].as_array().unwrap().len() < 1000);
    let restored = f.run(json!({"op":"restore","discard":true}));
    assert_eq!(restored["restored_records"], 1500);
    assert_eq!(f.status()["clean"], true);
    let changed: i64 =
        f.db.query_one(
            &format!(
                "SELECT count(*) FROM \"{}\".roads WHERE id>=3 AND width<>0",
                f.schema
            ),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(changed, 0);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn column_rename_preserves_same_named_table_index_and_function() {
    let mut f = Fixture::new();
    f.db.batch_execute(&format!(
        r#"
        CREATE TABLE "{schema}".features(id integer PRIMARY KEY, features text, lower text);
        CREATE INDEX lower ON "{schema}".features (lower(lower));
        INSERT INTO "{schema}".features VALUES(1,'table value','FUNCTION VALUE');
    "#,
        schema = f.schema
    ))
    .unwrap();
    f.run(json!({"op":"import","dataset":"extra","schema":f.schema,"table":"features","author":"test"}));
    let old = f.head();
    for (name, new_name) in [("features", "label"), ("lower", "note")] {
        f.run(json!({"op":"alter_schema","dataset":"extra","change":{"action":"rename","name":name,"new_name":new_name},"author":"test"}));
    }
    assert_eq!(f.status()["clean"], true);
    assert_eq!(
        f.run(json!({"op":"show","dataset":"extra","key":"1"}))["record"]["fields"]["note"]["value"],
        "FUNCTION VALUE"
    );
    f.run(json!({"op":"reset","target":old,"hard":true}));
    assert_eq!(f.status()["clean"], true);
    assert_eq!(
        f.run(json!({"op":"show","dataset":"extra","key":"1"}))["record"]["fields"]["features"]["value"],
        "table value"
    );
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
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
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
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
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn restores_test_fixture_and_reverts_by_creating_a_new_commit() {
    // Fixture refuses any database whose name is not geoledger_test.
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
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
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
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
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
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn unsupported_schema_drift_is_rejected() {
    let mut f = Fixture::new();
    f.sql("ALTER TABLE $roads ALTER COLUMN width SET NOT NULL");
    f.error(json!({"op":"commit","author":"integration","message":"reject DDL drift"}));
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn schema_commands_preserve_history_and_restore_lossy_casts() {
    let mut f = Fixture::new();
    let initial = f.head();
    f.run(json!({"op":"branch","name":"original"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"note","data_type":"text"}}));
    f.sql("UPDATE $roads SET note='remember' WHERE id=1");
    let with_note = f.commit("populate new field");
    let before = f.run(json!({"op":"schema","dataset":"roads"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"note","new_name":"memo"}}));
    let diff = f.run(json!({"op":"diff","from":with_note,"to":"HEAD","limit":1}));
    assert_eq!(diff["total"], 2);
    assert_eq!(diff["truncated"], true);
    assert_eq!(diff["changes"].as_array().unwrap().len(), 1);
    assert_eq!(diff["schema_changes"].as_array().unwrap().len(), 1);
    let after = f.run(json!({"op":"schema","dataset":"roads"}));
    let field_id = |v: &Value, name: &str| {
        v["schema"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap()["metadata"]["geoledger.column-id"]
            .clone()
    };
    assert_eq!(field_id(&before, "note"), field_id(&after, "memo"));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"alter_type","name":"width","data_type":"integer"}}));
    f.error(
        json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"memo"}}),
    );
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"memo","discard":true}}));
    let last = f.head();
    f.run(json!({"op":"branch","name":"final"}));
    f.run(json!({"op":"switch","branch":"original"}));
    assert_eq!(f.head(), initial);
    let width: String =
        f.db.query_one(
            &format!("SELECT width::text FROM {}.roads WHERE id=1", f.schema),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(width, "10.12345678901234567890");
    f.run(json!({"op":"reset","target":with_note,"hard":true}));
    let note: String =
        f.db.query_one(
            &format!("SELECT note FROM {}.roads WHERE id=1", f.schema),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(note, "remember");
    f.run(json!({"op":"switch","branch":"final"}));
    assert_eq!(f.head(), last);
    f.run(json!({"op":"revert","target":last}));
    let memo: String =
        f.db.query_one(
            &format!("SELECT memo FROM {}.roads WHERE id=1", f.schema),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(memo, "remember");
    assert!(f.status()["clean"].as_bool().unwrap());
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn external_ddl_is_detected_committed_and_discarded() {
    let mut f = Fixture::new();
    let initial = f.head();
    f.sql("ALTER TABLE $roads ADD COLUMN note text DEFAULT 'hello'");
    assert_eq!(f.status()["clean"], false);
    assert_eq!(f.status()["schema_changes"].as_array().unwrap().len(), 1);
    f.error(json!({"op":"switch","branch":"main"}));
    assert_eq!(f.status()["record_counts_complete"], false);
    let committed = f.run(json!({"op":"commit","message":"external add"}));
    assert_eq!(committed["changed_records"], Value::Null);
    assert_eq!(committed["rescanned_records"], 2);
    let with_default = f.head();
    f.sql("ALTER TABLE $roads RENAME COLUMN note TO memo; ALTER TABLE $roads ALTER COLUMN width TYPE text USING width::text");
    f.commit("external rename and type");
    f.sql("ALTER TABLE $roads DROP COLUMN memo; ALTER TABLE $roads ADD COLUMN memo text");
    assert_eq!(f.status()["clean"], false);
    f.run(json!({"op":"restore","discard":true}));
    let memo: String =
        f.db.query_one(
            &format!("SELECT memo FROM {}.roads WHERE id=1", f.schema),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(memo, "hello");
    f.sql("ALTER TABLE $roads DROP COLUMN memo");
    let dropped = f.commit("external drop");
    f.run(json!({"op":"reset","target":with_default,"hard":true}));
    assert!(f.status()["clean"].as_bool().unwrap());
    f.run(json!({"op":"reset","target":dropped,"hard":true}));
    f.run(json!({"op":"reset","target":initial,"hard":true}));
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn invalid_schema_commands_roll_back_without_changing_head_or_table() {
    let mut f = Fixture::new();
    let head = f.head();
    f.error(json!({"op":"alter_schema","dataset":"roads","change":{"action":"alter_type","name":"name","data_type":"integer"}}));
    f.error(json!({"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"evil","data_type":"text); DROP TABLE roads; --"}}));
    f.error(json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"id","discard":true}}));
    f.error(json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"name","discard":true}}));
    assert_eq!(f.head(), head);
    assert_eq!(f.name(), "Original");
    assert_eq!(f.status()["clean"], true);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn schema_rename_merges_with_independent_row_edits() {
    let mut f = Fixture::new();
    f.run(json!({"op":"branch","name":"draft"}));
    f.run(json!({"op":"switch","branch":"draft"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"width","new_name":"breadth"}}));
    f.run(json!({"op":"switch","branch":"main"}));
    f.sql("UPDATE $roads SET width=77,name='Edited' WHERE id=1");
    f.commit("edit existing field on main");
    f.run(json!({"op":"merge","source":"draft"}));
    let width: String =
        f.db.query_one(
            &format!("SELECT breadth::text FROM {}.roads WHERE id=1", f.schema),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(width, "77");
    assert_eq!(f.name(), "Edited");
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn schema_commit_recovery_uses_the_new_binding() {
    use geoledger_core::PendingOperation;
    use geoledger_storage::Repository;
    let mut f = Fixture::new();
    let before = {
        Repository::open(f._directory.path())
            .unwrap()
            .state()
            .unwrap()
    };
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"width","new_name":"breadth"}}));
    let operation: String =
        f.db.query_one(
            "SELECT operation FROM _geoledger.repositories WHERE id=$1",
            &[&before.repository_id],
        )
        .unwrap()
        .get(0);
    {
        let repo = Repository::open(f._directory.path()).unwrap();
        let after = repo.state().unwrap();
        repo.begin().unwrap();
        repo.save_state(&before).unwrap();
        repo.prepare(&PendingOperation {
            id: operation,
            before_head: before.head().unwrap().clone(),
            after,
        })
        .unwrap();
        repo.commit().unwrap();
    }
    assert_eq!(f.run(json!({"op":"recover"}))["recovered"], true);
    assert_eq!(f.status()["clean"], true);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn current_format_supports_field_history_from_first_import() {
    use geoledger_storage::Repository;
    let mut f = Fixture::new();
    let original_head;
    {
        let repo = Repository::open(f._directory.path()).unwrap();
        let state = repo.state().unwrap();
        assert_eq!(state.version, geoledger_core::STATE_VERSION);
        for binding in state.bindings.values() {
            geoledger_core::schema::validate_format(&binding.schema).unwrap();
        }
        original_head = state.head().unwrap().clone();
    }
    let version: i32 =
        f.db.query_one("SELECT version FROM _geoledger.format", &[])
            .unwrap()
            .get(0);
    assert_eq!(version, geoledger_core::FORMAT_VERSION as i32);
    f.sql("ALTER TABLE $roads RENAME COLUMN width TO breadth");
    f.commit("external field rename");
    f.run(json!({"op":"reset","target":original_head.as_str(),"hard":true}));
    assert_eq!(f.status()["clean"], true);
    assert_eq!(f.run(json!({"op":"fsck"}))["ok"], true);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn concurrent_schema_conflicts_fail_without_overwriting_working_copy() {
    let mut f = Fixture::new();
    f.run(json!({"op":"branch","name":"draft"}));
    f.run(json!({"op":"switch","branch":"draft"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"width","discard":true}}));
    f.run(json!({"op":"switch","branch":"main"}));
    f.sql("UPDATE $roads SET width=999 WHERE id=1");
    let head = f.commit("edit dropped field");
    f.error(json!({"op":"merge","source":"draft"}));
    assert_eq!(f.head(), head);
    assert_eq!(f.status()["clean"], true);
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn batched_inserts_updates_deletes_restore_exact_rows() {
    let mut f = Fixture::new();
    f.sql(
        "INSERT INTO $roads(id,name,width) SELECT i,'row-'||i,i FROM generate_series(3,2202) AS i",
    );
    let baseline = f.commit("bulk seed");
    f.sql("DELETE FROM $roads WHERE id<=500; UPDATE $roads SET width=width+100 WHERE id BETWEEN 501 AND 1500; INSERT INTO $roads(id,name,width) SELECT i,'row-'||i,i FROM generate_series(2203,2702) AS i");
    let status = f.status();
    assert_eq!(
        status["summary"],
        json!({"inserted":500,"updated":1000,"deleted":500})
    );
    f.commit("bulk changes");
    f.run(json!({"op":"reset","target":baseline,"hard":true}));
    assert_eq!(f.name(), "Original");
    let count: i64 =
        f.db.query_one(&format!("SELECT count(*) FROM {}.roads", f.schema), &[])
            .unwrap()
            .get(0);
    assert_eq!(count, 2202);
    assert_eq!(f.status()["clean"], true);
    f.run(json!({"op":"fsck"}));
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn quoted_column_rename_preserves_unique_constraint_and_indexes() {
    let mut f = Fixture::new();
    let initial = f.head();
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"name","new_name":"select"}}));
    let value: String =
        f.db.query_one(
            &format!("SELECT \"select\" FROM {}.roads WHERE id=1", f.schema),
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(value, "Original");
    f.run(json!({"op":"reset","target":initial,"hard":true}));
    assert_eq!(f.name(), "Original");
    assert_eq!(f.status()["clean"], true);
}

#[test]
#[ignore = "requires disposable GL_TEST_DATABASE_URL"]
fn independent_schema_additions_merge_and_incompatible_renames_fail() {
    let f = Fixture::new();
    f.run(json!({"op":"branch","name":"draft"}));
    f.run(json!({"op":"switch","branch":"draft"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"a","data_type":"text"}}));
    f.run(json!({"op":"switch","branch":"main"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"b","data_type":"text"}}));
    f.run(json!({"op":"merge","source":"draft"}));
    let record = f.run(json!({"op":"show","dataset":"roads","key":"1"}));
    assert!(record["record"]["fields"].get("a").is_some());
    assert!(record["record"]["fields"].get("b").is_some());
    f.run(json!({"op":"branch","name":"other"}));
    f.run(json!({"op":"switch","branch":"other"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"width","new_name":"width_other"}}));
    f.run(json!({"op":"switch","branch":"main"}));
    f.run(json!({"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"width","new_name":"width_main"}}));
    let head = f.head();
    f.error(json!({"op":"merge","source":"other"}));
    assert_eq!(f.head(), head);
    assert_eq!(f.status()["clean"], true);
    f.run(json!({"op":"fsck"}));
}
