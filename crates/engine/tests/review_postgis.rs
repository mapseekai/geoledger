//! Controlled PostgreSQL failure/concurrency regressions. Never run against a business database.
use geoledger_engine::{Application, Storage, StorageOptions};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use uuid::Uuid;
type BoxError = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), BoxError>;
struct Fixture {
    admin: postgres::Client,
    monitor: postgres::Client,
    app: Application,
    project: String,
    owner: String,
    tag: String,
    triggers: Vec<String>,
}
impl Fixture {
    fn new(options: StorageOptions) -> Result<Self, BoxError> {
        let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
        let mut admin = postgres::Client::connect(&dsn, postgres::NoTls)?;
        let database: String = admin.query_one("SELECT current_database()", &[])?.get(0);
        if database != "geoledger_test" {
            return Err("requires isolated geoledger_test".into());
        }
        // Observe waits outside the lock-barrier transaction and its statistics snapshot.
        let monitor = postgres::Client::connect(&dsn, postgres::NoTls)?;
        let tag = format!("review_{}", Uuid::new_v4().simple());
        let target = format!(
            "{}{}application_name={tag}",
            dsn,
            if dsn.starts_with("postgres://") || dsn.starts_with("postgresql://") {
                if dsn.contains('?') { "&" } else { "?" }
            } else {
                " "
            }
        );
        let app = Application::with_options(Storage::Postgis(target), options);
        app.migrate()?;
        let owner = Uuid::new_v4().to_string();
        let project = app.execute(&owner, "create_project", json!({"name":"review"}))?["project"]
            .as_str()
            .ok_or("project")?
            .to_owned();
        Ok(Self {
            admin,
            monitor,
            app,
            project,
            owner,
            tag,
            triggers: Vec::new(),
        })
    }
    fn call(&self, op: &str, mut input: Value) -> geoledger_engine::Result<Value> {
        input["project"] = json!(self.project);
        self.app.execute(&self.owner, op, input)
    }
    fn pause_updates(&mut self) -> TestResult {
        let name = format!("gl_review_{}", Uuid::new_v4().simple());
        self.admin.batch_execute(&format!("CREATE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(2); RETURN NEW; END $$; CREATE TRIGGER {name} BEFORE UPDATE ON gl_projects FOR EACH ROW WHEN (OLD.id='{}') EXECUTE FUNCTION {name}()",self.project))?;
        self.triggers.push(name);
        Ok(())
    }
    fn wait_event(&mut self, event: &str, count: i64) -> TestResult {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let n:i64=self.monitor.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND (wait_event_type=$2 OR wait_event=$2)",&[&self.tag,&event])?.get(0);
            if n >= count {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("did not observe {count} {event} waiters").into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn drained(&self) -> TestResult {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.app.pool_stats().ok_or("pool")?.active != 0 {
            if Instant::now() >= deadline {
                return Err("pool cleanup not confirmed".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
    fn backend(&mut self) -> Result<i32, BoxError> {
        Ok(self.monitor.query_one("SELECT pid FROM pg_stat_activity WHERE application_name=$1 ORDER BY backend_start LIMIT 1",&[&self.tag])?.get(0))
    }
    fn idle_backend(&mut self, expected: i32) -> TestResult {
        let stats = self.app.pool_stats().ok_or("pool")?;
        assert_eq!((stats.active, stats.idle, stats.retiring), (0, 1, 0));
        let rows = self.monitor.query(
            "SELECT pid,state,xact_start IS NULL,backend_xid IS NULL FROM pg_stat_activity WHERE application_name=$1",
            &[&self.tag],
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<_, i32>(0), expected);
        assert_eq!(rows[0].get::<_, String>(1), "idle");
        assert!(rows[0].get::<_, bool>(2));
        assert!(rows[0].get::<_, bool>(3));
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.admin.batch_execute("ROLLBACK");
        for name in &self.triggers {
            let _ = self.admin.batch_execute(&format!(
                "DROP TRIGGER IF EXISTS {name} ON gl_projects; DROP FUNCTION IF EXISTS {name}()"
            ));
        }
    }
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn expected_404s_reuse_connection_without_leaking_transaction_or_partial_save() -> TestResult {
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        ..StorageOptions::default()
    })?;
    let dataset = f.call(
        "create_dataset",
        json!({"name":"reuse","geometry_type":"point"}),
    )?["dataset"]
        .clone();
    let feature = json!({"type":"Feature","id":"deleted","properties":{"label":"seed"},"geometry":{"type":"Point","coordinates":[1,2]}});
    for value in [feature, Value::Null] {
        let workspace = f.call("create_workspace", json!({}))?["workspace"].clone();
        f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"edits":[{"dataset":dataset,"feature_id":"deleted","feature":value}]}))?;
        f.call("publish", json!({"workspace":workspace,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"404 reuse fixture"}))?;
    }
    let pid = f.backend()?;
    for _ in 0..20 {
        let error = f
            .call(
                "features",
                json!({"dataset":dataset,"feature_id":"deleted"}),
            )
            .err()
            .ok_or("deleted feature must return 404")?;
        assert_eq!(error.status, 404, "{error:?}");
        f.idle_backend(pid)?;
    }
    let workspace = f.call("create_workspace", json!({}))?["workspace"].clone();
    let mut edits: Vec<_> = (0..32)
        .map(|i| {
            let id = format!("partial-{i}");
            json!({"dataset":dataset,"feature_id":id,"feature":{"type":"Feature","id":id,"properties":{},"geometry":null}})
        })
        .collect();
    // The first 32 drafts are flushed before this missing dataset returns 404.
    edits.push(json!({"dataset":Uuid::new_v4(),"feature_id":"missing","feature":null}));
    assert_eq!(
        f.call(
            "save",
            json!({"workspace":workspace,"expected_workspace_version":0,"edits":edits})
        )
        .err()
        .ok_or("missing dataset must return 404")?
        .status,
        404
    );
    f.idle_backend(pid)?;
    assert_eq!(
        f.call("get_workspace", json!({"workspace":workspace}))?["version"],
        0
    );
    assert_eq!(
        f.call("diff", json!({"workspace":workspace}))?["changes"],
        json!([])
    );
    f.call("rename_project", json!({"name":"after rollback"}))?;
    assert_eq!(f.call("get_project", json!({}))?["name"], "after rollback");
    f.idle_backend(pid)?;
    eprintln!(
        "expected_404s=20 partial_save_404s=1 flushed_drafts_rolled_back=32 backend_pid={pid} distinct_backends=1 idle=1 active=0 retiring=0 residual_transactions=0 subsequent_read_write=ok"
    );
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn deadlines_cancel_database_work_and_hold_pool_slot_until_cleanup() -> TestResult {
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        ..StorageOptions::default()
    })?;
    f.pause_updates()?;
    for _ in 0..4 {
        f.app.check_schema()?;
        let pid = f.backend()?;
        let start = Instant::now();
        let error = f
            .app
            .clone()
            .with_timeout(Duration::from_millis(150))
            .execute(
                &f.owner,
                "rename_project",
                json!({"project":f.project,"name":"must-not-commit"}),
            )
            .err()
            .ok_or("timeout required")?;
        assert_eq!(error.status, 504, "{error:?}");
        assert!(start.elapsed() < Duration::from_secs(1));
        let active:i64=f.admin.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND state='active'",&[&f.tag])?.get(0);
        assert!(
            active <= 1,
            "database work exceeded pool capacity: {active}"
        );
        f.drained()?;
        assert_eq!(f.app.pool_stats().ok_or("pool")?.idle, 0);
        assert_eq!(f.call("get_project", json!({}))?["name"], "review");
        assert_ne!(f.backend()?, pid, "timed-out connection was reused");
        f.admin
            .batch_execute("BEGIN; SET LOCAL lock_timeout='250ms'")?;
        f.admin.query_one(
            "SELECT head FROM gl_projects WHERE id=$1 FOR UPDATE",
            &[&f.project],
        )?;
        f.admin.batch_execute("ROLLBACK")?;
    }
    assert_eq!(f.call("get_project", json!({}))?["name"], "review");
    let audits: i64 = f
        .admin
        .query_one(
            "SELECT count(*) FROM gl_audit_events WHERE project=$1 AND action='rename_project'",
            &[&f.project],
        )?
        .get(0);
    assert_eq!(audits, 0);
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn commit_deadline_retires_connection_without_partial_commit() -> TestResult {
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        ..StorageOptions::default()
    })?;
    let name = format!("gl_review_{}", Uuid::new_v4().simple());
    f.admin.batch_execute(&format!("CREATE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(2); RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER {name} AFTER UPDATE ON gl_projects DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (OLD.id='{}') EXECUTE FUNCTION {name}()",f.project))?;
    f.triggers.push(name);
    let pid = f.backend()?;
    let (app, project, owner) = (f.app.clone(), f.project.clone(), f.owner.clone());
    let task = std::thread::spawn(move || {
        app.with_timeout(Duration::from_millis(300)).execute(
            &owner,
            "rename_project",
            json!({"project":project,"name":"must-not-commit"}),
        )
    });
    f.wait_event("PgSleep", 1)?;
    let error = task
        .join()
        .map_err(|_| "worker panic")?
        .err()
        .ok_or("commit timeout")?;
    assert_eq!(error.status, 504, "{error:?}");
    f.drained()?;
    assert_eq!(f.app.pool_stats().ok_or("pool")?.idle, 0);
    assert_eq!(f.call("get_project", json!({}))?["name"], "review");
    let replacement = f.backend()?;
    assert_ne!(replacement, pid, "uncertain commit connection was reused");
    let audits: i64 = f
        .admin
        .query_one(
            "SELECT count(*) FROM gl_audit_events WHERE project=$1 AND action='rename_project'",
            &[&f.project],
        )?
        .get(0);
    assert_eq!(audits, 0);
    eprintln!(
        "commit_deadline=504 retired_backend={pid} replacement_backend={replacement} project_unchanged=true committed_audits=0"
    );
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn stale_idle_connections_recover_before_first_business_query() -> TestResult {
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        ..StorageOptions::default()
    })?;
    for _ in 0..3 {
        f.app.check_schema()?;
        let pid = f.backend()?;
        f.admin
            .query_one("SELECT pg_terminate_backend($1)", &[&pid])?;
        f.app.check_schema()?;
        assert_eq!(f.call("get_project", json!({}))?["name"], "review");
    }
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn terminated_active_backend_is_verified_before_replacement() -> TestResult {
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        ..StorageOptions::default()
    })?;
    f.admin.batch_execute("BEGIN")?;
    f.admin.query_one(
        "SELECT head FROM gl_projects WHERE id=$1 FOR UPDATE",
        &[&f.project],
    )?;
    let (app, p, owner) = (f.app.clone(), f.project.clone(), f.owner.clone());
    let task = std::thread::spawn(move || {
        app.execute(
            &owner,
            "rename_project",
            json!({"project":p,"name":"terminated"}),
        )
    });
    f.wait_event("Lock", 1)?;
    let pid = f.backend()?;
    f.admin
        .query_one("SELECT pg_terminate_backend($1)", &[&pid])?;
    f.admin.batch_execute("ROLLBACK")?;
    assert!(task.join().map_err(|_| "worker panic")?.is_err());
    f.drained()?;
    assert_eq!(f.app.pool_stats().ok_or("pool")?.idle, 0);
    assert_eq!(f.call("get_project", json!({}))?["name"], "review");
    assert_ne!(f.backend()?, pid, "terminated connection was reused");
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn statement_deadline_and_explicit_cancellation_are_distinct() -> TestResult {
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        statement_timeout: Duration::from_millis(40),
        ..StorageOptions::default()
    })?;
    f.pause_updates()?;
    let pid = f.backend()?;
    let error = f
        .call("rename_project", json!({"name":"timeout"}))
        .err()
        .ok_or("timeout")?;
    assert_eq!(error.status, 504, "{error:?}");
    f.drained()?;
    assert_eq!(f.app.pool_stats().ok_or("pool")?.idle, 0);
    assert_eq!(f.call("get_project", json!({}))?["name"], "review");
    assert_ne!(f.backend()?, pid, "statement timeout connection was reused");
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        ..StorageOptions::default()
    })?;
    f.pause_updates()?;
    let (app, p, owner) = (f.app.clone(), f.project.clone(), f.owner.clone());
    let task = std::thread::spawn(move || {
        app.execute(
            &owner,
            "rename_project",
            json!({"project":p,"name":"cancelled"}),
        )
    });
    f.wait_event("PgSleep", 1)?;
    let pid = f.backend()?;
    f.admin.query_one("SELECT pg_cancel_backend($1)", &[&pid])?;
    let error = task
        .join()
        .map_err(|_| "worker panic")?
        .err()
        .ok_or("cancelled")?;
    assert_eq!(error.body["error"]["code"], "cancelled", "{error:?}");
    f.drained()?;
    assert_eq!(f.app.pool_stats().ok_or("pool")?.idle, 0);
    assert_eq!(f.call("get_project", json!({}))?["name"], "review");
    let replacement = f.backend()?;
    assert_ne!(replacement, pid, "cancelled connection was reused");
    eprintln!(
        "explicit_cancellation=cancelled retired_backend={pid} replacement_backend={replacement} project_unchanged=true"
    );
    Ok(())
}
#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn create_workspace_rechecks_state_after_management_lock() -> TestResult {
    for operation in [
        "archive_project",
        "delete_project",
        "remove_member",
        "set_member",
    ] {
        let mut f = Fixture::new(StorageOptions {
            pool_size: 4,
            ..StorageOptions::default()
        })?;
        let editor = Uuid::new_v4().to_string();
        f.call("set_member", json!({"subject":editor,"role":"editor"}))?;
        f.admin.batch_execute("BEGIN")?;
        f.admin.query_one(
            "SELECT subject FROM gl_project_members WHERE project=$1 AND subject=$2 FOR UPDATE",
            &[&f.project, &f.owner],
        )?;
        let (app, p, owner, e) = (
            f.app.clone(),
            f.project.clone(),
            f.owner.clone(),
            editor.clone(),
        );
        let manage = std::thread::spawn(move || {
            let body = match operation {
                "archive_project" => json!({"project":p,"archived":true}),
                "delete_project" => json!({"project":p,"confirm_name":"review"}),
                "remove_member" => json!({"project":p,"subject":e}),
                _ => json!({"project":p,"subject":e,"role":"viewer"}),
            };
            app.execute(&owner, operation, body)
        });
        f.wait_event("Lock", 1)?;
        let (app, p) = (f.app.clone(), f.project.clone());
        let create = std::thread::spawn(move || {
            app.execute(&editor, "create_workspace", json!({"project":p}))
        });
        f.wait_event("Lock", 2)?;
        f.admin.batch_execute("ROLLBACK")?;
        manage.join().map_err(|_| "manager panic")??;
        let error = create
            .join()
            .map_err(|_| "creator panic")?
            .err()
            .ok_or("stale authorization accepted")?;
        assert_eq!(
            error.status,
            if operation == "archive_project" {
                409
            } else {
                404
            },
            "{operation}: {error:?}"
        );
        let count: i64 = f
            .admin
            .query_one(
                "SELECT count(*) FROM gl_workspaces WHERE project=$1",
                &[&f.project],
            )?
            .get(0);
        assert_eq!(count, 0, "{operation}");
    }
    Ok(())
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn independent_draft_saves_do_not_wait_for_another_workspace() -> TestResult {
    let mut f = Fixture::new(StorageOptions::default())?;
    let dataset = f.call(
        "create_dataset",
        json!({"name":"parallel","geometry_type":"point"}),
    )?["dataset"]
        .clone();
    let first = f.call("create_workspace", json!({}))?["workspace"].clone();
    let second = f.call("create_workspace", json!({}))?["workspace"].clone();
    f.admin.batch_execute("BEGIN")?;
    f.admin.query_one(
        "SELECT id FROM gl_workspaces WHERE id=$1 FOR UPDATE",
        &[&first.as_str().ok_or("workspace")?],
    )?;
    let request = |workspace: Value| json!({"project":f.project,"workspace":workspace,"expected_workspace_version":0,"edits":[{"dataset":dataset,"feature_id":"one","feature":{"type":"Feature","id":"one","properties":{},"geometry":null}}]});
    let first_request = request(first);
    let second_request = request(second);
    let app = f.app.clone();
    let owner = f.owner.clone();
    let first_thread = std::thread::spawn(move || app.execute(&owner, "save", first_request));
    if let Err(error) = f.wait_event("Lock", 1) {
        f.admin.batch_execute("ROLLBACK")?;
        return Err(format!("{error}; first save: {:?}", first_thread.join()).into());
    }
    let app = f.app.clone();
    let owner = f.owner.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let second_thread = std::thread::spawn(move || {
        let result = app.execute(&owner, "save", second_request);
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(Duration::from_secs(2));
    f.admin.batch_execute("ROLLBACK")?;
    first_thread.join().map_err(|_| "first save panicked")??;
    second_thread.join().map_err(|_| "second save panicked")?;
    assert_eq!(
        result.map_err(|_| "independent draft blocked by project lock")??["version"],
        1
    );
    Ok(())
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn committed_receipt_replay_does_not_wait_for_project_writer() -> TestResult {
    let mut f = Fixture::new(StorageOptions::default())?;
    let dataset = f.call(
        "create_dataset",
        json!({"name":"replay","geometry_type":"point"}),
    )?["dataset"]
        .clone();
    let workspace = f.call("create_workspace", json!({}))?["workspace"].clone();
    f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"edits":[{"dataset":dataset,"feature_id":"one","feature":{"type":"Feature","id":"one","properties":{},"geometry":null}}]}))?;
    let request = json!({"project":f.project,"workspace":workspace,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"replay"});
    let receipt = f.app.execute(&f.owner, "publish", request.clone())?;
    f.admin.batch_execute("BEGIN")?;
    f.admin.query_one(
        "SELECT head FROM gl_projects WHERE id=$1 FOR NO KEY UPDATE",
        &[&f.project],
    )?;
    let app = f.app.clone();
    let owner = f.owner.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let replay = std::thread::spawn(move || {
        let _ = send.send(app.execute(&owner, "publish", request));
    });
    let result = receive.recv_timeout(Duration::from_secs(2));
    f.admin.batch_execute("ROLLBACK")?;
    replay.join().map_err(|_| "replay panicked")?;
    assert_eq!(
        result.map_err(|_| "receipt replay queued behind project writer")??,
        receipt
    );
    Ok(())
}
