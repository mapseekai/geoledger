//! Real gRPC and HTTP credential lifecycle; fixtures contain no deployable credentials.
use geoledger_client::{Client, Page};
use geoledger_server::{
    Application, Authentication, Authenticator, Policy, Service, Storage, Tokens,
};
use serde_json::json;
use uuid::Uuid;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
const ROOT: &str = "test-only-root-token-credentials-integration";
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_credentials_authenticate_and_revoke_across_transports() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("db"))).with_policy(Policy {
        admins: ["root".into()].into(),
        ..Default::default()
    });
    app.migrate()?;
    let tokens = Tokens::from_json(
        json!([{"subject":"root","token_sha256":geoledger_server::sha256_hex(ROOT)}])
            .to_string()
            .as_bytes(),
    )?;
    let authentication = std::sync::Arc::new(Authentication::new(Authenticator::Tokens(tokens)));
    let service = Service::with_authentication(app, authentication.clone());
    let grpc = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", grpc.local_addr()?);
    let http_url = format!("http://{}/api/v1", http.local_addr()?);
    let router = geoledger_server::router(service.clone());
    let h = tokio::spawn(async move { axum::serve(http, router).await });
    let g = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(grpc))
            .await
    });
    let root = Client::connect(&endpoint, ROOT).await?;
    assert!(root.info().await?.platform_admin);
    let id = Uuid::new_v4().to_string();
    let issued = root
        .issue_token(&id, "alice", "work", "2099-01-01T00:00:00Z")
        .await?;
    let secret = issued.token.ok_or("missing plaintext")?;
    assert!(
        root.issue_token(&id, "alice", "work", "2099-01-01T00:00:00Z")
            .await?
            .token
            .is_none()
    );
    let alice = Client::connect(&endpoint, &secret).await?;
    assert!(!alice.info().await?.platform_admin);
    assert!(alice.tokens(Page::default()).await.is_err());
    let project = root.create_project("private").await?;
    assert!(alice.project(&project.id).await.is_err());
    root.set_member(&project.id, "alice", "viewer").await?;
    assert_eq!(alice.project(&project.id).await?.role, "viewer");
    assert!(alice.create_workspace(&project.id).await.is_err());
    let http_client = reqwest::Client::new();
    let response = http_client
        .post(format!("{http_url}/revoke_token"))
        .bearer_auth(ROOT)
        .body(json!({"token_id":issued.credential.id}).to_string())
        .send()
        .await?;
    assert!(response.status().is_success());
    assert_eq!(
        alice.info().await.err().ok_or("revocation ignored")?.code,
        "unauthenticated"
    );
    assert_eq!(
        http_client
            .post(format!("{http_url}/info"))
            .bearer_auth(&secret)
            .body("{}")
            .send()
            .await?
            .status(),
        401
    );
    let next = root
        .issue_token(
            &Uuid::new_v4().to_string(),
            "alice",
            "second",
            "2099-01-01T00:00:00Z",
        )
        .await?;
    let replacement = root
        .reissue_token(
            &Uuid::new_v4().to_string(),
            &next.credential.id,
            "replacement",
            "2099-01-01T00:00:00Z",
        )
        .await?;
    assert!(
        Client::connect(&endpoint, &next.token.ok_or("next")?)
            .await?
            .info()
            .await
            .is_err()
    );
    assert!(
        Client::connect(&endpoint, &replacement.token.ok_or("replacement")?)
            .await?
            .info()
            .await
            .is_ok()
    );
    // File credentials must never reactivate a revoked managed credential.
    authentication.replace(Authenticator::Tokens(Tokens::from_json(
        json!([{"subject":"root","token_sha256":geoledger_server::sha256_hex(&secret)}])
            .to_string()
            .as_bytes(),
    )?));
    assert_eq!(
        alice.info().await.err().ok_or("file fallback bypass")?.code,
        "unauthenticated"
    );
    // JWT mode advertises the feature as unavailable and rejects managed credentials.
    authentication.replace(Authenticator::Jwt(Box::new(
        geoledger_server::JwtAuthenticator::from_jwks(
            include_bytes!("fixtures/jwks.json"),
            "https://issuer.example",
            "geoledger",
        )?,
    )));
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-only".into());
    let jwt = jsonwebtoken::encode(
        &header,
        &json!({"iss":"https://issuer.example","aud":"geoledger","sub":"root","exp":4102444800_i64}),
        &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("fixtures/jwt-test-only.pem"))?,
    )?;
    let jwt_client = Client::connect(&endpoint, &jwt).await?;
    assert!(!jwt_client.info().await?.managed_tokens);
    assert!(
        jwt_client
            .issue_token(
                &Uuid::new_v4().to_string(),
                "alice",
                "jwt-disabled",
                "2099-01-01T00:00:00Z"
            )
            .await
            .is_err()
    );
    assert!(alice.info().await.is_err());
    h.abort();
    g.abort();
    Ok(())
}
