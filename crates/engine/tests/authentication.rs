use geoledger_engine::{Application, AuthenticatedIdentity, Storage};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Barrier},
    thread,
};
type TestResult = Result<(), Box<dyn std::error::Error>>;
const INITIAL: &str = "initial-test-password";
const PASSWORD: &str = "permanent-test-password";
struct Fixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    app: Application,
    admin: AuthenticatedIdentity,
    session: String,
}
impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("auth.db");
        let app = Application::new(Storage::Sqlite(path.clone()));
        app.migrate()?;
        app.bootstrap_admin("Admin", INITIAL)?;
        let first = app.auth_operation(
            None,
            "login_password",
            json!({"username":" admin ","password":INITIAL}),
        )?;
        let identity = app.authenticate_session(first["session"].as_str().ok_or("session")?)?;
        let changed = app.auth_operation(
            Some(&identity),
            "change_password",
            json!({"current_password":INITIAL,"new_password":PASSWORD}),
        )?;
        let session = changed["session"].as_str().ok_or("session")?.to_owned();
        let admin = app.authenticate_session(&session)?;
        Ok(Self {
            _dir: dir,
            path,
            app,
            admin,
            session,
        })
    }
    fn call(&self, op: &str, mut input: Value) -> geoledger_engine::Result<Value> {
        input["current_password"] = json!(PASSWORD);
        self.app.auth_operation(Some(&self.admin), op, input)
    }
    fn user(
        &self,
        name: &str,
        admin: bool,
    ) -> Result<(AuthenticatedIdentity, String), Box<dyn std::error::Error>> {
        self.call(
            "create_user",
            json!({"username":name,"password":INITIAL,"is_admin":admin}),
        )?;
        let first = self.app.auth_operation(
            None,
            "login_password",
            json!({"username":name,"password":INITIAL}),
        )?;
        let identity = self
            .app
            .authenticate_session(first["session"].as_str().ok_or("session")?)?;
        let changed = self.app.auth_operation(
            Some(&identity),
            "change_password",
            json!({"current_password":INITIAL,"new_password":PASSWORD}),
        )?;
        let session = changed["session"].as_str().ok_or("session")?.to_owned();
        Ok((self.app.authenticate_session(&session)?, session))
    }
}
#[test]
fn bootstrap_password_and_identity_constraints() -> TestResult {
    let f = Fixture::new()?;
    assert!(!f.app.bootstrap_admin("different", "different-password")?);
    assert!(!f.app.bootstrap_needed()?);
    let db = rusqlite::Connection::open(&f.path)?;
    let hash: String = db.query_row("SELECT password_hash FROM gl_passwords", [], |r| r.get(0))?;
    assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert!(!serde_json::to_string(&f.admin.user)?.contains(&hash));
    assert_eq!(f.admin.user.username, "admin");
    let uid = f.admin.user.id.clone();
    f.call(
        "update_user",
        json!({"user_id":uid,"username":"Renamed","display_name":"New name"}),
    )?;
    assert_eq!(f.app.authenticate_session(&f.session)?.user.id, uid);
    assert_eq!(
        f.app.authenticate_session(&f.session)?.user.username,
        "renamed"
    );
    assert!(
        f.app
            .auth_operation(
                None,
                "login_password",
                json!({"username":"admin","password":PASSWORD})
            )
            .is_err()
    );
    assert!(
        f.app
            .auth_operation(
                None,
                "login_password",
                json!({"username":"renamed","password":PASSWORD})
            )
            .is_ok()
    );
    assert_eq!(
        f.call(
            "create_user",
            json!({"username":"RENAMED","password":INITIAL})
        )
        .err()
        .ok_or("expected rejection")?
        .status,
        409
    );
    assert_eq!(
        f.call("update_user", json!({"user_id":uid,"is_admin":false}))
            .err()
            .ok_or("expected rejection")?
            .status,
        409
    );
    assert_eq!(
        f.call("update_user", json!({"user_id":uid,"status":"disabled"}))
            .err()
            .ok_or("expected rejection")?
            .status,
        409
    );
    assert_eq!(
        f.call("update_user", json!({"user_id":uid,"status":"deleted"}))
            .err()
            .ok_or("expected rejection")?
            .status,
        409
    );
    Ok(())
}
#[test]
fn initial_password_blocks_business_and_keys_and_login_is_uniform() -> TestResult {
    let f = Fixture::new()?;
    let user = f.call(
        "create_user",
        json!({"username":"newuser","password":INITIAL}),
    )?;
    let login = f.app.auth_operation(
        None,
        "login_password",
        json!({"username":"newuser","password":INITIAL}),
    )?;
    let identity = f
        .app
        .authenticate_session(login["session"].as_str().ok_or("session")?)?;
    assert!(identity.user.must_change_password);
    assert_eq!(
        f.app
            .execute_authenticated(&identity, "list_projects", json!({}))
            .err()
            .ok_or("expected rejection")?
            .status,
        403
    );
    assert_eq!(
        f.app
            .auth_operation(
                Some(&identity),
                "create_api_key",
                json!({"name":"x","current_password":INITIAL})
            )
            .err()
            .ok_or("expected rejection")?
            .status,
        403
    );
    let first = f
        .app
        .auth_operation(
            None,
            "login_password",
            json!({"username":"newuser","password":"incorrect-password"}),
        )
        .err()
        .ok_or("expected rejection")?;
    let absent = f
        .app
        .auth_operation(
            None,
            "login_password",
            json!({"username":"nonexistent","password":"incorrect-password"}),
        )
        .err()
        .ok_or("expected rejection")?;
    f.call(
        "update_user",
        json!({"user_id":user["id"],"status":"disabled"}),
    )?;
    let disabled = f
        .app
        .auth_operation(
            None,
            "login_password",
            json!({"username":"newuser","password":INITIAL}),
        )
        .err()
        .ok_or("expected rejection")?;
    for error in [absent, disabled] {
        assert_eq!(error.status, first.status);
        assert_eq!(
            error.body["error"]["message"],
            first.body["error"]["message"]
        );
    }
    assert!(
        f.app
            .authenticate_session(login["session"].as_str().ok_or("session")?)
            .is_err()
    );
    Ok(())
}
#[test]
fn key_rotation_scopes_source_session_expiry_and_secret_storage() -> TestResult {
    let f = Fixture::new()?;
    let (user, _) = f.user("alice", false)?;
    let (other, _) = f.user("other", false)?;
    let key = f.app.auth_operation(
        Some(&user),
        "create_api_key",
        json!({"name":"automation","current_password":PASSWORD}),
    )?;
    let secret = key["api_key"].as_str().ok_or("key")?;
    assert!(secret.starts_with("glk_"));
    assert_eq!(secret.len(), 68);
    let another = f.app.auth_operation(
        Some(&user),
        "create_api_key",
        json!({"name":"rotation","current_password":PASSWORD}),
    )?;
    assert!(f.app.authenticate_api_key(secret).is_ok());
    assert!(
        f.app
            .authenticate_api_key(another["api_key"].as_str().ok_or("key")?)
            .is_ok()
    );
    assert_eq!(
        f.app
            .auth_operation(
                Some(&other),
                "revoke_api_key",
                json!({"key_id":key["credential"]["id"],"current_password":PASSWORD})
            )
            .err()
            .ok_or("expected rejection")?
            .status,
        403
    );
    let login = f
        .app
        .auth_operation(None, "login_api_key", json!({"api_key":secret}))?;
    let session = login["session"].as_str().ok_or("session")?;
    let via_key = f.app.authenticate_session(session)?;
    assert_eq!(
        f.app
            .auth_operation(
                Some(&via_key),
                "create_api_key",
                json!({"name":"escape","current_password":"wrong-password"})
            )
            .err()
            .ok_or("expected rejection")?
            .status,
        403
    );
    f.app.auth_operation(Some(&user),"update_api_key",json!({"key_id":key["credential"]["id"],"name":"renamed","expires_at":"2099-01-01T00:00:00Z","current_password":PASSWORD}))?;
    let listed = f
        .app
        .auth_operation(Some(&user), "list_api_keys", json!({}))?;
    assert!(!listed.to_string().contains(secret));
    assert!(!format!("{:?}", via_key).contains(secret));
    f.app.auth_operation(
        Some(&user),
        "revoke_api_key",
        json!({"key_id":key["credential"]["id"],"current_password":PASSWORD}),
    )?;
    assert!(f.app.authenticate_api_key(secret).is_err());
    assert!(f.app.authenticate_session(session).is_err());
    let db = rusqlite::Connection::open(&f.path)?;
    db.execute(
        "UPDATE gl_api_keys SET created_at=?1,expires_at=?2 WHERE id=?3",
        [
            "2000-01-01T00:00:00.000Z",
            "2001-01-01T00:00:00.000Z",
            another["credential"]["id"].as_str().ok_or("key id")?,
        ],
    )?;
    assert!(
        f.app
            .authenticate_api_key(another["api_key"].as_str().ok_or("key")?)
            .is_err()
    );
    f.app.auth_operation(
        Some(&user),
        "delete_api_key",
        json!({"key_id":key["credential"]["id"],"current_password":PASSWORD}),
    )?;
    let raw = std::fs::read(&f.path)?;
    assert!(!raw.windows(secret.len()).any(|s| s == secret.as_bytes()));
    assert!(
        !raw.windows(PASSWORD.len())
            .any(|s| s == PASSWORD.as_bytes())
    );
    assert!(db.execute("DELETE FROM gl_auth_audit", []).is_err());
    Ok(())
}
#[test]
fn sessions_reset_logout_soft_delete_and_audit_atomicity() -> TestResult {
    let f = Fixture::new()?;
    let (user, session) = f.user("alice", false)?;
    let second = Application::new(Storage::Sqlite(f.path.clone()));
    let login = f.app.auth_operation(
        None,
        "login_password",
        json!({"username":"alice","password":PASSWORD}),
    )?;
    let other = login["session"].as_str().ok_or("session")?;
    assert_ne!(session, other);
    f.app.auth_operation(Some(&user), "logout", json!({}))?;
    assert!(second.authenticate_session(&session).is_err());
    assert!(second.authenticate_session(other).is_ok());
    f.call(
        "reset_password",
        json!({"user_id":user.user.id,"new_password":"reset-test-password"}),
    )?;
    assert!(second.authenticate_session(other).is_err());
    let reset = f.app.auth_operation(
        None,
        "login_password",
        json!({"username":"alice","password":"reset-test-password"}),
    )?;
    assert_eq!(reset["user"]["must_change_password"], true);
    f.call(
        "update_user",
        json!({"user_id":user.user.id,"status":"deleted"}),
    )?;
    assert!(
        second
            .authenticate_session(reset["session"].as_str().ok_or("session")?)
            .is_err()
    );
    let db = rusqlite::Connection::open(&f.path)?;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM gl_users WHERE id=?1 AND status=\u{27}deleted\u{27}",
            [&user.user.id],
            |r| r.get::<_, i64>(0)
        )?,
        1
    );
    db.execute_batch("CREATE TRIGGER fail_auth_audit BEFORE INSERT ON gl_auth_audit BEGIN SELECT RAISE(ABORT,\u{27}injected test failure\u{27}); END;")?;
    assert!(
        f.call("create_api_key", json!({"name":"rollback"}))
            .is_err()
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM gl_api_keys WHERE name=\u{27}rollback\u{27}",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    Ok(())
}
#[test]
fn session_idle_absolute_expiry_and_source_deletion() -> TestResult {
    let f = Fixture::new()?;
    let (user, session) = f.user("alice", false)?;
    let db = rusqlite::Connection::open(&f.path)?;
    db.execute(
        "UPDATE gl_sessions SET last_seen_at=?1 WHERE user_id=?2",
        ["2000-01-01T00:00:00.000Z", &user.user.id],
    )?;
    assert!(f.app.authenticate_session(&session).is_err());
    let key = f.call(
        "create_api_key",
        json!({"user_id":user.user.id,"name":"source"}),
    )?;
    let login = f
        .app
        .auth_operation(None, "login_api_key", json!({"api_key":key["api_key"]}))?;
    assert!(
        f.app
            .authenticate_session(login["session"].as_str().ok_or("session")?)
            .is_ok()
    );
    f.call("delete_api_key", json!({"key_id":key["credential"]["id"]}))?;
    assert!(
        f.app
            .authenticate_session(login["session"].as_str().ok_or("session")?)
            .is_err()
    );
    db.execute(
        "UPDATE gl_sessions SET expires_at=?1 WHERE user_id=?2",
        ["2000-01-01T00:00:00.000Z", &f.admin.user.id],
    )?;
    assert!(f.app.authenticate_session(&f.session).is_err());
    Ok(())
}
#[test]
fn concurrent_bootstrap_and_last_admin_are_serialized() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("bootstrap.db");
    let app = Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    let barrier = Arc::new(Barrier::new(4));
    let mut tasks = vec![];
    for _ in 0..4 {
        let app = Application::new(Storage::Sqlite(path.clone()));
        let b = barrier.clone();
        tasks.push(thread::spawn(move || {
            b.wait();
            app.bootstrap_admin("admin", INITIAL)
        }));
    }
    let mut created = 0;
    for task in tasks {
        if task.join().map_err(|_| "thread")?? {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    let f = Fixture::new()?;
    let (second, _) = f.user("second", true)?;
    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = vec![];
    for actor in [f.admin.clone(), second] {
        let app = Application::new(Storage::Sqlite(f.path.clone()));
        let b = barrier.clone();
        tasks.push(thread::spawn(move || {
            b.wait();
            app.auth_operation(
                Some(&actor),
                "update_user",
                json!({"user_id":actor.user.id,"is_admin":false,"current_password":PASSWORD}),
            )
        }));
    }
    let results = tasks
        .into_iter()
        .map(|t| t.join().map_err(|_| "thread"))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_ref().is_err_and(|e| e.status == 409))
            .count(),
        1
    );
    Ok(())
}
#[test]
fn backup_preserves_auth_and_logical_export_excludes_secrets() -> TestResult {
    let f = Fixture::new()?;
    let key = f.call("create_api_key", json!({"name":"backup"}))?;
    let secret = key["api_key"].as_str().ok_or("key")?;
    let mut export = Vec::new();
    f.app.export_data(&mut export)?;
    assert!(!String::from_utf8_lossy(&export).contains("gl_api_keys"));
    assert!(!String::from_utf8_lossy(&export).contains(secret));
    let backup = f._dir.path().join("backup.db");
    f.app.backup(&backup)?;
    let restored = Application::new(Storage::Sqlite(backup));
    assert!(restored.authenticate_api_key(secret).is_ok());
    assert!(restored.authenticate_session(&f.session).is_ok());
    Ok(())
}

