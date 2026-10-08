//! Several server instances sharing one database. Each `Application` owns its
//! own connection pool, as separate processes would, and publications race
//! across them: disjoint edits must all land exactly once, overlapping edits must
//! produce exactly one winner and explicit conflicts, and retried publications
//! must stay idempotent across instances.
use geoledger_engine::{Application, Storage};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
const WRITERS: usize = 8;

fn text(v: &Value, key: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(v[key]
        .as_str()
        .ok_or(format!("missing {key} in {v}"))?
        .to_owned())
}
/// Retry only transient capacity errors (SQLite busy / database unavailable).
fn execute(app: &Application, op: &str, body: Value) -> Result<Value, geoledger_engine::Error> {
    let mut attempt = 0;
    loop {
        match app.execute("owner", op, body.clone()) {
            Err(e) if matches!(e.status, 429 | 503) && attempt < 50 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20 * attempt));
            }
            other => return other,
        }
    }
}
fn feature(dataset: &str, id: &str, value: usize) -> Value {
    json!({"dataset":dataset,"feature_id":id,"feature":{"type":"Feature","id":id,"properties":{"writer":value},"geometry":{"type":"Point","coordinates":[value as f64 / 10.0, 1.0]}}})
}

fn scenario(instances: &[Application]) -> TestResult {
    let first = instances.first().ok_or("instances")?;
    first.migrate()?;
    for app in instances {
        app.migrate()?; // concurrent migrate on an already-current schema is a no-op
    }
    let project = text(
        &execute(first, "create_project", json!({"name":"multi"}))?,
        "project",
    )?;
    let dataset = text(
        &execute(
            first,
            "create_dataset",
            json!({"project":project,"name":"shared"}),
        )?,
        "dataset",
    )?;
    let open = |edits: Vec<Value>| -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let w = text(
            &execute(first, "create_workspace", json!({"project":project}))?,
            "workspace",
        )?;
        execute(
            first,
            "save",
            json!({"project":project,"workspace":w,"expected_workspace_version":0,"edits":edits}),
        )?;
        Ok(w)
    };
    let publish = |app: &Application, w: &str, request: &str| {
        execute(
            app,
            "publish",
            json!({"project":project,"workspace":w,"expected_workspace_version":1,"request_id":request,"message":"race"}),
        )
    };

    // Quota checks must serialize across independent pools, including the first project.
    let subject = format!("quota-{}", Uuid::new_v4());
    let barrier = std::sync::Barrier::new(WRITERS);
    let quota_apps: Vec<_> = instances
        .iter()
        .cloned()
        .map(|app| {
            app.with_policy(geoledger_engine::Policy {
                max_owned_projects: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..WRITERS)
            .map(|i| {
                let app = &quota_apps[i % quota_apps.len()];
                let subject = &subject;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    app.execute(subject, "create_project", json!({"name":"quota race"}))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().map_err(|_| "quota writer panicked"))
            .collect::<Result<Vec<_>, _>>()
    })?;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    for error in results.into_iter().filter_map(Result::err) {
        assert_eq!(error.status, 403, "{}", error.body);
    }

    // 1. Disjoint edits published concurrently through alternating instances.
    let workspaces = (0..WRITERS)
        .map(|i| open(vec![feature(&dataset, &format!("own-{i}"), i)]))
        .collect::<Result<Vec<_>, _>>()?;
    let requests: Vec<String> = (0..WRITERS).map(|_| Uuid::new_v4().to_string()).collect();
    let receipts = std::thread::scope(|scope| {
        let handles: Vec<_> = workspaces
            .iter()
            .zip(&requests)
            .enumerate()
            .map(|(i, (w, r))| {
                let app = &instances[i % instances.len()];
                scope.spawn(move || publish(app, w, r))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().map_err(|_| "publisher panicked"))
            .collect::<Result<Vec<_>, _>>()
    })?;
    let mut revisions = BTreeSet::new();
    for receipt in &receipts {
        let receipt = receipt
            .as_ref()
            .map_err(|e| format!("disjoint publish failed: {}", e.body))?;
        revisions.insert(
            receipt["revision"]
                .as_i64()
                .ok_or(format!("revision in {receipt}"))?,
        );
    }
    assert_eq!(
        revisions,
        (1..=WRITERS as i64).collect(),
        "every publication gets its own revision"
    );
    for app in instances {
        assert_eq!(
            execute(app, "get_project", json!({"project":project}))?["head"],
            WRITERS
        );
        let all = execute(
            app,
            "features",
            json!({"project":project,"dataset":dataset,"limit":100}),
        )?;
        let ids: BTreeSet<_> = all["features"]
            .as_array()
            .ok_or("features")?
            .iter()
            .filter_map(|f| f["id"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(ids, (0..WRITERS).map(|i| format!("own-{i}")).collect());
    }
    // A retried publication (lost response) is answered from the receipt on
    // any instance and never commits twice.
    for (i, (w, r)) in workspaces.iter().zip(&requests).enumerate() {
        let app = &instances[(i + 1) % instances.len()];
        let again = publish(app, w, r)?;
        assert_eq!(Some(&again), receipts[i].as_ref().ok());
    }
    assert_eq!(
        execute(first, "get_project", json!({"project":project}))?["head"],
        WRITERS
    );

    // 2. Overlapping edits to one feature: exactly one winner, the rest conflict.
    let head = WRITERS as i64;
    let contenders = (0..WRITERS)
        .map(|i| open(vec![feature(&dataset, "contested", i)]))
        .collect::<Result<Vec<_>, _>>()?;
    let outcomes = std::thread::scope(|scope| {
        let handles: Vec<_> = contenders
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let app = &instances[i % instances.len()];
                scope.spawn(move || publish(app, w, &Uuid::new_v4().to_string()))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().map_err(|_| "publisher panicked"))
            .collect::<Result<Vec<_>, _>>()
    })?;
    let winners: Vec<_> = outcomes
        .iter()
        .enumerate()
        .filter(|(_, o)| o.is_ok())
        .collect();
    assert_eq!(winners.len(), 1, "exactly one overlapping publication wins");
    let winner = winners.first().map(|(i, _)| *i).ok_or("winner")?;
    for outcome in &outcomes {
        if let Err(e) = outcome {
            assert_eq!(e.status, 409, "{}", e.body);
            assert_eq!(e.body["head"], head + 1, "{}", e.body);
            assert_eq!(
                e.body["conflicts"][0]["feature_id"], "contested",
                "{}",
                e.body
            );
        }
    }
    for app in instances {
        assert_eq!(
            execute(app, "get_project", json!({"project":project}))?["head"],
            head + 1
        );
        let contested = execute(
            app,
            "features",
            json!({"project":project,"dataset":dataset,"feature_id":"contested"}),
        )?;
        assert_eq!(contested["properties"]["writer"], winner);
    }
    // Losing workspaces stay open and unchanged.
    for (i, w) in contenders.iter().enumerate().filter(|(i, _)| *i != winner) {
        let ws = execute(
            &instances[i % instances.len()],
            "get_workspace",
            json!({"project":project,"workspace":w}),
        )?;
        assert_eq!(
            (ws["status"].as_str(), ws["version"].as_i64()),
            (Some("open"), Some(1))
        );
    }
    Ok(())
}

#[test]
fn sqlite_instances_sharing_a_file() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("shared.sqlite3");
    let instances: Vec<_> = (0..3)
        .map(|_| Application::new(Storage::Sqlite(path.clone())))
        .collect();
    scenario(&instances)
}

#[test]
#[ignore = "requires a PostGIS server via GL_TEST_DATABASE_URL"]
fn postgis_instances_sharing_a_database() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let name = format!("geoledger_multi_{}", Uuid::new_v4().simple());
    let mut admin = postgres::Client::connect(&dsn, postgres::NoTls)?;
    admin.batch_execute(&format!("CREATE DATABASE {name}"))?;
    let result = (|| -> TestResult {
        let target = dsn.replacen("/geoledger_test", &format!("/{name}"), 1);
        let mut db = postgres::Client::connect(&target, postgres::NoTls)?;
        db.batch_execute("CREATE EXTENSION postgis")?;
        drop(db);
        let instances: Vec<_> = (0..3)
            .map(|_| Application::new(Storage::Postgis(target.clone())))
            .collect();
        scenario(&instances)
    })();
    admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"))?;
    result
}
