use geoledger_client::{Client, FeatureQuery, Page};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    time::Duration,
};
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
struct Server {
    child: Child,
    http: SocketAddr,
    grpc: SocketAddr,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn start(dir: &std::path::Path) -> Result<Server, Box<dyn std::error::Error + Send + Sync>> {
    let h = TcpListener::bind("127.0.0.1:0")?;
    let g = TcpListener::bind("127.0.0.1:0")?;
    let http = h.local_addr()?;
    let grpc = g.local_addr()?;
    drop((h, g));
    let child = Command::new(env!("CARGO_BIN_EXE_geoledger-server"))
        .args([
            "--data-dir",
            dir.to_str().ok_or("path")?,
            "--http",
            &http.to_string(),
            "--grpc",
            &grpc.to_string(),
        ])
        .env_remove("GL_STORAGE")
        .env_remove("GL_DATABASE_URL")
        .env_remove("GL_TOKEN_FILE")
        .env_remove("GL_JWKS_FILE")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Server { child, http, grpc })
}
async fn connect(
    s: &Server,
    token: &str,
) -> Result<Client, Box<dyn std::error::Error + Send + Sync>> {
    for _ in 0..100 {
        if let Ok(c) = Client::connect(format!("http://{}", s.grpc), token).await {
            return Ok(c);
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    Err("server did not start".into())
}
fn http(
    address: SocketAddr,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> Result<(u16, String), Box<dyn std::error::Error + Send + Sync>> {
    let mut s = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let payload = body.unwrap_or("");
    write!(
        s,
        "{} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        if body.is_some() { "POST" } else { "GET" },
        payload.len()
    )?;
    let mut response = String::new();
    s.take(4 * 1024 * 1024).read_to_string(&mut response)?;
    let status = response
        .split_whitespace()
        .nth(1)
        .ok_or("status")?
        .parse()?;
    Ok((status, response))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_service_rpc_http_twenty_writers_restart_and_authentication() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut server = start(dir.path())?;
    let token_path = dir.path().join("tokens.json");
    for _ in 0..100 {
        if token_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let tokens: Value = serde_json::from_slice(&std::fs::read(token_path)?)?;
    let token = tokens[0]["token"].as_str().ok_or("token")?.to_owned();
    let client = connect(&server, &token).await?;
    assert_eq!(client.info().await?.backend, "sqlite");
    let bad = connect(&server, "invalid").await?;
    assert_eq!(
        bad.projects(Page::default())
            .await
            .err()
            .ok_or("authentication")?
            .code,
        "unauthenticated"
    );
    assert_eq!(
        http(server.http, "/api/v1/list_projects", None, Some("{}"))?.0,
        401
    );
    let (status, html) = http(server.http, "/", None, None)?;
    assert_eq!(status, 200);
    assert!(html.contains("content-security-policy"));
    assert!(html.contains("项目"));
    assert_eq!(http(server.http, "/ready", None, None)?.0, 200);
    assert_eq!(
        http(
            server.http,
            "/api/v1/create_project",
            Some(&token),
            Some(r#"{"name":"x","author":"forged"}"#)
        )?
        .0,
        400
    );
    let project = client.create_project("parallel").await?.id;
    let dataset = client.create_dataset(&project, "points").await?.id;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(20));
    let mut jobs = Vec::new();
    for i in 0..20 {
        let c = client.clone();
        let p = project.clone();
        let d = dataset.clone();
        let barrier = barrier.clone();
        jobs.push(tokio::spawn(async move {
            barrier.wait().await;
            let mut draft = c.create_workspace(&p).await?;
            let key = format!("f-{i:02}");
            draft.save(&d, json!({"type":"Feature","id":key,"properties":{"exact":18446744073709551615u64},"geometry":{"type":"Point","coordinates":[i,1,2]}})).await?;
            let result = draft.publish("concurrent").await?;
            assert_eq!(draft.publish("concurrent").await?, result);
            Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
 }));
    }
    for j in jobs {
        j.await??;
    }
    assert_eq!(client.project(&project).await?.head, 20);
    assert_eq!(
        client
            .features(&project, &dataset, FeatureQuery::default())
            .await?
            .features
            .len(),
        20
    );
    assert_eq!(
        http(
            server.http,
            "/api/v1/get_project",
            Some(&token),
            Some(&json!({"project":project}).to_string())
        )?
        .0,
        200
    );
    drop((client, bad));
    #[cfg(unix)]
    {
        assert!(
            Command::new("kill")
                .args(["-TERM", &server.child.id().to_string()])
                .status()?
                .success()
        );
        assert!(server.child.wait()?.success());
    }
    #[cfg(not(unix))]
    {
        server.child.kill()?;
        server.child.wait()?;
    }
    let restarted = start(dir.path())?;
    let client = connect(&restarted, &token).await?;
    assert_eq!(client.project(&project).await?.head, 20);
    Ok(())
}
