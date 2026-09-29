#![allow(clippy::unwrap_used)]
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use geoledger::Application;
use geoledger_server::{Service, grpc, http, proto};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn http_extractor_errors_share_envelope_status_and_correlation() {
    let directory = tempfile::tempdir().unwrap();
    let router = http::router(Service::new(Application::new(directory.path()), None));
    let cases = [
        (
            "/v1/commands",
            "POST",
            Some("application/json"),
            "{".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/commands",
            "POST",
            Some("application/json"),
            r#"{"op":"status","bogus":1}"#.into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/v1/commands",
            "POST",
            None,
            "{}".into(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            "/v1/commands",
            "POST",
            Some("application/json"),
            " ".repeat(geoledger_server::MAX_REQUEST_BYTES + 1),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "/v1/status?limit=secret-value",
            "GET",
            None,
            String::new(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/log?limit=bad",
            "GET",
            None,
            String::new(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/conflicts?limit=bad",
            "GET",
            None,
            String::new(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/branches",
            "POST",
            Some("application/json"),
            "{}".into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/v1/commits",
            "POST",
            Some("application/json"),
            "{}".into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/v1/merges",
            "POST",
            Some("application/json"),
            "{}".into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    let mut ids = std::collections::HashSet::new();
    for (uri, method, content_type, body, expected) in cases {
        let mut builder = Request::builder()
            .uri(uri)
            .method(method)
            .header("x-request-id", "untrusted-secret");
        if let Some(content_type) = content_type {
            builder = builder.header("content-type", content_type);
        }
        let response = router
            .clone()
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{uri}");
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(ids.insert(id.clone()));
        assert_ne!(id, "untrusted-secret");
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["request_id"], id);
        assert!(value["error"]["code"].is_string());
        assert!(value["error"]["message"].is_string());
        assert!(!String::from_utf8_lossy(&bytes).contains("secret"));
    }
    assert!(!directory.path().join(".geoledger").exists());
}

#[tokio::test]
async fn http_backend_failure_is_redacted_and_correlated() {
    let directory = tempfile::tempdir().unwrap();
    let storage = directory.path().join(".geoledger");
    std::fs::create_dir(&storage).unwrap();
    std::fs::write(
        storage.join("repository.sqlite"),
        b"secret-credential: corrupt database",
    )
    .unwrap();
    let router = http::router(Service::new(Application::new(directory.path()), None));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/v1/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["error"]["code"], "storage_error");
    assert_eq!(value["error"]["request_id"], request_id);
    let body = String::from_utf8_lossy(&bytes);
    assert!(!body.contains("secret-credential"));
    assert!(!body.contains("private-storage-path"));
}

#[tokio::test]
async fn http_executes_shared_application_and_requires_token() {
    let dir = tempfile::tempdir().unwrap();
    let router = http::router(Service::new(
        Application::new(dir.path()),
        Some("test-token-long-enough-for-tests".into()),
    ));
    let unauthorized = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let request_id = unauthorized.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = unauthorized.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["error"]["code"], "unauthenticated");
    assert_eq!(error["error"]["request_id"], request_id);
    let init = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/commands")
                .method("POST")
                .header("authorization", "Bearer test-token-long-enough-for-tests")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"op":"init"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(init.status(), StatusCode::OK);
    let status = router
        .oneshot(
            Request::builder()
                .uri("/v1/status")
                .header("authorization", "Bearer test-token-long-enough-for-tests")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = status.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["clean"], true);
}
#[tokio::test]
async fn grpc_roundtrip_uses_real_http2_and_typed_methods() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::new(Application::new(dir.path()), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(grpc::server(service))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = rx.await;
                },
            ),
    );
    let mut client =
        proto::geo_ledger_client::GeoLedgerClient::connect(format!("http://{address}"))
            .await
            .unwrap();
    client
        .execute(proto::ExecuteRequest {
            command_json: r#"{"op":"init"}"#.into(),
        })
        .await
        .unwrap();
    client
        .branch(proto::BranchRequest {
            name: "rpc-draft".into(),
            from: "HEAD".into(),
        })
        .await
        .unwrap();
    let reply = client
        .status(proto::StatusRequest { limit: 10 })
        .await
        .unwrap()
        .into_inner();
    let json: serde_json::Value = serde_json::from_str(&reply.json).unwrap();
    assert_eq!(json["branch"], "main");
    let reply = client
        .log(proto::LogRequest {
            reference: "rpc-draft".into(),
            limit: 10,
        })
        .await
        .unwrap()
        .into_inner();
    let json: serde_json::Value = serde_json::from_str(&reply.json).unwrap();
    assert_eq!(json["commits"].as_array().unwrap().len(), 1);
    assert_eq!(json["commits"][0]["commit"]["author"], "mapseekai");
    tx.send(()).unwrap();
    handle.await.unwrap().unwrap();
}
#[test]
fn non_loopback_requires_authentication() {
    let config = geoledger_server::ServerConfig {
        http: "0.0.0.0:7878".parse().unwrap(),
        grpc: "127.0.0.1:7879".parse().unwrap(),
        token: None,
    };
    assert!(config.validate().is_err());
}

#[tokio::test(start_paused = true)]
async fn http_admission_covers_body_reads_and_timeout_releases_capacity() {
    use tokio_stream::StreamExt;
    let directory = tempfile::tempdir().unwrap();
    let service = http::router(Service::new(
        Application::new(directory.path()),
        Some("secret".into()),
    ));
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let tx = tx.clone();
        let stream = tokio_stream::once(axum::body::Bytes::from_static(b"{ "))
            .chain(tokio_stream::pending())
            .map(move |bytes| {
                tx.try_send(()).unwrap();
                Ok::<_, std::io::Error>(bytes)
            });
        let request = Request::post("/v1/commands")
            .header("content-type", "application/json")
            .header("authorization", "Bearer secret")
            .body(Body::from_stream(stream))
            .unwrap();
        tasks.push(tokio::spawn(service.clone().oneshot(request)));
    }
    for _ in 0..8 {
        rx.recv().await.unwrap();
    }
    let request = |auth| {
        Request::post("/v1/commands")
            .header("content-type", "application/json")
            .header("authorization", auth)
            .body(Body::from("{"))
            .unwrap()
    };
    assert_eq!(
        service
            .clone()
            .oneshot(request("Bearer forged"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        service
            .clone()
            .oneshot(request("Bearer secret"))
            .await
            .unwrap()
            .status(),
        StatusCode::LOCKED
    );
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    for task in tasks {
        let response = task.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "deadline_exceeded");
    }
    // Reaching JSON validation proves ingress capacity was recovered.
    assert_eq!(
        service
            .oneshot(request("Bearer secret"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}
