//! Real HTTP session lifecycle and gRPC API key admission share database state.
mod common;
use axum::{body::Body, extract::ConnectInfo, http::Request};
use geoledger_server::{Application, Service, Storage};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
const INITIAL: &str = "initial-test-password-123";
const PASSWORD: &str = "changed-test-password-456";
async fn call(
    router: &axum::Router,
    op: &str,
    session: Option<&str>,
    input: Value,
) -> Result<(u16, Value), Box<dyn std::error::Error + Send + Sync>> {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/auth/v1/{op}"))
        .extension(ConnectInfo(geoledger_server::tls::PeerAddr(
            "127.0.0.1:12345".parse()?,
        )))
        .header("content-type", "application/json");
    if let Some(session) = session {
        request = request.header("x-geoledger-session", session);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(input.to_string()))?)
        .await?;
    let status = response.status().as_u16();
    let body = response.into_body().collect().await?.to_bytes();
    Ok((status, serde_json::from_slice(&body)?))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passwords_sessions_keys_and_transport_invalidation() -> TestResult {
    let dir = common::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("auth.db")));
    app.migrate()?;
    assert!(app.bootstrap_admin("Admin", INITIAL)?);
    let service = Service::new(app.clone());
    let router = geoledger_server::router(service.clone());
    let (wrong, _) = call(
        &router,
        "login_password",
        None,
        json!({"username":"absent","password":"wrong"}),
    )
    .await?;
    assert_eq!(wrong, 401);
    let (_, login) = call(
        &router,
        "login_password",
        None,
        json!({"username":"ADMIN","password":INITIAL}),
    )
    .await?;
    assert_eq!(login["user"]["must_change_password"], true);
    let temporary = login["session"].as_str().ok_or("session")?;
    let (blocked, _) = call(
        &router,
        "create_api_key",
        Some(temporary),
        json!({"name":"blocked","current_password":INITIAL}),
    )
    .await?;
    assert_eq!(blocked, 403);
    let (status, changed) = call(
        &router,
        "change_password",
        Some(temporary),
        json!({"current_password":INITIAL,"new_password":PASSWORD}),
    )
    .await?;
    assert_eq!(status, 200);
    let session = changed["session"].as_str().ok_or("session")?;
    assert_ne!(session, temporary);
    assert!(app.authenticate_session(temporary).is_err());
    let (status, key) = call(
        &router,
        "create_api_key",
        Some(session),
        json!({"name":"automation","current_password":PASSWORD}),
    )
    .await?;
    assert_eq!(status, 200);
    let api_key = key["api_key"].as_str().ok_or("key")?;
    assert!(api_key.starts_with("glk_"));
    assert_eq!(api_key.len(), 68);
    let (status, list) = call(&router, "list_api_keys", Some(session), json!({})).await?;
    assert_eq!(status, 200);
    assert!(!list.to_string().contains(api_key));
    let (_, key_login) = call(&router, "login_api_key", None, json!({"api_key":api_key})).await?;
    let key_session = key_login["session"].as_str().ok_or("session")?;
    let (denied, _) = call(
        &router,
        "create_api_key",
        Some(key_session),
        json!({"name":"escalate"}),
    )
    .await?;
    assert_eq!(denied, 403);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let client = geoledger_client::Client::connect(endpoint, api_key).await?;
    assert!(client.info().await?.platform_admin);
    client.create_project("authenticated project").await?;
    let (status, _) = call(
        &router,
        "revoke_api_key",
        Some(session),
        json!({"key_id":key["credential"]["id"],"current_password":PASSWORD}),
    )
    .await?;
    assert_eq!(status, 200);
    assert_eq!(
        client.info().await.err().ok_or("revoked accepted")?.code,
        "unauthenticated"
    );
    let (status, _) = call(&router, "me", Some(key_session), json!({})).await?;
    assert_eq!(status, 401);
    let (status, _) = call(&router, "logout", Some(session), json!({})).await?;
    assert_eq!(status, 200);
    let (status, _) = call(&router, "me", Some(session), json!({})).await?;
    assert_eq!(status, 401);
    task.abort();
    Ok(())
}
#[tokio::test]
async fn login_rate_limit_and_duplicate_headers() -> TestResult {
    let dir = common::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("auth.db")));
    app.migrate()?;
    let router = geoledger_server::router(Service::new(app));
    let mut accepted = 0;
    for index in 0..200 {
        let request = Request::builder()
            .method("POST")
            .uri("/auth/v1/login_api_key")
            .extension(ConnectInfo(geoledger_server::tls::PeerAddr(
                "127.0.0.1:12345".parse()?,
            )))
            .header("content-type", "application/json")
            .header("x-forwarded-for", format!("192.0.2.{index}"))
            .body(Body::from(r#"{"api_key":"invalid"}"#))?;
        let status = router.clone().oneshot(request).await?.status();
        if status == 429 {
            break;
        }
        assert_eq!(status, 401);
        accepted += 1;
    }
    // Allow timer refill without making the test depend on exact CPU scheduling.
    assert!((100..200).contains(&accepted));
    let (status, _) = call(&router, "login_api_key", None, json!({"api_key":"invalid"})).await?;
    assert_eq!(status, 429);
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/info")
        .header("authorization", format!("Bearer {}", common::KEY))
        .header("authorization", format!("Bearer {}", common::KEY))
        .body(Body::from("{}"))?;
    assert_eq!(router.oneshot(request).await?.status(), 401);
    Ok(())
}

#[tokio::test]
async fn password_limits_normalize_names_and_isolate_accounts_at_one_peer() -> TestResult {
    let dir = common::tempdir()?;
    let path = dir.path().join("password-peer.db");
    let app = Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    app.bootstrap_admin("admin", INITIAL)?;
    for index in 0..20 {
        common::seed(
            &path,
            &[(
                &format!("person-{index}"),
                &format!("glk_{index:064x}"),
                false,
            )],
        )?;
    }
    // Reuse a real Argon2 hash for public test passwords; every login still verifies it.
    let db = rusqlite::Connection::open(&path)?;
    db.execute(
        "UPDATE gl_passwords SET password_hash=(SELECT password_hash FROM gl_passwords WHERE user_id=(SELECT id FROM gl_users WHERE username='admin'))",
        [],
    )?;
    drop(db);
    let router = geoledger_server::router(Service::new(app));
    // Existing and nonexistent accounts follow the same normalization and failure budget.
    for name in ["admin", "absent"] {
        let variants = [
            name.to_owned(),
            name.to_ascii_uppercase(),
            format!(" \t{name}\n"),
        ];
        let mut failures = 0;
        for index in 0..20 {
            let (status, result) = call(
                &router,
                "login_password",
                None,
                json!({"username": variants[index % 3], "password":"wrong"}),
            )
            .await?;
            if status == 429 {
                break;
            }
            assert_eq!(status, 401);
            assert_eq!(result["error"]["message"], "authentication failed");
            failures += 1;
        }
        assert!((10..20).contains(&failures));
        for variant in &variants {
            assert_eq!(
                call(
                    &router,
                    "login_password",
                    None,
                    json!({"username":variant,"password":INITIAL})
                )
                .await?
                .0,
                429
            );
        }
    }
    for index in 0..20 {
        assert_eq!(
            call(
                &router,
                "login_password",
                None,
                json!({"username":format!("person-{index}"),"password":INITIAL})
            )
            .await?
            .0,
            200
        );
        // Keys neither share a username bucket nor consume the password budget.
        assert_eq!(
            call(
                &router,
                "login_api_key",
                None,
                json!({"api_key":format!("glk_{index:064x}")})
            )
            .await?
            .0,
            200
        );
    }
    Ok(())
}

#[tokio::test]
async fn sensitive_limits_isolate_users_sharing_one_console_peer() -> TestResult {
    let dir = common::tempdir()?;
    let path = dir.path().join("shared-peer.db");
    let app = Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    let second_key = format!("glk_{}", "b".repeat(64));
    common::seed(
        &path,
        &[
            ("first", common::KEY, false),
            ("second", &second_key, false),
        ],
    )?;
    let router = geoledger_server::router(Service::new(app));
    let (_, first) = call(
        &router,
        "login_api_key",
        None,
        json!({"api_key":common::KEY}),
    )
    .await?;
    let (_, second) = call(
        &router,
        "login_api_key",
        None,
        json!({"api_key":second_key}),
    )
    .await?;
    let first = first["session"].as_str().ok_or("first session")?;
    let second = second["session"].as_str().ok_or("second session")?;
    for _ in 0..10 {
        let (status, _) = call(
            &router,
            "create_api_key",
            Some(first),
            json!({"name":"denied","current_password":"wrong"}),
        )
        .await?;
        assert_eq!(status, 403);
    }
    assert_eq!(
        call(
            &router,
            "create_api_key",
            Some(first),
            json!({"name":"denied","current_password":"wrong"})
        )
        .await?
        .0,
        429
    );
    assert_eq!(
        call(
            &router,
            "create_api_key",
            Some(second),
            json!({"name":"denied","current_password":"wrong"})
        )
        .await?
        .0,
        403
    );
    assert_eq!(call(&router, "me", Some(first), json!({})).await?.0, 200);
    // Account reauthentication attempts do not consume the shared peer's anonymous login budget.
    assert_eq!(
        call(
            &router,
            "login_api_key",
            None,
            json!({"api_key":common::KEY})
        )
        .await?
        .0,
        200
    );
    Ok(())
}
