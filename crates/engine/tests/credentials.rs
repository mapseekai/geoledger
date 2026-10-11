use geoledger_engine::{Application, Policy, Storage};
use serde_json::json;
use uuid::Uuid;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn issue_replay_authorization_and_atomic_replacement() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("tokens.db"))).with_policy(Policy {
        admins: ["admin".into()].into(),
        ..Default::default()
    });
    app.migrate()?;
    let request = json!({"request_id":Uuid::new_v4(),"subject":"alice","label":"work","expires_at":"2099-01-01T00:00:00Z"});
    let first = app.execute("admin", "issue_token", request.clone())?;
    assert!(
        first["token"]
            .as_str()
            .is_some_and(|t| t.starts_with("glmt_"))
    );
    assert_eq!(
        app.execute("alice", "issue_token", request.clone())
            .err()
            .ok_or("non-admin allowed")?
            .status,
        403
    );
    let replay = app.execute("admin", "issue_token", request.clone())?;
    assert!(replay["token"].is_null());
    assert_eq!(replay["credential"]["id"], first["credential"]["id"]);
    assert_eq!(replay["replayed"], true);
    let mut changed = request.clone();
    changed["label"] = json!("changed");
    assert_eq!(
        app.execute("admin", "issue_token", changed)
            .err()
            .ok_or("changed replay allowed")?
            .status,
        409
    );
    let mut forbidden = request;
    forbidden["request_id"] = json!(Uuid::new_v4());
    forbidden["subject"] = json!("admin");
    assert_eq!(
        app.execute("admin", "issue_token", forbidden)
            .err()
            .ok_or("admin impersonation")?
            .status,
        403
    );
    let replacement = json!({"request_id":Uuid::new_v4(),"token_id":first["credential"]["id"],"label":"replacement","expires_at":"2099-01-01T00:00:00Z"});
    let next = app.execute("admin", "reissue_token", replacement.clone())?;
    assert_ne!(next["token"], first["token"]);
    assert!(app.execute("admin", "reissue_token", replacement)?["token"].is_null());
    let listed = app.execute("admin", "list_tokens", json!({}))?;
    assert_eq!(listed.as_array().ok_or("array")?.len(), 2);
    assert!(
        !listed
            .to_string()
            .contains(first["token"].as_str().ok_or("token")?)
    );
    let revoke = json!({"token_id":next["credential"]["id"]});
    app.execute("admin", "revoke_token", revoke.clone())?;
    app.execute("admin", "revoke_token", revoke)?;
    Ok(())
}