#[test]
fn expired_session_cleanup_uses_index_and_deletes_a_bounded_batch() -> TestResult {
    let f = Fixture::new()?;
    let key = f.call("create_api_key", json!({"name":"cleanup"}))?;
    let mut db = rusqlite::Connection::open(&f.path)?;
    let transaction = db.transaction()?;
    for _ in 0..250 {
        let id = uuid::Uuid::new_v4().to_string();
        transaction.execute("INSERT INTO gl_sessions(id,user_id,digest,credential_version,created_at,last_seen_at,expires_at) VALUES(?1,?2,?1,0,'2000-01-01T00:00:00.000Z','2000-01-01T00:00:00.000Z','2001-01-01T00:00:00.000Z')",[&id,&f.admin.user.id])?;
    }
    transaction.commit()?;
    let plan:String=db.query_row("EXPLAIN QUERY PLAN SELECT id FROM gl_sessions WHERE expires_at<=?1 ORDER BY expires_at LIMIT 100",["2002-01-01T00:00:00.000Z"],|r|r.get(3))?;
    assert!(plan.contains("gl_sessions_expiry"));
    for expected in [150, 50, 0] {
        f.app
            .auth_operation(None, "login_api_key", json!({"api_key":key["api_key"]}))?;
        let count: i64 = db.query_row(
            "SELECT count(*) FROM gl_sessions WHERE expires_at<'2002-01-01T00:00:00.000Z'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(count, expected);
    }
    assert!(f.app.authenticate_session(&f.session).is_ok());
    Ok(())
}

#[test]
fn trusted_business_identity_membership_password_and_history_survive_rename() -> TestResult {
    let f = Fixture::new()?;
    let (alice, _) = f.user("alice", false)?;
    let (bob, _) = f.user("bob", false)?;
    let project = f.app.execute_authenticated(
        &alice,
        "create_project",
        json!({"name":"stable identity"}),
    )?["project"]
        .clone();
    let member = json!({"project":project,"subject":bob.user.id,"role":"editor"});
    assert_eq!(
        f.app
            .execute_authenticated(&alice, "set_member", member.clone())
            .err()
            .ok_or("password required")?
            .status,
        403
    );
    let mut verified = member.clone();
    verified["current_password"] = json!(PASSWORD);
    f.app
        .execute_authenticated(&alice, "set_member", verified)?;
    let key = f.app.auth_operation(
        Some(&alice),
        "create_api_key",
        json!({"name":"automation","current_password":PASSWORD}),
    )?;
    let api = f
        .app
        .authenticate_api_key(key["api_key"].as_str().ok_or("key")?)?;
    f.app.execute_authenticated(&api, "set_member", member)?;
    assert!(
        f.app
            .execute_authenticated(
                &api,
                "set_member",
                json!({"project":project,"subject":"bob","role":"viewer"})
            )
            .is_err()
    );
    let dataset = f.app.execute_authenticated(
        &alice,
        "create_dataset",
        json!({"project":project,"name":"points","geometry_type":"point"}),
    )?["dataset"]
        .clone();
    let workspace = f.app.execute_authenticated(
        &alice,
        "create_workspace",
        json!({"project":project}),
    )?["workspace"]
        .clone();
    f.app.execute_authenticated(&alice,"save",json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"edits":[{"dataset":dataset,"feature_id":"one","feature":{"type":"Feature","id":"one","properties":{"value":18446744073709551615u64},"geometry":{"type":"Point","coordinates":[1,2]}}}]}))?;
    f.app.execute_authenticated(&alice,"publish",json!({"project":project,"workspace":workspace,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"identity test"}))?;
    f.app.auth_operation(
        Some(&alice),
        "update_user",
        json!({"user_id":alice.user.id,"username":"alice-renamed","current_password":PASSWORD}),
    )?;
    let history = f
        .app
        .execute_authenticated(&api, "history", json!({"project":project}))?;
    assert!(history.to_string().contains(&alice.user.id));
    let data = f.app.execute_authenticated(
        &api,
        "features",
        json!({"project":project,"dataset":dataset}),
    )?;
    assert_eq!(
        data["features"][0]["properties"]["value"],
        json!(18446744073709551615u64)
    );
    let audit = f
        .app
        .execute_authenticated(&api, "audit", json!({"project":project}))?;
    assert!(audit.to_string().contains(&alice.user.id));
    f.call(
        "update_user",
        json!({"user_id":alice.user.id,"status":"disabled"}),
    )?;
    assert!(
        f.app
            .execute_authenticated(&api, "list_projects", json!({}))
            .is_err()
    );
    f.call(
        "update_user",
        json!({"user_id":alice.user.id,"status":"active"}),
    )?;
    assert!(
        f.app
            .authenticate_api_key(key["api_key"].as_str().ok_or("key")?)
            .is_err()
    );
    Ok(())
}

