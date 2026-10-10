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
fn deadlines_cancel_database_work_and_hold_pool_slot_until_cleanup() -> TestResult {
    let mut f = Fixture::new(StorageOptions {
        pool_size: 1,
        ..StorageOptions::default()
    })?;
    f.pause_updates()?;
    for _ in 0..4 {
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
    assert_eq!(f.call("get_project", json!({}))?["name"], "review");
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
    let error = f
        .call("rename_project", json!({"name":"timeout"}))
        .err()
        .ok_or("timeout")?;
    assert_eq!(error.status, 504, "{error:?}");
    f.drained()?;
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
    assert_eq!(f.call("get_project", json!({}))?["name"], "review");
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
