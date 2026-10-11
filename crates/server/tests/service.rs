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
    let token_path = dir.path().join("admin-credentials.json");
    for _ in 0..100 {
        if token_path.exists() && dir.path().join("tokens.json").exists() {
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
    let (status, discovery) = http(server.http, "/", None, None)?;
    assert_eq!(status, 200);
    assert!(discovery.contains("application/json"));
    assert!(discovery.contains("geoledger-server"));
    assert_eq!(http(server.http, "/logo.png", None, None)?.0, 404);
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
    let dataset = client
        .create_dataset_with_dimension(&project, "points", "point", 3)
        .await?
        .id;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_streams_and_http_preserve_atomic_saves_without_four_mib_cap() -> TestResult {
    use geoledger_rpc::v1 as pb;
    use prost::Message;
    let dir = tempfile::tempdir()?;
    let server = start(dir.path())?;
    let token_path = dir.path().join("admin-credentials.json");
    for _ in 0..100 {
        if token_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let tokens: Value = serde_json::from_slice(&std::fs::read(token_path)?)?;
    let token = tokens[0]["token"].as_str().ok_or("token")?;
    let client = connect(&server, token).await?;
    assert_eq!(client.info().await?.max_request_bytes, 64 * 1024 * 1024);
    let project = client.create_project("large streaming").await?.id;
    let dataset = client
        .create_dataset_with_dimension(&project, "large features", "point", 3)
        .await?
        .id;
    let workspace = client.create_workspace(&project).await?.info().id.clone();
    let feature = json!({"type":"Feature","id":"large","properties":{"payload":"x".repeat(5*1024*1024),"exact":18446744073709551615u64},"geometry":null});
    let save = pb::SaveRequest {
        project: project.clone(),
        workspace: workspace.clone(),
        expected_workspace_version: 0,
        edits: vec![pb::Edit {
            dataset: dataset.clone(),
            feature_id: "large".into(),
            feature: Some(pb::Feature {
                geojson: feature.to_string(),
            }),
        }],
    };
    let bytes = save.encode_to_vec();
    let auth: tonic::metadata::MetadataValue<_> = format!("Bearer {token}").parse()?;
    let mut rpc =
        pb::geo_ledger_client::GeoLedgerClient::connect(format!("http://{}", server.grpc)).await?;
    // Clean EOF with a valid protobuf prefix must still reject incomplete uploads.
    let mut incomplete = tonic::Request::new(tokio_stream::iter(vec![pb::DataChunk {
        data: bytes.clone(),
        total_bytes: bytes.len() as u64 + 1,
    }]));
    incomplete
        .metadata_mut()
        .insert("authorization", auth.clone());
    assert_eq!(
        rpc.save_stream(incomplete)
            .await
            .err()
            .ok_or("expected incomplete error")?
            .code(),
        tonic::Code::InvalidArgument
    );
    let mut version_request = tonic::Request::new(pb::WorkspaceRequest {
        project: project.clone(),
        workspace: workspace.clone(),
    });
    version_request
        .metadata_mut()
        .insert("authorization", auth.clone());
    assert_eq!(
        rpc.get_workspace(version_request)
            .await?
            .into_inner()
            .version,
        0
    );
    let chunks: Vec<_> = bytes
        .chunks(64 * 1024)
        .enumerate()
        .map(|(i, data)| pb::DataChunk {
            data: data.to_vec(),
            total_bytes: if i == 0 { bytes.len() as u64 } else { 0 },
        })
        .collect();
    let mut request = tonic::Request::new(tokio_stream::iter(chunks));
    request.metadata_mut().insert("authorization", auth.clone());
    assert_eq!(rpc.save_stream(request).await?.into_inner().version, 1);
    let query = pb::FeaturesRequest {
        project: project.clone(),
        dataset: dataset.clone(),
        workspace: Some(workspace.clone()),
        ..Default::default()
    };
    let mut request = tonic::Request::new(query);
    request.metadata_mut().insert("authorization", auth.clone());
    let mut stream = rpc.features_stream(request).await?.into_inner();
    let mut output = Vec::new();
    let mut total = None;
    let mut chunks = 0;
    while let Some(chunk) = stream.message().await? {
        assert!(chunk.data.len() <= 64 * 1024);
        if chunks == 0 {
            total = Some(chunk.total_bytes);
        } else {
            assert_eq!(chunk.total_bytes, 0);
        }
        output.extend_from_slice(&chunk.data);
        chunks += 1;
    }
    assert!(chunks > 80);
    assert_eq!(total, Some(output.len() as u64));
    let reply = pb::FeaturesReply::decode(output.as_slice())?;
    assert_eq!(reply.workspace_version, Some(1));
    assert_eq!(
        serde_json::from_str::<Value>(&reply.features[0].geojson)?,
        feature
    );
    // The ordinary HTTP transport and Rust SDK also accept large payloads.
    let request = json!({"project":project,"workspace":workspace,"expected_workspace_version":1,"edits":[{"dataset":dataset,"feature_id":"large","feature":feature}]}).to_string();
    assert!(request.len() > 4 * 1024 * 1024);
    assert_eq!(
        http(server.http, "/api/v1/save", Some(token), Some(&request))?.0,
        200
    );
    let page = client
        .features(
            &project,
            &dataset,
            FeatureQuery {
                workspace: Some(workspace),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(page.features[0], feature);
    Ok(())
}
