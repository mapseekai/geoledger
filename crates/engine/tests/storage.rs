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
    let d = call("alice", "create_dataset", json!({"name":"roads"}))?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    let w = call("alice", "create_workspace", json!({}))?["workspace"].clone();
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
    assert_eq!(
        call("alice", "history", json!({}))?
            .as_array()
            .ok_or("history")?
            .len(),
        1
    );
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
    scenario(&Application::new(Storage::Sqlite(
        dir.path().join("test.db"),
    )))
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
