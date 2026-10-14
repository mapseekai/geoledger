use geoledger_engine::{Application, FORMAT_VERSION, Storage};
use serde_json::json;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn fresh_initialization_process() -> TestResult {
    let Some(path) = std::env::var_os("GL_AUTH_FRESH_DATABASE") else {
        return Ok(());
    };
    let gate = std::env::var_os("GL_AUTH_FRESH_GATE").ok_or("missing start gate")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !std::path::Path::new(&gate).exists() {
        if std::time::Instant::now() >= deadline {
            return Err("start gate timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let app = Application::new(Storage::Sqlite(path.into()));
    app.migrate()?;
    app.bootstrap_admin("admin", "fresh-process-test-password")?;
    Ok(())
}

#[test]
fn concurrent_processes_initialize_a_fresh_database() -> TestResult {
    for _ in 0..4 {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("fresh.db");
        let gate = dir.path().join("start");
        let mut children = Vec::new();
        for _ in 0..6 {
            children.push(
                std::process::Command::new(std::env::current_exe()?)
                    .args(["--exact", "fresh_initialization_process", "--nocapture"])
                    .env("GL_AUTH_FRESH_DATABASE", &path)
                    .env("GL_AUTH_FRESH_GATE", &gate)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?,
            );
        }
        std::fs::write(&gate, b"start")?;
        let mut successful = true;
        for mut child in children {
            successful &= child.wait()?.success();
        }
        assert!(successful, "concurrent fresh database startup failed");
        let db = rusqlite::Connection::open(&path)?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM gl_users", [], |r| r.get::<_, i64>(0))?,
            1
        );
    }
    Ok(())
}

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
