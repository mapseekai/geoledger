use geoledger_engine::{Application, Storage};
use serde_json::json;
use std::time::{Duration, Instant};
#[test]
fn sqlite_lock_deadline_rolls_back_and_restart_retains_committed_data()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("state.db");
    let app = Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    let p =
        app.execute("alice", "create_project", json!({"name":"persistent"}))?["project"].clone();
    let blocker = rusqlite::Connection::open(&path)?;
    blocker.execute_batch("BEGIN IMMEDIATE")?;
    let start = Instant::now();
    let error = app
        .clone()
        .with_timeout(Duration::from_millis(80))
        .execute("alice", "create_project", json!({"name":"blocked"}))
        .err()
        .ok_or("expected busy")?;
    assert!(matches!(error.status, 429 | 504));
    assert!(start.elapsed() < Duration::from_secs(1));
    blocker.execute_batch("ROLLBACK")?;
    drop(app);
    let reopened = Application::new(Storage::Sqlite(path));
    reopened.migrate()?;
    let projects = reopened.execute("alice", "list_projects", json!({}))?;
    assert_eq!(projects.as_array().ok_or("projects")?.len(), 1);
    assert_eq!(projects[0]["project"], p);
    Ok(())
}
#[test]
fn sqlite_commit_failure_preserves_atomic_history_and_original_request_can_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("state.db");
    let app = Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    let p = app.execute("alice", "create_project", json!({"name":"atomic"}))?["project"].clone();
    let d = app.execute(
        "alice",
        "create_dataset",
        json!({"project":p,"name":"points"}),
    )?["dataset"]
        .clone();
    let w = app.execute("alice", "create_workspace", json!({"project":p}))?["workspace"].clone();
    app.execute("alice","save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"one","feature":{"type":"Feature","id":"one","properties":{},"geometry":null}}]}))?;
    let control = rusqlite::Connection::open(&path)?;
    control.execute_batch("CREATE TABLE failure_parent(id INTEGER PRIMARY KEY); CREATE TABLE failure_child(id INTEGER REFERENCES failure_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fail_commit AFTER INSERT ON gl_commits BEGIN INSERT INTO failure_child VALUES(1); END;")?;
    let request = json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"atomic"});
    assert!(app.execute("alice", "publish", request.clone()).is_err());
    for table in [
        "gl_commits",
        "gl_commit_changes",
        "gl_history",
        "gl_idempotency",
    ] {
        let count: i64 =
            control.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        assert_eq!(count, 0, "{table}");
    }
    assert_eq!(
        app.execute("alice", "get_project", json!({"project":p}))?["head"],
        0
    );
    assert_eq!(
        app.execute("alice", "get_workspace", json!({"project":p,"workspace":w}))?["status"],
        "open"
    );
    control.execute_batch("DROP TRIGGER fail_commit")?;
    let result = app.execute("alice", "publish", request.clone())?;
    drop(app);
    let app = Application::new(Storage::Sqlite(path));
    assert_eq!(app.execute("alice", "publish", request)?, result);
    for sql in [
        "DELETE FROM gl_commits",
        "UPDATE gl_history SET properties='{}'",
        "DELETE FROM gl_idempotency",
        "UPDATE gl_commit_changes SET after_value=NULL",
    ] {
        assert!(control.execute(sql, []).is_err(), "immutable {sql}");
    }
    Ok(())
}