#[test]
fn committed_revocation_is_observed_by_concurrent_instances() -> TestResult {
    let f = Fixture::new()?;
    let key = f.call("create_api_key", json!({"name":"concurrent"}))?;
    let secret = key["api_key"].as_str().ok_or("key")?.to_owned();
    let barrier = Arc::new(Barrier::new(5));
    let mut tasks = vec![];
    for _ in 0..4 {
        let app = Application::new(Storage::Sqlite(f.path.clone()));
        let secret = secret.clone();
        let b = barrier.clone();
        tasks.push(thread::spawn(move || {
            let first = app.authenticate_api_key(&secret).is_ok();
            b.wait();
            b.wait();
            (first, app.authenticate_api_key(&secret).is_err())
        }));
    }
    barrier.wait();
    f.call("revoke_api_key", json!({"key_id":key["credential"]["id"]}))?;
    barrier.wait();
    for task in tasks {
        assert_eq!(task.join().map_err(|_| "thread")?, (true, true));
    }
    Ok(())
}

mod support;
#[test]
#[ignore = "requires dedicated geoledger_test database; run with --test-threads=1"]
fn postgis_source_session_touch_and_key_delete_have_one_lock_order() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let (target, mut db) = support::connect_isolated(&dsn, "geoledger_auth_locks")?;
    support::reset_database(&mut db)?;
    db.batch_execute("CREATE EXTENSION IF NOT EXISTS postgis")?;
    let result = (|| -> TestResult {
        let app = Application::new(Storage::Postgis(target.clone()));
        app.migrate()?;
        app.bootstrap_admin("admin", INITIAL)?;
        let first = app.auth_operation(
            None,
            "login_password",
            json!({"username":"admin","password":INITIAL}),
        )?;
        let initial = app.authenticate_session(first["session"].as_str().ok_or("session")?)?;
        let login = app.auth_operation(
            Some(&initial),
            "change_password",
            json!({"current_password":INITIAL,"new_password":PASSWORD}),
        )?;
        let admin = app.authenticate_session(login["session"].as_str().ok_or("session")?)?;
        let key = app.auth_operation(
            Some(&admin),
            "create_api_key",
            json!({"name":"lock-order","current_password":PASSWORD}),
        )?;
        let key_id = key["credential"]["id"].as_str().ok_or("key id")?;
        let login = app.auth_operation(None, "login_api_key", json!({"api_key":key["api_key"]}))?;
        let session = login["session"].as_str().ok_or("session")?.to_owned();
        let stamp = (chrono::Utc::now() - chrono::Duration::minutes(6))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        db.execute(
            "UPDATE gl_api_keys SET last_used_at=$1 WHERE id=$2",
            &[&stamp, &key_id],
        )?;
        db.execute(
            "UPDATE gl_sessions SET last_seen_at=$1 WHERE source_key_id=$2",
            &[&stamp, &key_id],
        )?;
        let tag = format!("auth_lock_{}", uuid::Uuid::new_v4().simple());
        let separator = if target.starts_with("postgres://") || target.starts_with("postgresql://")
        {
            if target.contains('?') { "&" } else { "?" }
        } else {
            " "
        };
        let client = Application::new(Storage::Postgis(format!(
            "{target}{separator}application_name={tag}"
        )))
        .with_timeout(std::time::Duration::from_secs(5));
        let mut monitor = postgres::Client::connect(&target, postgres::NoTls)?;
        let mut transaction = db.transaction()?;
        transaction.batch_execute("SET LOCAL lock_timeout='2s'")?;
        transaction.query_one(
            "SELECT id FROM gl_api_keys WHERE id=$1 FOR UPDATE",
            &[&key_id],
        )?;
        let worker = thread::spawn(move || client.authenticate_session(&session));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut waiting = false;
        while std::time::Instant::now() < deadline {
            if monitor.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock'",&[&tag])?.get::<_,i64>(0)>0 {waiting=true;break;}
            thread::sleep(std::time::Duration::from_millis(10));
        }
        let deletion = transaction.execute("DELETE FROM gl_api_keys WHERE id=$1", &[&key_id]);
        if deletion.is_ok() {
            transaction.commit()?;
        } else {
            transaction.rollback()?;
        }
        let authentication = worker.join().map_err(|_| "authentication thread failed")?;
        assert!(
            waiting,
            "authentication did not reach the controlled key lock"
        );
        assert!(
            deletion.is_ok(),
            "key deletion deadlocked with session activity"
        );
        assert!(
            authentication.is_ok()
                || authentication
                    .as_ref()
                    .is_err_and(|error| error.status == 401),
            "authentication failed due to lock inversion"
        );
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

#[test]
#[ignore = "requires dedicated geoledger_test database; run with --test-threads=1"]
fn postgis_authentication_parity_and_atomicity() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let (target, mut db) = support::connect_isolated(&dsn, "geoledger_authentication")?;
    support::reset_database(&mut db)?;
    db.batch_execute("CREATE EXTENSION IF NOT EXISTS postgis")?;
    let result = (|| -> TestResult {
        let app = Application::new(Storage::Postgis(target.clone()));
        app.migrate()?;
        let barrier = Arc::new(Barrier::new(4));
        let mut threads = vec![];
        for _ in 0..4 {
            let app = Application::new(Storage::Postgis(target.clone()));
            let b = barrier.clone();
            threads.push(thread::spawn(move || {
                b.wait();
                app.bootstrap_admin("admin", INITIAL)
            }));
        }
        let mut created = 0;
        for task in threads {
            if task.join().map_err(|_| "thread")?? {
                created += 1;
            }
        }
        assert_eq!(created, 1);
        let first = app.auth_operation(
            None,
            "login_password",
            json!({"username":"admin","password":INITIAL}),
        )?;
        let initial = app.authenticate_session(first["session"].as_str().ok_or("session")?)?;
        let login = app.auth_operation(
            Some(&initial),
            "change_password",
            json!({"current_password":INITIAL,"new_password":PASSWORD}),
        )?;
        let admin = app.authenticate_session(login["session"].as_str().ok_or("session")?)?;
        let call = |op: &str, mut input: Value| {
            input["current_password"] = json!(PASSWORD);
            app.auth_operation(Some(&admin), op, input)
        };
        let key = call("create_api_key", json!({"name":"postgis"}))?;
        let secret = key["api_key"].as_str().ok_or("key")?;
        let other = Application::new(Storage::Postgis(target.clone()));
        assert_eq!(other.authenticate_api_key(secret)?.user.id, admin.user.id);
        let key_login = app.auth_operation(None, "login_api_key", json!({"api_key":secret}))?;
        let session = key_login["session"].as_str().ok_or("session")?;
        assert!(other.authenticate_session(session).is_ok());
        call("revoke_api_key", json!({"key_id":key["credential"]["id"]}))?;
        assert!(other.authenticate_api_key(secret).is_err());
        assert!(other.authenticate_session(session).is_err());
        assert_eq!(
            call(
                "update_user",
                json!({"user_id":admin.user.id,"status":"deleted"})
            )
            .err()
            .ok_or("expected rejection")?
            .status,
            409
        );
        db.batch_execute("CREATE FUNCTION fail_auth_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected test failure'; END $$; CREATE TRIGGER fail_auth_audit BEFORE INSERT ON gl_auth_audit FOR EACH ROW EXECUTE FUNCTION fail_auth_audit();")?;
        assert!(call("create_api_key", json!({"name":"rollback"})).is_err());
        assert_eq!(
            db.query_one(
                "SELECT count(*) FROM gl_api_keys WHERE name='rollback'",
                &[]
            )?
            .get::<_, i64>(0),
            0
        );
        assert!(db.batch_execute("DELETE FROM gl_auth_audit").is_err());
        db.batch_execute(
            "DROP TRIGGER fail_auth_audit ON gl_auth_audit; DROP FUNCTION fail_auth_audit();",
        )?;
        call(
            "create_user",
            json!({"username":"second","password":INITIAL,"is_admin":true}),
        )?;
        let second_login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"second","password":INITIAL}),
        )?;
        let second =
            app.authenticate_session(second_login["session"].as_str().ok_or("session")?)?;
        let changed = app.auth_operation(
            Some(&second),
            "change_password",
            json!({"current_password":INITIAL,"new_password":PASSWORD}),
        )?;
        let second = app.authenticate_session(changed["session"].as_str().ok_or("session")?)?;
        let barrier = Arc::new(Barrier::new(2));
        let mut tasks = vec![];
        for actor in [admin.clone(), second] {
            let app = Application::new(Storage::Postgis(target.clone()));
            let b = barrier.clone();
            tasks.push(thread::spawn(move || {
                b.wait();
                app.auth_operation(
                    Some(&actor),
                    "update_user",
                    json!({"user_id":actor.user.id,"is_admin":false,"current_password":PASSWORD}),
                )
            }));
        }
        let results = tasks
            .into_iter()
            .map(|t| t.join().map_err(|_| "thread"))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| r.as_ref().is_err_and(|e| e.status == 409))
                .count(),
            1
        );
        Ok(())
    })();
    support::reset_database(&mut db)?;
    result
}

