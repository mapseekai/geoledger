//! Operational behaviour: request-ID correlation, limits, metrics, health and shutdown.
use axum::{body::Body, http::Request};
use geoledger_server::{
    Application, Authentication, Authenticator, Limits, Rate, Service, Storage, Tokens,
};
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    net::TcpListener,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tower::ServiceExt;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
// Public, test-only credentials for ephemeral in-process servers.
const TOKEN: &str = "test-only-operations-credential-0123456789-abcdefgh";

fn service(
    dir: &std::path::Path,
    limits: Limits,
) -> Result<Service, Box<dyn std::error::Error + Send + Sync>> {
    let app = Application::new(Storage::Sqlite(dir.join("ops.sqlite3")));
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
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<(u16, axum::http::HeaderMap, String), Box<dyn std::error::Error + Send + Sync>> {
    let mut request = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        request = request.header(*k, *v);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_owned()))?)
        .await?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((status, headers, String::from_utf8(bytes.to_vec())?))
}
const AUTH: (&str, &str) = (
    "authorization",
    "Bearer test-only-operations-credential-0123456789-abcdefgh",
);

#[tokio::test]
async fn request_ids_correlate_errors_and_metrics_are_labelled() -> TestResult {
    let dir = tempfile::tempdir()?;
    let router = geoledger_server::router(service(dir.path(), Limits::default())?);
    let (status, headers, body) = send(
        &router,
        "POST",
        "/api/v1/get_project",
        &[AUTH, ("x-request-id", "gateway-req.42")],
        r#"{"project":"00000000-0000-4000-8000-000000000000"}"#,
    )
    .await?;
    assert_eq!(status, 404);
    assert_eq!(headers["x-request-id"], "gateway-req.42");
    let body: Value = serde_json::from_str(&body)?;
    assert_eq!(body["error"]["request_id"], "gateway-req.42");
    // Unsafe incoming IDs are replaced, and the body still matches the header.
    let (_, headers, body) = send(
        &router,
        "POST",
        "/api/v1/get_project",
        &[AUTH, ("x-request-id", "bad id with spaces")],
        r#"{"project":"00000000-0000-4000-8000-000000000000"}"#,
    )
    .await?;
    let body: Value = serde_json::from_str(&body)?;
    assert_ne!(headers["x-request-id"], "bad id with spaces");
    assert_eq!(
        body["error"]["request_id"],
        headers["x-request-id"].to_str()?
    );
    let (status, _, _) = send(
        &router,
        "POST",
        "/api/v1/create_project",
        &[AUTH],
        r#"{"name":"ops"}"#,
    )
    .await?;
    assert_eq!(status, 200);
    let (status, _, _) = send(
        &router,
        "POST",
        "/api/v1/list_projects",
        &[("authorization", "Bearer wrong")],
        "{}",
    )
    .await?;
    assert_eq!(status, 401);
    let (status, _, metrics) = send(&router, "GET", "/metrics", &[AUTH], "").await?;
    assert_eq!(status, 200);
    for expected in [
        "geoledger_operations_total{protocol=\"http\",operation=\"create_project\",status=\"200\"} 1",
        "geoledger_operations_total{protocol=\"http\",operation=\"get_project\",status=\"404\"} 2",
        "geoledger_operations_total{protocol=\"http\",operation=\"list_projects\",status=\"401\"} 1",
        "geoledger_auth_failures_total 1",
        "geoledger_operation_duration_seconds_count{protocol=\"http\",operation=\"create_project\"} 1",
        "geoledger_execution_slots_capacity 20",
    ] {
        assert!(metrics.contains(expected), "missing {expected}\n{metrics}");
    }
    Ok(())
}

#[tokio::test]
async fn subject_and_ip_rate_limits() -> TestResult {
    let dir = tempfile::tempdir()?;
    let router = geoledger_server::router(service(
        dir.path(),
        Limits {
            per_subject: Rate::new(0.001, 2.0),
            per_ip: Rate::new(0.001, 3.0),
            trust_forwarded_for: true,
            ..Limits::default()
        },
    )?);
    let call = |ip: &'static str| {
        let router = router.clone();
        async move {
            send(
                &router,
                "POST",
                "/api/v1/list_projects",
                &[AUTH, ("x-forwarded-for", ip)],
                "{}",
            )
            .await
        }
    };
    assert_eq!(call("192.0.2.1").await?.0, 200);
    assert_eq!(call("192.0.2.1").await?.0, 200);
    let (status, headers, body) = call("192.0.2.2").await?;
    assert_eq!(status, 429, "subject limit applies across addresses");
    assert_eq!(headers["retry-after"], "1");
    assert!(body.contains("rate limit"));
    // The address bucket (3) empties before authentication, even with bad credentials.
    let anonymous = |ip: &'static str| {
        let router = router.clone();
        async move {
            send(
                &router,
                "POST",
                "/api/v1/list_projects",
                &[("authorization", "Bearer nope"), ("x-forwarded-for", ip)],
                "{}",
            )
            .await
        }
    };
    assert_eq!(anonymous("198.51.100.7").await?.0, 401);
    assert_eq!(anonymous("198.51.100.7").await?.0, 401);
    assert_eq!(anonymous("198.51.100.7").await?.0, 401);
    assert_eq!(anonymous("198.51.100.7").await?.0, 429);
    let (_, _, metrics) = send(&router, "GET", "/metrics", &[AUTH], "").await?;
    assert!(
        metrics.contains("geoledger_rate_limited_total{scope=\"subject\"} 1"),
        "{metrics}"
    );
    assert!(
        metrics.contains("geoledger_rate_limited_total{scope=\"ip\"} 1"),
        "{metrics}"
    );
    Ok(())
}

