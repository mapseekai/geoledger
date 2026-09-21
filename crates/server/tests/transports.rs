#![allow(clippy::unwrap_used)]
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use spatial_version::Application;
use spatial_version_server::{Service, grpc, http, proto};
use tower::ServiceExt;

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
    let init = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/commands")
                .method("POST")
                .header("authorization", "Bearer test-token-long-enough-for-tests")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"op":"init","author":"http-test"}"#))
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
        proto::spatial_version_client::SpatialVersionClient::connect(format!("http://{address}"))
            .await
            .unwrap();
    client
        .execute(proto::ExecuteRequest {
            command_json: r#"{"op":"init","author":"grpc-test"}"#.into(),
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
    tx.send(()).unwrap();
    handle.await.unwrap().unwrap();
}
#[test]
fn non_loopback_requires_authentication() {
    let config = spatial_version_server::ServerConfig {
        http: "0.0.0.0:7878".parse().unwrap(),
        grpc: "127.0.0.1:7879".parse().unwrap(),
        token: None,
    };
    assert!(config.validate().is_err());
}