#[test]
fn auth_response_failure_rolls_back_credentials_and_audit() -> TestResult {
    let f = Fixture::new()?;
    let db = rusqlite::Connection::open(&f.path)?;
    let before: i64 = db.query_row("SELECT count(*) FROM gl_auth_audit", [], |r| r.get(0))?;
    for app in [
        f.app.clone().with_response_limit(1),
        f.app.clone().with_response_reservation(Arc::new(|_| {
            Err(geoledger_engine::Error::new(429, "test budget exhausted"))
        })),
    ] {
        let error = app
            .auth_operation(
                Some(&f.admin),
                "create_api_key",
                json!({"name":"rollback","current_password":PASSWORD}),
            )
            .err()
            .ok_or("response must fail before commit")?;
        assert!([413, 429].contains(&error.status));
        assert_eq!(
            db.query_row("SELECT count(*) FROM gl_api_keys", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM gl_auth_audit", [], |r| r
                .get::<_, i64>(0))?,
            before
        );
    }
    Ok(())
}

#[test]
fn user_listing_pages_public_fields_and_uses_status_index() -> TestResult {
    let f = Fixture::new()?;
    let db = rusqlite::Connection::open(&f.path)?;
    for (id, status) in [("u1", "disabled"), ("u2", "active"), ("u3", "disabled")] {
        db.execute("INSERT INTO gl_users(id,username,display_name,status,is_admin,must_change_password,created_at) VALUES(?1,?1,?1,?2,false,false,'2026-01-01T00:00:00Z')", rusqlite::params![id,status])?;
    }
    let first = f.call("list_users", json!({"status":"disabled","limit":1}))?;
    assert_eq!(first[0]["id"], "u1");
    let next = f.call(
        "list_users",
        json!({"status":"disabled","after":"u1","limit":1}),
    )?;
    assert_eq!(next[0]["id"], "u3");
    let all = f.call("list_users", json!({"after":"u1"}))?;
    assert_eq!(all.as_array().ok_or("users")?.len(), 2);
    assert_eq!(all[0]["id"], "u2");
    assert_eq!(all[0].as_object().ok_or("user")?.len(), 7);
    let plan: String = db.query_row("EXPLAIN QUERY PLAN SELECT id,username,display_name,status,is_admin,must_change_password,created_at FROM gl_users WHERE status=?3 AND id>?1 ORDER BY id LIMIT ?2", rusqlite::params!["",1,"disabled"], |r| r.get(3))?;
    assert!(plan.contains("gl_users_status_id"), "{plan}");
    Ok(())
}
