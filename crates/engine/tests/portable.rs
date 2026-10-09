//! Logical export/import round trips keep every table, including history,
//! audit and publication receipts, and reject damaged files atomically.
use geoledger_engine::{Application, DataSummary, Storage};
use serde_json::{Value, json};
use std::io::Cursor;
use uuid::Uuid;
type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Seed {
    project: String,
    dataset: String,
    request_id: String,
    workspace: String,
    receipt: Value,
}

/// Populate every table: members (one removed), archived and deleted projects,
/// published history with a deletion, an open draft, receipts and audit.
fn seed(app: &Application) -> Result<Seed, Box<dyn std::error::Error>> {
    let call = |s: &str, op: &str, v: Value| app.execute(s, op, v);
    let project = call("alice", "create_project", json!({"name":"道路"}))?["project"]
        .as_str()
        .ok_or("project")?
        .to_owned();
    let p = json!(project);
    let dataset = call(
        "alice",
        "create_dataset",
        json!({"geometry_type":"point","project":p,"name":"roads"}),
    )?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    for (subject, role) in [("bob", "editor"), ("carol", "viewer")] {
        call(
            "alice",
            "set_member",
            json!({"project":p,"subject":subject,"role":role}),
        )?;
    }
    let edit =
        |id: &str, feature: Value| json!({"dataset":dataset,"feature_id":id,"feature":feature});
    let feature = |id: &str, x: f64| {
        json!({"type":"Feature","id":id,"properties":{"name":id,"exact":18446744073709551615u64,"x":x},
               "geometry":{"type":"Point","coordinates":[x,30.5,12]}})
    };
    let w = call("bob", "create_workspace", json!({"project":p}))?["workspace"]
        .as_str()
        .ok_or("workspace")?
        .to_owned();
    call(
        "bob",
        "save",
        json!({"project":p,"workspace":w,"expected_workspace_version":0,
               "edits":[edit("a",feature("a",120.1)),edit("b",feature("b",121.0)),
                        edit("c",json!({"type":"Feature","id":"c","properties":{},"geometry":null}))]}),
    )?;
    let request_id = Uuid::new_v4().to_string();
    let publish = json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":request_id,"message":"初始"});
    let receipt = call("bob", "publish", publish)?;
    let w2 = call("alice", "create_workspace", json!({"project":p}))?["workspace"]
        .as_str()
        .ok_or("workspace")?
        .to_owned();
    call(
        "alice",
        "save",
        json!({"project":p,"workspace":w2,"expected_workspace_version":0,
               "edits":[edit("a",feature("a",122.0)),edit("b",Value::Null)]}),
    )?;
    call(
        "alice",
        "publish",
        json!({"project":p,"workspace":w2,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"second"}),
    )?;
    // An open draft with changes stays in the export.
    let draft = call("bob", "create_workspace", json!({"project":p}))?["workspace"]
        .as_str()
        .ok_or("workspace")?
        .to_owned();
    call(
        "bob",
        "save",
        json!({"project":p,"workspace":draft,"expected_workspace_version":0,"edits":[edit("d",feature("d",123.0))]}),
    )?;
    call(
        "alice",
        "remove_member",
        json!({"project":p,"subject":"carol"}),
    )?;
    let archived = call("alice", "create_project", json!({"name":"archived"}))?["project"].clone();
    call(
        "alice",
        "archive_project",
        json!({"project":archived,"archived":true}),
    )?;
    let deleted = call("alice", "create_project", json!({"name":"gone"}))?["project"].clone();
    call(
        "alice",
        "delete_project",
        json!({"project":deleted,"confirm_name":"gone"}),
    )?;
    Ok(Seed {
        project,
        dataset,
        request_id,
        workspace: w,
        receipt,
    })
}

/// More than one export page (1000 rows) in history, changes and drafts, with
/// mixed-case and non-ASCII keys so collation order differs between backends.
fn bulk(app: &Application, seed: &Seed) -> TestResult {
    let p = json!(seed.project);
    let w = app.execute("alice", "create_workspace", json!({"project":p}))?["workspace"]
        .as_str()
        .ok_or("workspace")?
        .to_owned();
    for batch in 0..11i64 {
        let edits: Vec<_> = (batch * 100..(batch + 1) * 100)
            .map(|i| {
                let id = match i % 3 {
                    0 => format!("Road-{i:04}"),
                    1 => format!("road-{i:04}"),
                    _ => format!("道路-{i:04}"),
                };
                json!({"dataset":seed.dataset,"feature_id":id,"feature":{"type":"Feature","id":id,
                       "properties":{"i":i},"geometry":{"type":"Point","coordinates":[i % 180,i % 90]}}})
            })
            .collect();
        if batch == 10 {
            app.execute(
                "alice",
                "publish",
                json!({"project":p,"workspace":w,"expected_workspace_version":10,"request_id":Uuid::new_v4(),"message":"bulk"}),
            )?;
            let next = app.execute("alice", "create_workspace", json!({"project":p}))?["workspace"]
                .as_str()
                .ok_or("workspace")?
                .to_owned();
            app.execute(
                "alice",
                "save",
                json!({"project":p,"workspace":next,"expected_workspace_version":0,"edits":edits}),
            )?;
        } else {
            app.execute(
                "alice",
                "save",
                json!({"project":p,"workspace":w,"expected_workspace_version":batch,"edits":edits}),
            )?;
        }
    }
    Ok(())
}