#[tokio::test]
async fn draining_reports_not_ready_and_health_router_is_minimal() -> TestResult {
    let dir = tempfile::tempdir()?;
    let service = service(dir.path(), Limits::default())?;
    let router = geoledger_server::router(service.clone());
    let health = geoledger_server::health_router(service.clone());
    assert_eq!(send(&health, "GET", "/ready", &[], "").await?.0, 200);
    assert_eq!(send(&health, "GET", "/health", &[], "").await?.0, 200);
    assert_eq!(
        send(&health, "POST", "/api/v1/list_projects", &[AUTH], "{}")
            .await?
            .0,
        404
    );
    assert_eq!(send(&health, "GET", "/metrics", &[AUTH], "").await?.0, 404);
    service.start_draining();
    assert_eq!(send(&router, "GET", "/ready", &[], "").await?.0, 503);
    assert_eq!(send(&health, "GET", "/ready", &[], "").await?.0, 503);
    assert_eq!(send(&router, "GET", "/health", &[], "").await?.0, 200);
    Ok(())
}

fn free_port() -> Result<std::net::SocketAddr, std::io::Error> {
    TcpListener::bind("127.0.0.1:0")?.local_addr()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_grpc_health_and_bounded_shutdown() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (http, grpc, health) = (free_port()?, free_port()?, free_port()?);
    let mut child = Command::new(env!("CARGO_BIN_EXE_geoledger-server"))
        .args(["--data-dir", dir.path().to_str().ok_or("path")?])
        .args(["--http", &http.to_string(), "--grpc", &grpc.to_string()])
        .env("GL_HEALTH_LISTEN", health.to_string())
        .env("GL_SHUTDOWN_TIMEOUT_SECS", "5")
        .env_remove("GL_STORAGE")
        .env_remove("GL_DATABASE_URL")
        .env_remove("GL_TOKEN_FILE")
        .env_remove("GL_JWKS_FILE")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let probe = || {
        Command::new(env!("CARGO_BIN_EXE_geoledger-server"))
            .arg("probe")
            .env("GL_HEALTH_LISTEN", health.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };
    let mut ready = false;
    for _ in 0..100 {
        if probe()?.success() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ready, "probe never succeeded");
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{grpc}"))?
        .connect()
        .await?;
    let mut client = tonic_health::pb::health_client::HealthClient::new(channel);
    let reply = client
        .check(tonic_health::pb::HealthCheckRequest {
            service: "geoledger.v1.GeoLedger".into(),
        })
        .await?
        .into_inner();
    assert_eq!(
        reply.status,
        tonic_health::pb::health_check_response::ServingStatus::Serving as i32
    );
    // gRPC errors carry the caller's request ID in metadata and in ErrorDetail.
    let credentials: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("admin-credentials.json"))?)?;
    let token = credentials[0]["token"].as_str().ok_or("token")?;
    let mut gl = geoledger_rpc::v1::geo_ledger_client::GeoLedgerClient::new(
        tonic::transport::Endpoint::from_shared(format!("http://{grpc}"))?
            .connect()
            .await?,
    );
    let mut request = tonic::Request::new(geoledger_rpc::v1::ProjectRequest {
        project: "00000000-0000-4000-8000-000000000000".into(),
    });
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse()?);
    request
        .metadata_mut()
        .insert("x-request-id", "grpc-req-7".parse()?);
    let error = gl
        .get_project(request)
        .await
        .err()
        .ok_or("expected error")?;
    assert_eq!(error.code(), tonic::Code::NotFound);
    assert_eq!(
        error.metadata().get("x-request-id").ok_or("id")?.to_str()?,
        "grpc-req-7"
    );
    #[cfg(unix)]
    {
        let started = Instant::now();
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()?;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "shutdown not bounded"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert!(status.success(), "{status:?}");
        assert!(!probe()?.success());
    }
    #[cfg(not(unix))]
    {
        let _ = Instant::now();
        child.kill()?;
        child.wait()?;
    }
    Ok(())
}
