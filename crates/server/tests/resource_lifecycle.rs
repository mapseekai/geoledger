//! Regressions for pre-decode admission, transfer ownership and detached operations.
use axum::{
    body::{Body, Bytes},
    http::{Request, StatusCode},
};
use geoledger_server::{
    Application, Authentication, Authenticator, Limits, Service, Storage, Tokens,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tower::ServiceExt;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
const TOKEN: &str = "test-only-resource-lifecycle-no-production-use";
fn service(
    path: &std::path::Path,
    limits: Limits,
) -> Result<Service, Box<dyn std::error::Error + Send + Sync>> {
    let app = Application::new(Storage::Sqlite(path.to_owned()));
    app.migrate()?;
    let tokens = Tokens::from_json(&serde_json::to_vec(
        &json!([{"subject":"owner","token_sha256":geoledger_server::sha256_hex(TOKEN)}]),
    )?)?;
    Ok(Service::new(app, Authentication::new(Authenticator::Tokens(tokens))).with_limits(limits))
}
fn request(op: &str, value: Value) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/v1/{op}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
}
struct ObservedBody {
    polls: Arc<AtomicUsize>,
    data: Option<Bytes>,
}
impl http_body::Body for ObservedBody {
    type Data = Bytes;
    type Error = tonic::Status;
    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(
            this.data
                .take()
                .map(|data| Ok(http_body::Frame::data(data))),
        )
    }
}
async fn metrics(
    router: &axum::Router,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())?,
        )
        .await?;
    Ok(String::from_utf8(
        response.into_body().collect().await?.to_bytes().to_vec(),
    )?)
}
#[tokio::test]
async fn grpc_rejects_before_body_poll_and_releases_failed_decode_admission() -> TestResult {
    let dir = tempfile::tempdir()?;
    let service = service(
        &dir.path().join("state.db"),
        Limits {
            max_concurrency: 1,
            max_request_bytes: 1024,
            request_memory_bytes: 1024,
            ..Limits::default()
        },
    )?;
    let polls = Arc::new(AtomicUsize::new(0));
    for authenticated in [false, true] {
        // A gRPC frame declaring a message larger than the configured limit.
        let observed = ObservedBody {
            polls: polls.clone(),
            data: Some(Bytes::from_static(&[0, 0, 0, 8, 0])),
        };
        let mut req = Request::builder()
            .method("POST")
            .uri("/geoledger.v1.GeoLedger/CreateProject")
            .header("content-type", "application/grpc");
        if authenticated {
            req = req.header("authorization", format!("Bearer {TOKEN}"));
        }
        let response = geoledger_server::grpc(service.clone())
            .oneshot(req.body(tonic::body::Body::new(observed))?)
            .await?;
        assert_eq!(
            response
                .headers()
                .get("grpc-status")
                .and_then(|v| v.to_str().ok()),
            Some(if authenticated { "8" } else { "16" })
        );
        if !authenticated {
            assert_eq!(polls.load(Ordering::SeqCst), 0);
        }
        drop(response);
    }
    let router = geoledger_server::router(service);
    let response = router.oneshot(request("list_projects", json!({}))?).await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}
