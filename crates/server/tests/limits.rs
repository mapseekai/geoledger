//! Failure paths under load: a storage lock held past the request deadline,
//! saturated execution slots, stalled request bodies and malformed credentials.
//! Every failure is bounded in time, carries a request ID and leaves the service
//! healthy afterwards.
use axum::{
    body::{Body, Bytes},
    http::Request,
};
use geoledger_server::{
    Application, Authentication, Authenticator, Limits, Service, Storage, Tokens,
};
use http_body_util::BodyExt;
use serde_json::Value;
use std::time::{Duration, Instant};
use tower::ServiceExt;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
type BoxError = Box<dyn std::error::Error + Send + Sync>;
// Public, test-only credential for ephemeral in-process servers.
const TOKEN: &str = "test-only-limits-credential-0123456789-abcdefghijkl";

fn service(db: &std::path::Path, limits: Limits) -> Result<Service, BoxError> {
    let app = Application::new(Storage::Sqlite(db.to_owned()));
    app.migrate()?;
    let tokens =
        Tokens::from_json(format!(r#"[{{"subject":"ops","token":"{TOKEN}"}}]"#).as_bytes())?;
    Ok(Service::new(app, Authentication::new(Authenticator::Tokens(tokens))).with_limits(limits))
}
async fn send(
    router: &axum::Router,
    path: &str,
    auth: Option<&str>,
    body: Body,
) -> Result<(u16, Value), BoxError> {
    let mut request = Request::builder()
        .method(if path.starts_with("/api/") {
            "POST"
        } else {
            "GET"
        })
        .uri(path)
        .header("content-type", "application/json");
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    let response = router.clone().oneshot(request.body(body)?).await?;
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}
fn bearer() -> String {
    format!("Bearer {TOKEN}")
}
async fn call(router: &axum::Router, op: &str, body: &str) -> Result<(u16, Value), BoxError> {
    send(
        router,
        &format!("/api/v1/{op}"),
        Some(&bearer()),
        Body::from(body.to_owned()),
    )
    .await
}
fn assert_error(body: &Value, code: &str) {
    assert_eq!(body["error"]["code"], code, "{body}");
    assert!(
        body["error"]["request_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "{body}"
    );
}

#[tokio::test]
async fn storage_lock_past_deadline_and_saturated_slots_fail_fast_and_recover() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("limits.sqlite3");
    let router = geoledger_server::router(service(
        &db,
        Limits {
            max_concurrency: 1,
            request_timeout: Duration::from_secs(1),
            ..Limits::default()
        },
    )?);
    let (status, project) = call(&router, "create_project", r#"{"name":"limits"}"#).await?;
    assert_eq!(status, 200, "{project}");
    let project = project["project"].as_str().ok_or("project id")?.to_owned();

    // Another process holds the SQLite write lock (e.g. a long maintenance job).
    let blocker = rusqlite::Connection::open(&db)?;
    blocker.execute_batch("BEGIN IMMEDIATE")?;
    let started = Instant::now();
    let write = {
        let router = router.clone();
        let body = format!(r#"{{"project":"{project}","name":"roads"}}"#);
        tokio::spawn(async move { call(&router, "create_dataset", &body).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The only execution slot is taken: reads are refused immediately with 429,
    // and readiness reports the saturation instead of queueing.
    let refused = Instant::now();
    let (status, body) = call(&router, "list_projects", "{}").await?;
    assert_eq!(status, 429, "{body}");
    assert_error(&body, "busy");
    assert!(refused.elapsed() < Duration::from_millis(500));
    assert_eq!(send(&router, "/ready", None, Body::empty()).await?.0, 503);

    // The blocked write waits for the lock only until the request deadline, then
    // fails with the documented retryable "busy" error (SQLITE_BUSY maps to 429).
    let (status, body) = write.await??;
    let elapsed = started.elapsed();
    assert_eq!(status, 429, "{body}");
    assert_error(&body, "busy");
    assert!(
        elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(5),
        "deadline not enforced: {elapsed:?}"
    );

    // Nothing was written, and the service recovers as soon as the lock is gone.
    blocker.execute_batch("ROLLBACK")?;
    drop(blocker);
    let body = format!(r#"{{"project":"{project}","name":"roads"}}"#);
    let (status, created) = call(&router, "create_dataset", &body).await?;
    assert_eq!(
        status, 200,
        "the timed-out write must not have committed: {created}"
    );
    assert_eq!(send(&router, "/ready", None, Body::empty()).await?.0, 200);
    let (status, metrics) = {
        let request = Request::get("/metrics")
            .header("authorization", bearer())
            .body(Body::empty())?;
        let response = router.clone().oneshot(request).await?;
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await?.to_bytes();
        (status, String::from_utf8(bytes.to_vec())?)
    };
    assert_eq!(status, 200);
    assert!(
        metrics.contains(
            "geoledger_operations_total{protocol=\"http\",operation=\"list_projects\",status=\"429\"} 1"
        ),
        "{metrics}"
    );
    Ok(())
}

#[tokio::test]
async fn stalled_request_body_times_out_and_releases_its_slot() -> TestResult {
    let dir = tempfile::tempdir()?;
    let router = geoledger_server::router(service(
        &dir.path().join("body.sqlite3"),
        Limits {
            max_concurrency: 1,
            request_timeout: Duration::from_secs(1),
            ..Limits::default()
        },
    )?);
    let started = Instant::now();
    let stalled = Body::from_stream(tokio_stream::pending::<Result<Bytes, std::io::Error>>());
    let (status, body) = send(&router, "/api/v1/list_projects", Some(&bearer()), stalled).await?;
    assert_eq!(status, 408, "{body}");
    assert_error(&body, "deadline_exceeded");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(call(&router, "list_projects", "{}").await?.0, 200);
    Ok(())
}

#[tokio::test]
async fn malformed_credentials_are_rejected_uniformly() -> TestResult {
    let dir = tempfile::tempdir()?;
    let router = geoledger_server::router(service(
        &dir.path().join("auth.sqlite3"),
        Limits::default(),
    )?);
    let oversized = format!("Bearer {}", "a".repeat(16 * 1024));
    let near_miss = format!("Bearer {}x", &TOKEN[..TOKEN.len() - 1]);
    let lowercase = format!("bearer {TOKEN}");
    let cases: [(&str, Option<&str>); 7] = [
        ("missing header", None),
        ("empty", Some("")),
        ("basic scheme", Some("Basic b3BzOnNlY3JldA==")),
        ("bearer without token", Some("Bearer ")),
        ("oversized token", Some(&oversized)),
        ("one character off", Some(&near_miss)),
        ("scheme case", Some(&lowercase)),
    ];
    let mut rejected = 0;
    for (case, auth) in cases {
        let (status, body) = send(&router, "/api/v1/list_projects", auth, Body::from("{}")).await?;
        if case == "scheme case" && status == 200 {
            continue; // RFC 7235 schemes are case-insensitive; accepting is fine.
        }
        assert_eq!(status, 401, "{case}: {body}");
        assert_error(&body, "unauthenticated");
        let text = body.to_string();
        assert!(
            !text.contains(TOKEN) && !text.contains("aaaa"),
            "{case} echoes input: {text}"
        );
        rejected += 1;
    }
    assert_eq!(call(&router, "list_projects", "{}").await?.0, 200);
    let request = Request::get("/metrics")
        .header("authorization", bearer())
        .body(Body::empty())?;
    let response = router.clone().oneshot(request).await?;
    let bytes = response.into_body().collect().await?.to_bytes();
    let metrics = String::from_utf8(bytes.to_vec())?;
    assert!(
        metrics.contains(&format!("geoledger_auth_failures_total {rejected}")),
        "{metrics}"
    );
    Ok(())
}