fn app(storage: Storage) -> Result<Application, Box<dyn std::error::Error>> {
    let app = Application::new(storage).with_policy(Policy {
        admins: ["admin".into()].into(),
        ..Default::default()
    });
    app.migrate()?;
    Ok(app)
}
fn issue(app: &Application, subject: &str) -> Result<serde_json::Value, geoledger_engine::Error> {
    app.execute("admin","issue_token",json!({"request_id":Uuid::new_v4(),"subject":subject,"label":"test","expires_at":"2099-01-01T00:00:00Z"}))
}
fn authentication_and_race(storage: Storage) -> TestResult {
    let first = app(storage.clone())?;
    let second = app(storage.clone())?;
    let issued = issue(&first, "bob")?;
    let token = issued["token"].as_str().ok_or("token")?;
    assert_eq!(
        second.authenticate_managed_token(token)?.as_deref(),
        Some("bob")
    );
    for bad in ["glmt_", "glmt_é", "glmt_unknown"] {
        assert!(second.authenticate_managed_token(bad)?.is_none());
    }
    let promoted = first.clone().with_policy(Policy {
        admins: ["admin".into(), "bob".into()].into(),
        ..Default::default()
    });
    assert!(promoted.authenticate_managed_token(token)?.is_none());
    // Failed replacement leaves old credential valid.
    assert!(first.execute("admin","reissue_token",json!({"request_id":Uuid::new_v4(),"token_id":issued["credential"]["id"],"label":"expired","expires_at":"2000-01-01T00:00:00Z"})).is_err());
    assert!(second.authenticate_managed_token(token)?.is_some());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles:Vec<_>=[first.clone(),second.clone()].into_iter().map(|a|{
        let b=barrier.clone();let id=issued["credential"]["id"].clone();
        std::thread::spawn(move|| {b.wait(); a.execute("admin","reissue_token",json!({"request_id":Uuid::new_v4(),"token_id":id,"label":"new","expires_at":"2099-01-01T00:00:00Z"}))})
    }).collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().map_err(|_| "thread panic"))
        .collect::<Result<_, _>>()?;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .next()
            .ok_or("conflict")?
            .status,
        409
    );
    assert!(second.authenticate_managed_token(token)?.is_none());
    let success = results.into_iter().find_map(Result::ok).ok_or("success")?;
    let new = success["token"].as_str().ok_or("new token")?;
    assert_eq!(
        app(storage)?.authenticate_managed_token(new)?.as_deref(),
        Some("bob")
    );
    first.execute(
        "admin",
        "revoke_token",
        json!({"token_id":success["credential"]["id"]}),
    )?;
    assert!(second.authenticate_managed_token(new)?.is_none());
    Ok(())
}
#[test]
fn sqlite_cross_instance_authentication_and_concurrent_reissue() -> TestResult {
    let dir = tempfile::tempdir()?;
    authentication_and_race(Storage::Sqlite(dir.path().join("db")))
}
#[test]
fn sqlite_failure_rolls_back_credentials_audit_and_receipts_and_backup_preserves_revocations()
-> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("db");
    let app = app(Storage::Sqlite(path.clone()))?;
    let db = rusqlite::Connection::open(&path)?;
    db.execute_batch("CREATE TRIGGER fail_token_audit BEFORE INSERT ON gl_token_audit BEGIN SELECT RAISE(ABORT,'injected test failure'); END;")?;
    assert!(issue(&app, "rollback").is_err());
    for table in ["gl_managed_tokens", "gl_token_receipts", "gl_token_audit"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))?,
            0
        );
    }
    db.execute_batch("DROP TRIGGER fail_token_audit")?;
    let issued = issue(&app, "alice")?;
    let token = issued["token"].as_str().ok_or("secret")?;
    app.execute(
        "admin",
        "revoke_token",
        json!({"token_id":issued["credential"]["id"]}),
    )?;
    let active = issue(&app, "alice")?;
    let active_token = active["token"].as_str().ok_or("active")?;
    let expiring = issue(&app, "expires")?;
    db.execute("UPDATE gl_managed_tokens SET created_at='2000-01-01T00:00:00.000Z', expires_at='2001-01-01T00:00:00.000Z' WHERE id=?1",[expiring["credential"]["id"].as_str().ok_or("id")?])?;
    assert!(
        app.authenticate_managed_token(expiring["token"].as_str().ok_or("expired")?)?
            .is_none()
    );
    let mut export = Vec::new();
    app.export_data(&mut export)?;
    assert!(!String::from_utf8_lossy(&export).contains("gl_managed_tokens"));
    let backup = dir.path().join("backup.db");
    app.backup(&backup)?;
    let restored = Application::new(Storage::Sqlite(backup.clone()));
    assert!(restored.authenticate_managed_token(token)?.is_none());
    assert_eq!(
        restored
            .authenticate_managed_token(active_token)?
            .as_deref(),
        Some("alice")
    );
    let bytes = std::fs::read(backup)?;
    for secret in [token, active_token] {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
    assert!(db.execute("DELETE FROM gl_token_audit", []).is_err());
    assert!(db.execute("DELETE FROM gl_token_receipts", []).is_err());
    db.execute_batch("ALTER TABLE gl_managed_tokens RENAME TO unavailable_managed_tokens")?;
    assert!(app.authenticate_managed_token(active_token).is_err());
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_cross_instance_authentication_and_concurrent_reissue() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = postgres::Client::connect(&dsn, postgres::NoTls)?;
    assert_eq!(
        admin
            .query_one("SELECT current_database()", &[])?
            .get::<_, String>(0),
        "geoledger_test"
    );
    let name = format!("geoledger_tokens_{}", Uuid::new_v4().simple());
    admin.batch_execute(&format!("CREATE DATABASE {name}"))?;
    let test_dsn = dsn.replacen("/geoledger_test", &format!("/{name}"), 1);
    let result = (|| -> TestResult {
        postgres::Client::connect(&test_dsn, postgres::NoTls)?
            .batch_execute("CREATE EXTENSION postgis")?;
        let instance = app(Storage::Postgis(test_dsn.clone()))?;
        let mut db = postgres::Client::connect(&test_dsn, postgres::NoTls)?;
        db.batch_execute("CREATE FUNCTION fail_test_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected test failure'; END $$; CREATE TRIGGER fail_test_audit BEFORE INSERT ON gl_token_audit FOR EACH ROW EXECUTE FUNCTION fail_test_audit();")?;
        assert!(issue(&instance, "rollback").is_err());
        for table in ["gl_managed_tokens", "gl_token_receipts", "gl_token_audit"] {
            assert_eq!(
                db.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
                    .get::<_, i64>(0),
                0
            );
        }
        db.batch_execute("DROP FUNCTION fail_test_audit() CASCADE")?;
        authentication_and_race(Storage::Postgis(test_dsn.clone()))?;
        let active = issue(&instance, "backup")?;
        let revoked = issue(&instance, "backup")?;
        instance.execute(
            "admin",
            "revoke_token",
            json!({"token_id":revoked["credential"]["id"]}),
        )?;
        let expired = issue(&instance, "expired")?;
        db.execute("UPDATE gl_managed_tokens SET created_at='2000-01-01T00:00:00.000Z',expires_at='2001-01-01T00:00:00.000Z' WHERE id=$1",&[&expired["credential"]["id"].as_str().ok_or("id")?])?;
        assert!(
            instance
                .authenticate_managed_token(expired["token"].as_str().ok_or("token")?)?
                .is_none()
        );
        assert!(db.batch_execute("DELETE FROM gl_token_audit").is_err());
        assert!(db.batch_execute("DELETE FROM gl_token_receipts").is_err());
        if let Ok(bin) = std::env::var("GL_TEST_PG_BIN") {
            let temp = tempfile::tempdir()?;
            let backup = temp.path().join("backup.dump");
            let status = std::process::Command::new(std::path::Path::new(&bin).join("pg_dump"))
                .args([
                    "--format=custom",
                    "--no-owner",
                    "--no-acl",
                    "--dbname",
                    &test_dsn,
                    "--file",
                ])
                .arg(&backup)
                .status()?;
            assert!(status.success());
            let restored_name = format!("geoledger_tokens_restore_{}", Uuid::new_v4().simple());
            admin.batch_execute(&format!("CREATE DATABASE {restored_name}"))?;
            let restore_dsn = dsn.replacen("/geoledger_test", &format!("/{restored_name}"), 1);
            let restored_result = (|| -> TestResult {
                assert!(
                    std::process::Command::new(std::path::Path::new(&bin).join("pg_restore"))
                        .args([
                            "--exit-on-error",
                            "--no-owner",
                            "--no-acl",
                            "--dbname",
                            &restore_dsn
                        ])
                        .arg(backup)
                        .status()?
                        .success()
                );
                let restored = app(Storage::Postgis(restore_dsn))?;
                assert_eq!(
                    restored
                        .authenticate_managed_token(active["token"].as_str().ok_or("active")?)?
                        .as_deref(),
                    Some("backup")
                );
                assert!(
                    restored
                        .authenticate_managed_token(revoked["token"].as_str().ok_or("revoked")?)?
                        .is_none()
                );
                assert!(
                    restored
                        .authenticate_managed_token(expired["token"].as_str().ok_or("expired")?)?
                        .is_none()
                );
                assert_eq!(
                    restored.execute("admin", "list_tokens", json!({}))?,
                    instance.execute("admin", "list_tokens", json!({}))?
                );
                Ok(())
            })();
            admin.batch_execute(&format!("DROP DATABASE {restored_name} WITH (FORCE)"))?;
            restored_result?;
        }
        Ok(())
    })();
    admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))?;
    result
}

#[test]
fn rejects_either_incompatible_format_twelve_without_modifying_it() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("old.db");
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch("CREATE TABLE gl_format(singleton boolean PRIMARY KEY, version integer NOT NULL); INSERT INTO gl_format VALUES(true,12);")?;
    let app = Application::new(Storage::Sqlite(path));
    assert!(app.migrate().is_err());
    assert_eq!(
        connection.query_row("SELECT version FROM gl_format", [], |row| row
            .get::<_, i32>(0))?,
        12
    );
    assert_eq!(connection.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('gl_managed_tokens','gl_workspace_sizes')", [], |row| row.get::<_, i32>(0))?, 0);
    Ok(())
}
