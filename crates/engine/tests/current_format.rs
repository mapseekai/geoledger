use geoledger_engine::{Application, FORMAT_VERSION, Storage};
use serde_json::json;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn initialization_preserves_current_data() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("current.db");
    let app = Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    let project =
        app.execute("alice", "create_project", json!({"name":"roads"}))?["project"].clone();
    app.migrate()?;
    app.check_schema()?;
    assert_eq!(
        app.execute("alice", "get_project", json!({"project":project}))?["state"],
        "active"
    );
    let db = rusqlite::Connection::open(path)?;
    assert_eq!(
        db.query_row("SELECT version FROM gl_format", [], |r| r.get::<_, i32>(0))?,
        FORMAT_VERSION
    );
    Ok(())
}

#[test]
fn other_formats_are_rejected_without_modifying_data() -> TestResult {
    for version in [FORMAT_VERSION - 1, FORMAT_VERSION + 1] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("other.db");
        let db = rusqlite::Connection::open(&path)?;
        db.execute_batch("CREATE TABLE gl_format(singleton BOOLEAN, version INTEGER); CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('preserve');")?;
        db.execute("INSERT INTO gl_format VALUES(true, ?1)", [version])?;
        let app = Application::new(Storage::Sqlite(path));
        assert_eq!(
            app.migrate()
                .err()
                .ok_or("unexpected format accepted")?
                .status,
            409
        );
        assert_eq!(
            app.check_schema()
                .err()
                .ok_or("unexpected format accepted")?
                .status,
            409
        );
        assert_eq!(
            db.query_row("SELECT version FROM gl_format", [], |r| r.get::<_, i32>(0))?,
            version
        );
        assert_eq!(
            db.query_row("SELECT value FROM sentinel", [], |r| r.get::<_, String>(0))?,
            "preserve"
        );
    }
    Ok(())
}
