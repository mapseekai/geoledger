//! Failure paths under load (HTTP and gRPC): a storage lock held past the request deadline,
//! saturated execution slots, stalled request bodies and malformed credentials.
//! Every failure is bounded in time, carries a request ID and leaves the service
//! healthy afterwards.
use axum::{
    body::{Body, Bytes},
    http::Request,
};
use geoledger_rpc::v1 as pb;
use geoledger_server::{
    Application, Authentication, Authenticator, Limits, Service, Storage, Tokens,
};
use http_body_util::BodyExt;
use prost::Message;
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
    let tokens = Tokens::from_json(
        format!(
            r#"[{{"subject":"ops","token_sha256":"{}"}}]"#,
            geoledger_server::sha256_hex(TOKEN)
        )
        .as_bytes(),
    )?;
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
        let body = format!(r#"{{"project":"{project}","name":"roads","geometry_type":"point"}}"#);
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
    let body = format!(r#"{{"project":"{project}","name":"roads","geometry_type":"point"}}"#);
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
            !text.contains(TOKEN) && !text.contains(&"a".repeat(64)),
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

fn rpc_request<T>(body: T, auth: Option<&str>) -> Result<tonic::Request<T>, BoxError> {
    let mut request = tonic::Request::new(body);
    if let Some(auth) = auth {
        request
            .metadata_mut()
            .insert("authorization", auth.parse()?);
    }
    Ok(request)
}
fn rpc_detail(status: &tonic::Status) -> Result<pb::ErrorDetail, BoxError> {
    let envelope = pb::RpcStatus::decode(status.details())?;
    let any = envelope.details.first().ok_or("error detail")?;
    Ok(pb::ErrorDetail::decode(any.value.as_slice())?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grpc_deadline_busy_and_unauthenticated_paths() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("rpc.sqlite3");
    let service = service(
        &db,
        Limits {
            max_concurrency: 1,
            request_timeout: Duration::from_secs(10),
            ..Limits::default()
        },
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let channel = tonic::transport::Endpoint::from_shared(endpoint)?
        .connect()
        .await?;
    let mut client = pb::geo_ledger_client::GeoLedgerClient::new(channel);
    let auth = bearer();

    for bad in [
        None,
        Some("Basic b3BzOnNlY3JldA=="),
        Some("Bearer not-a-known-token"),
    ] {
        let status = client
            .list_projects(rpc_request(pb::PageRequest::default(), bad)?)
            .await
            .err()
            .ok_or("unauthenticated call must fail")?;
        assert_eq!(status.code(), tonic::Code::Unauthenticated, "{bad:?}");
        let detail = rpc_detail(&status)?;
        assert_eq!(detail.code, "unauthenticated");
        assert!(uuid::Uuid::parse_str(&detail.request_id).is_ok());
    }

    let project = client
        .create_project(rpc_request(
            pb::NameRequest {
                name: "rpc-limits".into(),
            },
            Some(&auth),
        )?)
        .await?
        .into_inner()
        .project;

    // The client's grpc-timeout (2 s) is shorter than the server limit (10 s)
    // and bounds how long the write waits for the external lock.
    let blocker = rusqlite::Connection::open(&db)?;
    blocker.execute_batch("BEGIN IMMEDIATE")?;
    let started = Instant::now();
    let write = {
        let mut client = client.clone();
        let mut request = rpc_request(
            pb::DatasetRequest {
                coordinate_dimension: 2,
                geometry_type: "point".into(),
                project: project.clone(),
                name: "roads".into(),
            },
            Some(&auth),
        )?;
        request.set_timeout(Duration::from_secs(2));
        tokio::spawn(async move { client.create_dataset(request).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let refused = Instant::now();
    let status = client
        .list_projects(rpc_request(pb::PageRequest::default(), Some(&auth))?)
        .await
        .err()
        .ok_or("saturated server must refuse")?;
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    assert_eq!(rpc_detail(&status)?.code, "busy");
    assert!(refused.elapsed() < Duration::from_millis(500));

    let status = write.await?.err().ok_or("blocked write must fail")?;
    let elapsed = started.elapsed();
    assert!(
        matches!(
            status.code(),
            tonic::Code::ResourceExhausted | tonic::Code::DeadlineExceeded | tonic::Code::Cancelled
        ),
        "{status:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(1800) && elapsed < Duration::from_secs(5),
        "client deadline not honoured: {elapsed:?}"
    );

    blocker.execute_batch("ROLLBACK")?;
    drop(blocker);
    // The execution slot is released once the engine gives up on the lock.
    let mut recovered = None;
    for _ in 0..50 {
        match client
            .create_dataset(rpc_request(
                pb::DatasetRequest {
                    coordinate_dimension: 2,
                    geometry_type: "point".into(),
                    project: project.clone(),
                    name: "roads".into(),
                },
                Some(&auth),
            )?)
            .await
        {
            Ok(reply) => {
                recovered = Some(reply.into_inner());
                break;
            }
            Err(s) if s.code() == tonic::Code::ResourceExhausted => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(s) => return Err(s.into()),
        }
    }
    assert!(
        recovered.is_some(),
        "service did not recover after the lock was released"
    );
    servers.abort_all();
    Ok(())
}
