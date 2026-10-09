use geoledger_engine::{Application, Storage};
use serde_json::{Value, json};
fn scenario(app: &Application) -> Result<(), Box<dyn std::error::Error>> {
    let result = inner(app);
    if let Err(e) = &result {
        let mut cause = e.source();
        while let Some(c) = cause {
            eprintln!("test cause: {c}");
            cause = c.source();
        }
    }
    result
}
fn inner(app: &Application) -> Result<(), Box<dyn std::error::Error>> {
    app.migrate()?;
    let p = app.execute("alice", "create_project", json!({"name":"conformance"}))?["project"]
        .as_str()
        .ok_or("project")?
        .to_owned();
    let call = |s: &str, op: &str, mut v: Value| {
        v["project"] = json!(p);
        app.execute(s, op, v)
    };
    call(
        "alice",
        "set_member",
        json!({"subject":"bob","role":"editor"}),
    )?;
    let d = call(
        "alice",
        "create_dataset",
        json!({"geometry_type":"point","name":"roads","coordinate_dimension":3}),
    )?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    let w = call("alice", "create_workspace", json!({}))?["workspace"].clone();
    let w2 = call("alice", "create_workspace", json!({}))?["workspace"].clone();
    let feature = json!({"type":"Feature","id":"one","properties":{"n":18446744073709551615u64},"geometry":{"type":"Point","coordinates":[1,2,3]}});
    call(
        "alice",
        "save",
        json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"one","feature":feature}]}),
    )?;
    let req = json!({"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"seed"});
    let published = call("alice", "publish", req.clone())?;
    assert_eq!(published, call("alice", "publish", req)?);
    let rows = call("bob", "features", json!({"dataset":d,"bbox":[0,0,3,3]}))?;
    assert_eq!(rows["features"][0], feature);
    assert!(call("eve", "features", json!({"dataset":d})).is_err());
    assert_eq!(
        call("bob", "features", json!({"dataset":d,"revision":0}))?["features"],
        json!([])
    );
    assert_eq!(
        call("alice", "commit", json!({"revision":1}))?["changes"][0]["after"],
        feature
    );
    let first_history = call("alice", "history", json!({}))?;
    assert_eq!(first_history.as_array().ok_or("history")?.len(), 1);
    assert_eq!(first_history[0]["source_workspace"], w);
    let mut second_feature = feature.clone();
    second_feature["id"] = json!("two");
    call(
        "alice",
        "save",
        json!({"workspace":w2,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"two","feature":second_feature}]}),
    )?;
    let rebased = call(
        "alice",
        "rebase",
        json!({"workspace":w2,"expected_workspace_version":1,"expected_head":1,"resolutions":[]}),
    )?;
    assert_eq!(rebased["base_revision"], 1);
    call(
        "alice",
        "publish",
        json!({"workspace":w2,"expected_workspace_version":2,"request_id":uuid::Uuid::new_v4(),"message":"rebased workspace"}),
    )?;
    let history = call("alice", "history", json!({}))?;
    let history = history.as_array().ok_or("history")?;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["source_workspace"], w);
    assert_eq!(history[0]["source_base_revision"], 0);
    assert_eq!(history[1]["source_workspace"], w2);
    assert_eq!(history[1]["source_base_revision"], 1);
    assert!(
        !call("alice", "audit", json!({}))?["events"]
            .as_array()
            .ok_or("audit")?
            .is_empty()
    );
    Ok(())
}
#[test]
fn sqlite_contract() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("test.db");
    scenario(&Application::new(Storage::Sqlite(path.clone())))?;
    let c = rusqlite::Connection::open(path)?;
    let extension =
        std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
    geoledger_spatialite::load(&c, std::path::Path::new(&extension))?;
    let (count, z, srid): (i64, f64, i32) = c.query_row(
        "SELECT count(*), ST_Z(geom_z), ST_SRID(geom_z) FROM gl_history WHERE typeof(geom_z)='blob' AND geom IS NULL", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!((count, z, srid), (2, 3.0, 4326));
    for table in ["gl_history", "gl_workspace_changes"] {
        for column in ["geom", "geom_z"] {
            assert_eq!(
                c.query_row("SELECT CheckSpatialIndex(?1,?2)", [table, column], |row| {
                    row.get::<_, i32>(0)
                })?,
                1
            );
        }
    }
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_contract() -> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut c = postgres::Client::connect(&dsn, postgres::NoTls)?;
    let name: String = c.query_one("SELECT current_database()", &[])?.get(0);
    assert_eq!(name, "geoledger_test");
    scenario(&Application::new(Storage::Postgis(dsn)))
}

#[test]
fn deletion_rolls_back_with_audit_and_purges_storage_atomically()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("purge.sqlite3");
    let app = Application::new(Storage::Sqlite(db.clone()));
    app.migrate()?;
    let p = app.execute("alice", "create_project", json!({"name":"purge"}))?["project"]
        .as_str()
        .ok_or("project")?
        .to_owned();
    let d = app.execute(
        "alice",
        "create_dataset",
        json!({"project":p,"name":"points","geometry_type":"point"}),
    )?["dataset"]
        .clone();
    let w = app.execute("alice", "create_workspace", json!({"project":p}))?["workspace"].clone();
    app.execute("alice","save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"p","feature":{"type":"Feature","id":"p","properties":{},"geometry":null}}]}))?;
    app.execute("alice","publish",json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"seed"}))?;
    let connection = rusqlite::Connection::open(&db)?;
    connection.execute_batch("CREATE TRIGGER fail_delete_audit BEFORE INSERT ON gl_audit_events WHEN NEW.action='delete_dataset' BEGIN SELECT RAISE(ABORT,'test rollback'); END;")?;
    assert!(
        app.execute(
            "alice",
            "delete_dataset",
            json!({"project":p,"dataset":d,"confirm_name":"points"})
        )
        .is_err()
    );
    for table in [
        "gl_datasets",
        "gl_workspaces",
        "gl_commits",
        "gl_commit_changes",
        "gl_history",
        "gl_idempotency",
    ] {
        assert_eq!(
            connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))?,
            1,
            "{table} must roll back"
        );
    }
    assert_eq!(
        connection.query_row("SELECT count(*) FROM gl_purge", [], |r| r.get::<_, i64>(0))?,
        0
    );
    assert!(connection.execute("DELETE FROM gl_commits", []).is_err());
    connection.execute_batch("DROP TRIGGER fail_delete_audit")?;
    app.execute(
        "alice",
        "delete_project",
        json!({"project":p,"confirm_name":"purge"}),
    )?;
    for table in [
        "gl_datasets",
        "gl_workspaces",
        "gl_workspace_changes",
        "gl_commits",
        "gl_commit_changes",
        "gl_history",
        "gl_idempotency",
        "gl_purge",
        "idx_gl_history_geom",
        "idx_gl_history_geom_z",
        "idx_gl_workspace_changes_geom",
        "idx_gl_workspace_changes_geom_z",
    ] {
        assert_eq!(
            connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))?,
            0,
            "{table} must be empty"
        );
    }
    assert!(
        connection.query_row("SELECT count(*) FROM gl_audit_events", [], |r| r
            .get::<_, i64>(0))?
            > 0
    );
    assert!(
        connection
            .execute("DELETE FROM gl_audit_events", [])
            .is_err()
    );
    Ok(())
}
