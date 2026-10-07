//! Disposable loopback protocol stubs; never reads a DSN or credentials.
#![cfg(feature = "http")]
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use geoledger_center::{CenterApplication, Tokens, router};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn stalled(query: bool, tls: bool) -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let server = std::thread::spawn(move || -> TestResult {
        std::thread::scope(|scope| -> TestResult {
            let mut workers = Vec::new();
            let accept_deadline = Instant::now() + Duration::from_secs(5);
            for _ in 0..17 {
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < accept_deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(1))
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                workers.push(scope.spawn(move || -> TestResult {
                    // Accepted sockets may inherit nonblocking mode on some OSes.
                    socket.set_nonblocking(false)?;
                    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
                    let mut length = [0; 4];
                    socket.read_exact(&mut length)?;
                    let n = u32::from_be_bytes(length) as usize;
                    if !(8..=4096).contains(&n) {
                        return Err("invalid startup size".into());
                    }
                    let mut startup = vec![0; n - 4];
                    socket.read_exact(&mut startup)?;
                    if tls {
                        socket.write_all(b"S")?;
                    } else if query {
                        socket.write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I")?;
                    }
                    // Both orderly EOF and a TCP reset prove peer closure. Linux
                    // may reset a socket closed with unread query/TLS bytes.
                    // TimedOut/WouldBlock must still fail: they mean it stayed open.
                    let mut buffer = [0; 4096];
                    loop {
                        match socket.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(_) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {
                                break;
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
                    Ok(())
                }));
            }
            for worker in workers {
                worker.join().map_err(|_| "stub panicked")??;
            }
            Ok(())
        })
    });
    let app = CenterApplication::new(format!(
        "host=127.0.0.1 port={} user=stub dbname=stub sslmode={}",
        address.port(),
        if tls { "require" } else { "disable" }
    ))
    .with_timeout(Duration::from_millis(500));
    let tokens = Tokens::from_json(
        serde_json::json!([{"token":"a".repeat(43),"subject":"alice"}])
            .to_string()
            .as_bytes(),
    )?;
    let service = router(app, tokens);
    let request = || {
        Request::post("/api/center/list_projects")
            .header("authorization", format!("Bearer {}", "a".repeat(43)))
            .body(Body::from("{}"))
    };
    let started = Instant::now();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let service = service.clone();
        let request = request()?;
        tasks.spawn(async move { service.oneshot(request).await });
    }
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result??.status(), StatusCode::GATEWAY_TIMEOUT);
    }
    // All capacity recovered after timeout: this must reach the database too.
    assert_eq!(
        service.oneshot(request()?).await?.status(),
        StatusCode::GATEWAY_TIMEOUT
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    server.join().map_err(|_| "stub panicked")??;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_startup_closes_sockets_and_recovers_all_permits() -> TestResult {
    stalled(false, false).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_query_closes_sockets_and_recovers_all_permits() -> TestResult {
    stalled(true, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_tls_closes_sockets_and_recovers_all_permits() -> TestResult {
    stalled(false, true).await
}

#[tokio::test]
async fn rejects_oversized_integer_before_database_connection() -> TestResult {
    use http_body_util::BodyExt;
    let tokens = Tokens::from_json(
        serde_json::json!([{"token":"a".repeat(43),"subject":"alice"}])
            .to_string()
            .as_bytes(),
    )?;
    let service = router(CenterApplication::new("invalid dsn".into()), tokens);
    let response = service
        .oneshot(
            Request::post("/api/center/save")
                .header("authorization", format!("Bearer {}", "a".repeat(43)))
                .body(Body::from(
                    r#"{"edits":[{"feature":{"properties":{"n":18446744073709551617}}}]}"#,
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await?.to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(body["error"]["code"], "invalid_argument");
    assert!(body["error"]["request_id"].is_string());
    Ok(())
}