fn export(app: &Application) -> Result<(Vec<u8>, DataSummary), Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    let summary = app.export_data(&mut out)?;
    Ok((out, summary))
}

/// Business-level equality after import, plus continued writes.
fn check_imported(app: &Application, seed: &Seed, expected: &DataSummary) -> TestResult {
    assert_eq!(&app.data_summary()?, expected);
    let p = json!(seed.project);
    let a = app.execute(
        "bob",
        "features",
        json!({"project":p,"dataset":seed.dataset,"feature_id":"a"}),
    )?;
    assert_eq!(
        a["properties"]["exact"],
        json!(18446744073709551615u64),
        "{a}"
    );
    assert_eq!(a["properties"]["x"], json!(122), "{a}");
    let history = app.execute("bob", "history", json!({"project":p}))?;
    assert!(history.as_array().ok_or("history")?.len() >= 2);
    // The stored receipt answers the original publication retry.
    let retry = app.execute(
        "bob",
        "publish",
        json!({"project":p,"workspace":seed.workspace,"expected_workspace_version":1,"request_id":seed.request_id,"message":"初始"}),
    )?;
    assert_eq!(retry, seed.receipt);
    // Removed members stay removed; archived and deleted projects keep their state.
    assert_eq!(
        app.execute("carol", "get_project", json!({"project":p}))
            .err()
            .map(|e| e.status),
        Some(404)
    );
    let projects = app.execute("alice", "list_projects", json!({}))?;
    let names: Vec<_> = projects
        .as_array()
        .ok_or("projects")?
        .iter()
        .map(|p| (p["name"].clone(), p["state"].clone()))
        .collect();
    assert_eq!(names.len(), 2, "{projects}");
    assert!(names.contains(&(json!("archived"), json!("archived"))));
    // Audit ids continue after the imported events.
    let before = app.execute("alice", "audit", json!({"project":p,"limit":1000}))?["events"]
        .as_array()
        .ok_or("events")?
        .len();
    app.execute(
        "alice",
        "set_member",
        json!({"project":p,"subject":"dave","role":"viewer"}),
    )?;
    let events = app.execute("alice", "audit", json!({"project":p,"limit":1000}))?["events"]
        .as_array()
        .ok_or("events")?
        .clone();
    assert_eq!(events.len(), before + 1);
    Ok(())
}

#[test]
fn sqlite_export_import_round_trip_preserves_everything() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Application::new(Storage::Sqlite(dir.path().join("source.sqlite3")));
    source.migrate()?;
    let seed = seed(&source)?;
    bulk(&source, &seed)?;
    let (bytes, summary) = export(&source)?;
    assert_eq!(summary, source.data_summary()?);
    assert!(summary.rows("gl_history") > 1000 && summary.rows("gl_workspace_changes") > 1000);
    assert_eq!(Application::read_export(&mut Cursor::new(&bytes))?, summary);
    for table in [
        "gl_projects",
        "gl_project_members",
        "gl_datasets",
        "gl_workspaces",
        "gl_commits",
        "gl_history",
        "gl_commit_changes",
        "gl_workspace_changes",
        "gl_idempotency",
        "gl_audit_events",
    ] {
        assert!(summary.rows(table) > 0, "{table} is empty in the seed");
    }
    // Import initializes a missing database.
    let target = Application::new(Storage::Sqlite(dir.path().join("target.sqlite3")));
    assert_eq!(target.import_data(&mut Cursor::new(&bytes))?, summary);
    check_imported(&target, &seed, &summary)?;
    // A second import into a non-empty database is refused.
    let err = target
        .import_data(&mut Cursor::new(&bytes))
        .err()
        .ok_or("import into non-empty database accepted")?;
    assert_eq!(err.status, 409);
    Ok(())
}

