mod common;
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
    common::database(&dir.join("geoledger.sqlite3"))?;
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
    let dir = common::tempdir()?;
    let mut server = start(dir.path())?;
    let token = common::KEY.to_owned();
    let client = connect(&server, &token).await?;
    assert_eq!(client.info().await?.backend, "sqlite");
    let bad = connect(
        &server,
        "glk_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .await?;
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
    let dir = common::tempdir()?;
    let server = start(dir.path())?;
    let token = common::KEY;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn query_pages_match_http_and_grpc_and_audit_stays_owner_only() -> TestResult {
    let dir = common::tempdir()?;
    common::database(&dir.path().join("geoledger.sqlite3"))?;
    let outsider_key = "glk_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    common::seed(
        &dir.path().join("geoledger.sqlite3"),
        &[("other-admin", outsider_key, true)],
    )?;
    let server = start(dir.path())?;
    let client = connect(&server, common::KEY).await?;
    let other_admin = connect(&server, outsider_key).await?;
    assert!(other_admin.info().await?.platform_admin);
    let project = client.create_project("query pages").await?.id;
    let dataset = client.create_dataset(&project, "points", "point").await?.id;
    let read_http = |operation: &str,
                     mut request: Value|
     -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        request["project"] = json!(project);
        let (status, response) = http(
            server.http,
            &format!("/api/v1/{operation}"),
            Some(common::KEY),
            Some(&request.to_string()),
        )?;
        assert_eq!(status, 200, "{operation}: {response}");
        Ok(serde_json::from_str(
            response.split_once("\r\n\r\n").ok_or("HTTP body")?.1,
        )?)
    };
    assert!(
        client
            .history(&project, 0, Some(2))
            .await?
            .commits
            .is_empty()
    );
    assert_eq!(
        read_http("history", json!({"limit":2}))?,
        json!({"commits":[],"next_after":null})
    );
    let mut draft = client.create_workspace(&project).await?;
    for key in ["a/1", "b/2"] {
        draft.save(&dataset, json!({"type":"Feature","id":key,"properties":{"exact":9007199254740993_u64},"geometry":null})).await?;
    }
    for limit in [1, 2, 3] {
        let page = draft
            .diff(Page {
                limit: Some(limit),
                ..Default::default()
            })
            .await?;
        let raw = read_http("diff", json!({"workspace":draft.info().id,"limit":limit}))?;
        assert_eq!(
            raw["changes"].as_array().ok_or("changes")?.len(),
            page.changes.len()
        );
        assert_eq!(raw["next_after"], json!(page.next_after));
        assert_eq!(page.next_after.is_some(), limit == 1);
        if let Some(after) = page.next_after {
            let terminal = draft
                .diff(Page {
                    after,
                    limit: Some(limit),
                })
                .await?;
            assert_eq!(terminal.changes[0].feature_id, "b/2");
            assert!(terminal.next_after.is_none());
        }
    }
    let receipt = draft.publish("first page").await?;
    for limit in [1, 2, 3] {
        let page = client
            .commit(
                &project,
                receipt.revision,
                Page {
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
        let raw = read_http("commit", json!({"revision":receipt.revision,"limit":limit}))?;
        assert_eq!(raw["next_after"], json!(page.next_after));
        assert_eq!(page.next_after.is_some(), limit == 1);
    }
    let mut second = client.create_workspace(&project).await?;
    second
        .save(
            &dataset,
            json!({"type":"Feature","id":"c","properties":{},"geometry":null}),
        )
        .await?;
    second.publish("second page").await?;
    for limit in [1, 2, 3] {
        let page = client.history(&project, 0, Some(limit)).await?;
        let raw = read_http("history", json!({"limit":limit}))?;
        assert_eq!(raw["next_after"], json!(page.next_after));
        assert_eq!(page.next_after, if limit == 1 { Some(1) } else { None });
        assert_eq!(page.commits[0].revision, 1);
    }
    let next = client.history(&project, 1, Some(1)).await?;
    assert_eq!(next.commits[0].revision, 2);
    assert!(next.next_after.is_none());
    assert!(
        client
            .history(&project, 2, Some(1))
            .await?
            .commits
            .is_empty()
    );
    let events = client.audit(&project, 0, Some(1000)).await?.events;
    let count = events.len() as i64;
    for limit in [1, count, count + 1] {
        let page = client.audit(&project, 0, Some(limit)).await?;
        let raw = read_http("audit", json!({"limit":limit}))?;
        assert_eq!(raw["next_after"], json!(page.next_after));
        assert_eq!(page.next_after.is_some(), limit < count);
    }
    let last = events.last().ok_or("last audit event")?.id;
    let terminal = client.audit(&project, last, Some(2)).await?;
    assert!(terminal.events.is_empty());
    assert!(terminal.next_after.is_none());
    assert_eq!(
        other_admin
            .audit(&project, 0, Some(1))
            .await
            .err()
            .ok_or("audit denied")?
            .code,
        "not_found"
    );
    assert_eq!(
        http(
            server.http,
            "/api/v1/audit",
            Some(outsider_key),
            Some(&json!({"project":project,"limit":1}).to_string())
        )?
        .0,
        404
    );
    Ok(())
}
