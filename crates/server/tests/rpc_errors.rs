//! Exercise error contracts over real HTTP/2 with a bounded metadata budget.
use geoledger_rpc::v1 as pb;
use geoledger_server::{Application, Authentication, Authenticator, Service, Storage, Tokens};
use prost::Message;
use serde_json::json;
use tonic::{Request, transport::Endpoint};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn authenticated<T>(body: T) -> Result<Request<T>, Box<dyn std::error::Error + Send + Sync>> {
    let mut request = Request::new(body);
    // Public, test-only credential for this ephemeral in-process server.
    request.metadata_mut().insert(
        "authorization",
        "Bearer test-only-credential-not-for-deployment-123456789".parse()?,
    );
    Ok(request)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_conflicts_fit_trailers_and_keep_paginated_details() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("errors.sqlite3")));
    app.migrate()?;
    let project = app.execute("review", "create_project", json!({"name":"errors"}))?["project"]
        .as_str()
        .ok_or("project")?
        .to_owned();
    let dataset = app.execute(
        "review",
        "create_dataset",
        json!({"geometry_type":"point","project":project,"name":"features"}),
    )?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    let mut workspaces = Vec::new();
    for value in ["a", "b"] {
        let workspace =
            app.execute("review", "create_workspace", json!({"project":project}))?["workspace"]
                .as_str()
                .ok_or("workspace")?
                .to_owned();
        let edits: Vec<_> = (0..20)
            .map(|i| {
                let id = format!("feature-{i:02}");
                json!({"dataset":dataset,"feature_id":id,"feature":{"type":"Feature","id":id,"geometry":null,"properties":{"content":value.repeat(7000)}}})
            })
            .collect();
        app.execute("review", "save", json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"edits":edits}))?;
        workspaces.push(workspace);
    }
    app.execute("review", "publish", json!({"project":project,"workspace":workspaces[0],"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"first"}))?;

    let tokens = Tokens::from_json(
        format!(
            r#"[{{"subject":"review","token_sha256":"{}"}}]"#,
            geoledger_server::sha256_hex("test-only-credential-not-for-deployment-123456789")
        )
        .as_bytes(),
    )?;
    let service = Service::new(app, Authentication::new(Authenticator::Tokens(tokens)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let channel = Endpoint::from_shared(endpoint)?
        .http2_max_header_list_size(8192)
        .connect()
        .await?;
    let mut client = pb::geo_ledger_client::GeoLedgerClient::new(channel);
    let error = client
        .publish(authenticated(pb::PublishRequest {
            project: project.clone(),
            workspace: workspaces[1].clone(),
            expected_workspace_version: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            message: "conflicting publication".into(),
        })?)
        .await
        .err()
        .ok_or("publication must conflict")?;
    assert_eq!(error.code(), tonic::Code::Aborted);
    assert!(error.details().len() < 4096);
    let envelope = pb::RpcStatus::decode(error.details())?;
    let detail = pb::ErrorDetail::decode(
        envelope
            .details
            .first()
            .ok_or("error detail")?
            .value
            .as_slice(),
    )?;
    assert_eq!(detail.code, "conflict");
    assert!(uuid::Uuid::parse_str(&detail.request_id).is_ok());
    let conflicts = detail.conflicts.ok_or("conflict summary")?;
    assert_eq!(
        (conflicts.head, conflicts.version, conflicts.total),
        (1, 1, 20)
    );
    assert!(conflicts.truncated);
    assert!(conflicts.conflicts.is_empty());
    assert_eq!(conflicts.next_after, None);

    let page = client
        .conflicts(authenticated(pb::DiffRequest {
            project: project.clone(),
            workspace: workspaces[1].clone(),
            after: String::new(),
            limit: Some(5),
        })?)
        .await?
        .into_inner();
    assert_eq!(page.total, 20);
    assert_eq!(page.conflicts.len(), 5);
    assert!(page.truncated);
    assert!(
        page.conflicts[0]
            .draft
            .as_ref()
            .ok_or("draft")?
            .geojson
            .contains(&"b".repeat(7000))
    );
    let next = client
        .conflicts(authenticated(pb::DiffRequest {
            project: project.clone(),
            workspace: workspaces[1].clone(),
            after: page.next_after.ok_or("cursor")?,
            limit: Some(5),
        })?)
        .await?
        .into_inner();
    assert_eq!(next.conflicts.len(), 5);
    assert_ne!(page.conflicts[0].feature_id, next.conflicts[0].feature_id);
    assert_eq!(
        client
            .get_project(authenticated(pb::ProjectRequest {
                project: project.clone()
            })?)
            .await?
            .into_inner()
            .head,
        1
    );
    let duplicate = client
        .create_dataset(authenticated(pb::DatasetRequest {
            coordinate_dimension: 2,
            geometry_type: "point".into(),
            project,
            name: "features".into(),
        })?)
        .await
        .err()
        .ok_or("duplicate")?;
    assert_eq!(duplicate.code(), tonic::Code::Aborted);
    servers.abort_all();
    Ok(())
}
