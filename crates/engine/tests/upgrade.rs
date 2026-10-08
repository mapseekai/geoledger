//! Format upgrades are explicit: an older database is refused at startup and
//! upgraded only by `Application::upgrade`, keeping every existing row.
use geoledger_engine::{Application, FORMAT_VERSION, Storage};
use serde_json::json;
type TestResult = Result<(), Box<dyn std::error::Error>>;
const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const DATASET: &str = "22222222-2222-4222-8222-222222222222";
const SEED: &str = "
INSERT INTO gl_projects(id,name) VALUES('11111111-1111-4111-8111-111111111111','legacy');
INSERT INTO gl_project_members VALUES('11111111-1111-4111-8111-111111111111','alice','owner');
INSERT INTO gl_project_members VALUES('11111111-1111-4111-8111-111111111111','bob','editor');
INSERT INTO gl_datasets VALUES('11111111-1111-4111-8111-111111111111','22222222-2222-4222-8222-222222222222','roads');
INSERT INTO gl_audit_events(project,subject,action,detail) VALUES('11111111-1111-4111-8111-111111111111','alice','create_project','{}');
";

fn check_upgraded(app: &Application) -> TestResult {
    let refused = app.migrate().err().ok_or("format 5 accepted at startup")?;
    assert_eq!(refused.status, 409);
    assert!(refused.to_string().contains("geoledger-server migrate"));
    assert!(app.check_schema().is_err());
    assert_eq!(app.upgrade(true)?, (5, FORMAT_VERSION));
    assert!(
        app.migrate().is_err(),
        "dry run must not change the database"
    );
    assert_eq!(app.upgrade(false)?, (5, FORMAT_VERSION));
    app.migrate()?;
    app.check_schema()?;
    assert_eq!(app.upgrade(false)?, (FORMAT_VERSION, FORMAT_VERSION));
    let project = app.execute("bob", "get_project", json!({"project":PROJECT}))?;
    assert_eq!(project["state"], "active");
    assert_eq!(project["role"], "editor");
    let datasets = app.execute("bob", "list_datasets", json!({"project":PROJECT}))?;
    assert_eq!(datasets[0]["dataset"], DATASET);
    app.execute(
        "alice",
        "remove_member",
        json!({"project":PROJECT,"subject":"bob"}),
    )?;
    let audit = app.execute("alice", "audit", json!({"project":PROJECT}))?;
    assert_eq!(audit["events"].as_array().ok_or("events")?.len(), 2);
    Ok(())
}

#[test]
fn sqlite_format_5_upgrades_explicitly() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("legacy.sqlite3");
    {
        let db = rusqlite::Connection::open(&path)?;
        db.execute_batch(include_str!("../src/sqlite.sql"))?;
        db.execute_batch(SEED)?;
    }
    check_upgraded(&Application::new(Storage::Sqlite(path)))
}

#[test]
fn fresh_databases_are_created_at_the_current_format() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("fresh.sqlite3")));
    assert!(app.upgrade(false).is_err(), "nothing to upgrade yet");
    app.migrate()?;
    assert_eq!(app.upgrade(true)?, (FORMAT_VERSION, FORMAT_VERSION));
    Ok(())
}

#[test]
#[ignore = "requires GL_TEST_DATABASE_URL with permission to create databases"]
fn postgis_format_5_upgrades_explicitly() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let name = format!("geoledger_upgrade_{}", uuid::Uuid::new_v4().simple());
    let mut admin = postgres::Client::connect(&dsn, postgres::NoTls)?;
    admin.batch_execute(&format!("CREATE DATABASE {name}"))?;
    let result = (|| -> TestResult {
        let target = dsn.replacen("/geoledger_test", &format!("/{name}"), 1);
        let mut db = postgres::Client::connect(&target, postgres::NoTls)?;
        db.batch_execute("CREATE EXTENSION postgis")?;
        db.batch_execute(include_str!("../src/postgis.sql"))?;
        db.batch_execute(SEED)?;
        drop(db);
        check_upgraded(&Application::new(Storage::Postgis(target)))
    })();
    admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))?;
    result
}