#[tokio::test]
async fn unconsumed_http_response_holds_and_releases_its_budget() -> TestResult {
    let dir = tempfile::tempdir()?;
    let router = geoledger_server::router(service(
        &dir.path().join("state.db"),
        Limits {
            max_response_bytes: 1024,
            response_memory_bytes: 1024,
            ..Limits::default()
        },
    )?);
    let first = router
        .clone()
        .oneshot(request("list_projects", json!({}))?)
        .await?;
    assert_eq!(first.status(), StatusCode::OK);
    let rejected = router
        .clone()
        .oneshot(request("list_projects", json!({}))?)
        .await?;
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    drop((first, rejected));
    assert_eq!(
        router
            .oneshot(request("list_projects", json!({}))?)
            .await?
            .status(),
        StatusCode::OK
    );
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_waiter_does_not_hide_running_or_completed_operation() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("state.db");
    let router = geoledger_server::router(service(
        &path,
        Limits {
            max_concurrency: 1,
            ..Limits::default()
        },
    )?);
    let control = rusqlite::Connection::open(&path)?;
    control.execute_batch("BEGIN IMMEDIATE")?;
    let r = router.clone();
    let req = request("create_project", json!({"name":"detached"}))?;
    let task = tokio::spawn(async move { r.oneshot(req).await });
    // Do not cancel until the actual blocking operation has entered.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if metrics(&router)
                .await?
                .contains("geoledger_in_flight_requests 1\n")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    task.abort();
    assert!(task.await.is_err_and(|e| e.is_cancelled()));
    assert!(
        metrics(&router)
            .await?
            .contains("geoledger_in_flight_requests 1\n")
    );
    control.execute_batch("ROLLBACK")?;
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            let text = metrics(&router).await?;
            if text.contains("geoledger_in_flight_requests 0\n") {
                assert!(text.contains("geoledger_operations_total{protocol=\"http\",operation=\"create_project\",status=\"200\"} 1"),"{text}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }).await??;
    let count: i64 = control.query_row("SELECT count(*) FROM gl_projects", [], |r| r.get(0))?;
    assert_eq!(count, 1);
    Ok(())
}

#[tokio::test]
async fn stream_budget_survives_slow_consumer_and_releases_at_eof_or_drop() -> TestResult {
    use geoledger_rpc::v1::{self as pb, geo_ledger_server::GeoLedger};
    use tokio_stream::StreamExt;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("stream.db");
    let app = Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    let project = app.execute("owner", "create_project", json!({"name":"stream"}))?["project"]
        .as_str()
        .ok_or("project")?
        .to_owned();
    let dataset = app.execute(
        "owner",
        "create_dataset",
        json!({"project":project,"name":"points","geometry_type":"point"}),
    )?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    let service = service(
        &path,
        Limits {
            max_response_bytes: 1024,
            response_memory_bytes: 1024,
            ..Limits::default()
        },
    )?;
    let query = || -> Result<tonic::Request<pb::FeaturesRequest>,Box<dyn std::error::Error+Send+Sync>> {
        let mut r = tonic::Request::new(pb::FeaturesRequest { project:project.clone(),dataset:dataset.clone(), ..Default::default() });
        r.metadata_mut().insert("authorization",format!("Bearer {TOKEN}").parse()?);
        Ok(r)
    };
    let mut stream = service.features_stream(query()?).await?.into_inner();
    let error = service
        .features_stream(query()?)
        .await
        .err()
        .ok_or("second stream must be refused")?;
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
    while let Some(chunk) = stream.next().await {
        chunk?;
    }
    // The exhausted stream is still alive: reaching EOF must release its resources.
    let another = service.features_stream(query()?).await?.into_inner();
    drop(another);
    let recovered = service.features_stream(query()?).await?.into_inner();
    drop((recovered, stream));
    Ok(())
}

#[tokio::test]
async fn tls_grpc_admission_preserves_ip_rate_limits() -> TestResult {
    use geoledger_rpc::v1 as pb;
    use geoledger_server::{
        Rate,
        tls::{Tls, TlsFiles, spawn_acceptor},
    };
    use tokio_stream::StreamExt;
    let dir = tempfile::tempdir()?;
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let pem = cert.cert.pem();
    let files = TlsFiles {
        cert: dir.path().join("cert.pem"),
        key: dir.path().join("key.pem"),
        client_ca: None,
    };
    std::fs::write(&files.cert, &pem)?;
    std::fs::write(&files.key, cert.key_pair.serialize_pem())?;
    let service = service(
        &dir.path().join("state.db"),
        Limits {
            per_ip: Rate::new(0.001, 1.0),
            ..Limits::default()
        },
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::ReceiverStream::new(spawn_acceptor(
        listener,
        Tls::load(files)?,
        true,
    ))
    .map(|(stream, _)| Ok::<_, std::io::Error>(stream));
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(incoming)
            .await
    });
    let channel = tonic::transport::Endpoint::from_shared(format!("https://{address}"))?
        .tls_config(
            tonic::transport::ClientTlsConfig::new()
                .domain_name("localhost")
                .ca_certificate(tonic::transport::Certificate::from_pem(pem)),
        )?
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(3))
        .connect()
        .await?;
    let mut client = pb::geo_ledger_client::GeoLedgerClient::new(channel);
    let request =
        || -> Result<tonic::Request<pb::Empty>, Box<dyn std::error::Error + Send + Sync>> {
            let mut request = tonic::Request::new(pb::Empty {});
            request
                .metadata_mut()
                .insert("authorization", format!("Bearer {TOKEN}").parse()?);
            Ok(request)
        };
    client.info(request()?).await?;
    let error = client
        .info(request()?)
        .await
        .err()
        .ok_or("TLS peer lost before IP rate limiting")?;
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
    servers.abort_all();
    Ok(())
}

#[tokio::test]
async fn unconsumed_error_response_holds_transfer_budget() -> TestResult {
    let dir = tempfile::tempdir()?;
    let router = geoledger_server::router(service(
        &dir.path().join("errors.db"),
        Limits {
            max_response_bytes: 1024,
            response_memory_bytes: 1024,
            ..Limits::default()
        },
    )?);
    let first = router
        .clone()
        .oneshot(request(
            "get_project",
            json!({"project":"00000000-0000-4000-8000-000000000000"}),
        )?)
        .await?;
    assert_eq!(first.status(), StatusCode::NOT_FOUND);
    let refused = router
        .clone()
        .oneshot(request("list_projects", json!({}))?)
        .await?;
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    drop((first, refused));
    assert_eq!(
        router
            .oneshot(request("list_projects", json!({}))?)
            .await?
            .status(),
        StatusCode::OK
    );
    Ok(())
}