#[test]
fn damaged_exports_are_rejected_without_partial_writes() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Application::new(Storage::Sqlite(dir.path().join("source.sqlite3")));
    source.migrate()?;
    seed(&source)?;
    let (bytes, summary) = export(&source)?;
    let text = String::from_utf8(bytes.clone())?;
    let tampered = text.replacen("120.1", "120.2", 1);
    assert!(tampered != text);
    let truncated = &bytes[..bytes.len() - 10];
    let mut lines: Vec<&str> = text.lines().collect();
    let dropped_row = lines
        .iter()
        .rposition(|l| l.starts_with("{\"r\":"))
        .ok_or("row")?;
    lines.remove(dropped_row);
    let missing_row = lines.join("\n") + "\n";
    let newer = text.replacen("\"format\":7", "\"format\":99", 1);
    let target = Application::new(Storage::Sqlite(dir.path().join("target.sqlite3")));
    for (case, input, status) in [
        ("tampered", tampered.as_bytes(), 400),
        ("truncated", truncated, 400),
        ("missing row", missing_row.as_bytes(), 400),
        ("newer format", newer.as_bytes(), 409),
        ("empty", b"".as_slice(), 400),
    ] {
        let err = target
            .import_data(&mut Cursor::new(input))
            .err()
            .ok_or(case)?;
        assert_eq!(err.status, status, "{case}: {err}");
        assert!(Application::read_export(&mut Cursor::new(input)).is_err());
        // Nothing was committed: the same database still accepts the good file.
        assert_eq!(target.data_summary()?.rows("gl_projects"), 0, "{case}");
    }
    assert_eq!(target.import_data(&mut Cursor::new(&bytes))?, summary);
    Ok(())
}

#[test]
fn sqlite_online_backup_is_consistent_and_verified() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("live.sqlite3");
    let app = Application::new(Storage::Sqlite(path));
    app.migrate()?;
    seed(&app)?;
    let target = dir.path().join("backup.sqlite3");
    app.backup(&target)?;
    assert_eq!(
        app.backup(&target).err().map(|e| e.status),
        Some(409),
        "existing backups are never overwritten"
    );
    Application::verify_sqlite_file(&target, std::time::Duration::from_secs(10))?;
    let restored = Application::new(Storage::Sqlite(target));
    restored.migrate()?;
    assert_eq!(restored.data_summary()?, app.data_summary()?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("backup.sqlite3"))?
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "backups are private");
    }
    // A missing database is reported and leaves no file behind.
    let none = Application::new(Storage::Sqlite(dir.path().join("missing.sqlite3")));
    let empty_target = dir.path().join("empty-backup.sqlite3");
    assert_eq!(
        none.backup(&empty_target).err().map(|e| e.status),
        Some(404)
    );
    assert!(!empty_target.exists() && !dir.path().join("missing.sqlite3").exists());
    let garbage = dir.path().join("garbage.sqlite3");
    std::fs::write(&garbage, b"not a database at all, just bytes")?;
    assert!(Application::verify_sqlite_file(&garbage, std::time::Duration::from_secs(10)).is_err());
    Ok(())
}

#[test]
#[ignore = "requires GL_TEST_DATABASE_URL with permission to create databases"]
fn postgis_and_sqlite_exchange_exports_both_ways() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let name = format!("geoledger_portable_{}", Uuid::new_v4().simple());
    let mut admin = postgres::Client::connect(&dsn, postgres::NoTls)?;
    admin.batch_execute(&format!("CREATE DATABASE {name}"))?;
    let result = (|| -> TestResult {
        let target = dsn.replacen("/geoledger_test", &format!("/{name}"), 1);
        let mut db = postgres::Client::connect(&target, postgres::NoTls)?;
        db.batch_execute("CREATE EXTENSION postgis")?;
        drop(db);
        let dir = tempfile::tempdir()?;
        let sqlite = Application::new(Storage::Sqlite(dir.path().join("source.sqlite3")));
        sqlite.migrate()?;
        let seed = seed(&sqlite)?;
        bulk(&sqlite, &seed)?;
        let (bytes, summary) = export(&sqlite)?;
        let pg = Application::new(Storage::Postgis(target));
        assert_eq!(pg.import_data(&mut Cursor::new(&bytes))?, summary);
        assert_eq!(pg.data_summary()?, summary);
        let (from_pg, pg_summary) = export(&pg)?;
        assert_eq!(pg_summary, summary);
        check_imported(&pg, &seed, &summary)?;
        let back = Application::new(Storage::Sqlite(dir.path().join("back.sqlite3")));
        assert_eq!(back.import_data(&mut Cursor::new(&from_pg))?, summary);
        assert_eq!(
            pg.backup(&dir.path().join("pg.sqlite3"))
                .err()
                .map(|e| e.status),
            Some(409)
        );
        Ok(())
    })();
    admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))?;
    result
}
