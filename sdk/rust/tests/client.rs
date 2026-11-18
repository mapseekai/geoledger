use geoledger_client::{Client, ConnectOptions, FeatureQuery, MessageLimits, PostgisTable};
use geoledger_rpc::v1 as wire;
use prost::Message;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tonic::{Request, Response, Status};
#[derive(Clone, Default)]
struct Service {
    requests: Arc<Mutex<Vec<wire::PublishRequest>>>,
    registrations: Arc<Mutex<Vec<wire::DatasetRequest>>>,
    saves: Arc<Mutex<Vec<wire::SaveRequest>>>,
    schemas: Arc<Mutex<Vec<wire::SchemaRequest>>>,
    projects: Arc<Mutex<Vec<wire::NameRequest>>>,
    archives: Arc<Mutex<Vec<wire::ArchiveProjectRequest>>>,
    feature_reply: Option<wire::FeaturesReply>,
    publication_replies: Arc<Mutex<VecDeque<Result<wire::PublishReply, Status>>>>,
}
#[tonic::async_trait]
impl wire::geo_ledger_server::GeoLedger for Service {
    type FeaturesStreamStream = tokio_stream::Empty<Result<wire::DataChunk, Status>>;
    async fn save_stream(
        &self,
        _request: Request<tonic::Streaming<wire::DataChunk>>,
    ) -> Result<Response<wire::SaveReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn features_stream(
        &self,
        _request: Request<wire::FeaturesRequest>,
    ) -> Result<Response<Self::FeaturesStreamStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn info(
        &self,
        _request: Request<wire::Empty>,
    ) -> Result<Response<wire::InfoReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn create_project(
        &self,
        request: Request<wire::NameRequest>,
    ) -> Result<Response<wire::ProjectReply>, Status> {
        let request = request.into_inner();
        self.projects
            .lock()
            .map_err(|_| Status::internal("lock"))?
            .push(request.clone());
        Ok(Response::new(project_reply(request.name)))
    }
    async fn list_projects(
        &self,
        _request: Request<wire::PageRequest>,
    ) -> Result<Response<wire::ProjectsReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn get_project(
        &self,
        _request: Request<wire::ProjectRequest>,
    ) -> Result<Response<wire::ProjectReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn set_member(
        &self,
        _request: Request<wire::MemberRequest>,
    ) -> Result<Response<wire::OkReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn rename_project(
        &self,
        _request: Request<wire::RenameProjectRequest>,
    ) -> Result<Response<wire::ProjectReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn rename_dataset(
        &self,
        _request: Request<wire::RenameDatasetRequest>,
    ) -> Result<Response<wire::DatasetReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn delete_dataset(
        &self,
        _request: Request<wire::DeleteDatasetRequest>,
    ) -> Result<Response<wire::OkReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn create_dataset(
        &self,
        request: Request<wire::DatasetRequest>,
    ) -> Result<Response<wire::DatasetReply>, Status> {
        let request = request.into_inner();
        self.registrations
            .lock()
            .map_err(|_| Status::internal("lock"))?
            .push(request.clone());
        if request.name == "denied" {
            return Err(Status::permission_denied("registration denied"));
        }
        Ok(Response::new(wire::DatasetReply {
            dataset: "dataset".into(),
            name: request.name,
            geometry_type: "line".into(),
            coordinate_dimension: 3,
            crs: "EPSG:3857".into(),
            postgis_table: request.postgis_table,
        }))
    }
    async fn list_datasets(
        &self,
        _request: Request<wire::ProjectPageRequest>,
    ) -> Result<Response<wire::DatasetsReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn create_workspace(
        &self,
        _request: Request<wire::ProjectRequest>,
    ) -> Result<Response<wire::WorkspaceReply>, Status> {
        Ok(Response::new(wire::WorkspaceReply {
            workspace: "workspace".into(),
            version: 4,
            base_revision: 1,
            status: "open".into(),
        }))
    }
    async fn list_workspaces(
        &self,
        _request: Request<wire::ProjectPageRequest>,
    ) -> Result<Response<wire::WorkspacesReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn get_workspace(
        &self,
        _request: Request<wire::WorkspaceRequest>,
    ) -> Result<Response<wire::WorkspaceReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn save(
        &self,
        request: Request<wire::SaveRequest>,
    ) -> Result<Response<wire::SaveReply>, Status> {
        let request = request.into_inner();
        let version = request.expected_workspace_version + 1;
        self.saves
            .lock()
            .map_err(|_| Status::internal("lock"))?
            .push(request);
        Ok(Response::new(wire::SaveReply {
            version,
            changes: 1,
            warnings: vec![],
        }))
    }
    async fn schema(
        &self,
        request: Request<wire::SchemaRequest>,
    ) -> Result<Response<wire::SchemaReply>, Status> {
        let request = request.into_inner();
        let revision = request.revision.unwrap_or(1);
        let workspace_version = request.workspace.as_ref().map(|_| 4);
        self.schemas
            .lock()
            .map_err(|_| Status::internal("lock"))?
            .push(request);
        Ok(Response::new(wire::SchemaReply {
            revision,
            workspace_version,
            fields: vec![wire::SchemaField {
                name: "id".into(),
                r#type: "int8".into(),
                nullable: false,
                editable: false,
            }],
        }))
    }
    async fn discard(
        &self,
        _request: Request<wire::VersionRequest>,
    ) -> Result<Response<wire::DiscardReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn features(
        &self,
        _request: Request<wire::FeaturesRequest>,
    ) -> Result<Response<wire::FeaturesReply>, Status> {
        self.feature_reply
            .clone()
            .map(Response::new)
            .ok_or_else(|| Status::unimplemented("unused"))
    }
    async fn workspace_summary(
        &self,
        _request: Request<wire::WorkspaceRequest>,
    ) -> Result<Response<wire::ChangeSummaryReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn commit_summary(
        &self,
        _request: Request<wire::SummaryCommitRequest>,
    ) -> Result<Response<wire::ChangeSummaryReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn diff(
        &self,
        _request: Request<wire::DiffRequest>,
    ) -> Result<Response<wire::DiffReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn conflicts(
        &self,
        _request: Request<wire::DiffRequest>,
    ) -> Result<Response<wire::ConflictsReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn history(
        &self,
        _request: Request<wire::HistoryRequest>,
    ) -> Result<Response<wire::HistoryReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn commit(
        &self,
        _request: Request<wire::CommitRequest>,
    ) -> Result<Response<wire::CommitReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn audit(
        &self,
        _request: Request<wire::HistoryRequest>,
    ) -> Result<Response<wire::AuditReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn publish(
        &self,
        _request: Request<wire::PublishRequest>,
    ) -> Result<Response<wire::PublishReply>, Status> {
        let mut requests = self.requests.lock().map_err(|_| Status::internal("lock"))?;
        requests.push(_request.into_inner());
        if let Some(reply) = self
            .publication_replies
            .lock()
            .map_err(|_| Status::internal("lock"))?
            .pop_front()
        {
            return reply.map(Response::new);
        }
        if requests.len() == 1 {
            return Err(Status::unavailable("response lost after publication"));
        }
        match requests.len() {
            2 => return Err(Status::unauthenticated("expired token")),
            3 => return Err(Status::permission_denied("access denied")),
            4 => return Err(Status::not_found("hidden project")),
            6 => return Err(Status::unavailable("lost response")),
            7 => return Err(Status::aborted("conflict")),
            8 => return Err(Status::unauthenticated("expired token")),
            9 => return Err(Status::permission_denied("access denied")),
            10 => return Err(Status::not_found("hidden project")),
            _ => {}
        }
        Ok(Response::new(wire::PublishReply {
            workspace: "workspace".into(),
            revision: 2,
            version: 5,
            status: "published".into(),
            changes: 1,
        }))
    }
    async fn resolve(
        &self,
        _request: Request<wire::ResolveRequest>,
    ) -> Result<Response<wire::ResolveReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn rebase(
        &self,
        _request: Request<wire::ResolveRequest>,
    ) -> Result<Response<wire::RebaseReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn restore(
        &self,
        _request: Request<wire::RestoreRequest>,
    ) -> Result<Response<wire::WorkspaceReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_members(
        &self,
        _request: Request<wire::ProjectPageRequest>,
    ) -> Result<Response<wire::MembersReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn remove_member(
        &self,
        _request: Request<wire::MemberRefRequest>,
    ) -> Result<Response<wire::OkReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn archive_project(
        &self,
        request: Request<wire::ArchiveProjectRequest>,
    ) -> Result<Response<wire::ProjectReply>, Status> {
        let request = request.into_inner();
        self.archives
            .lock()
            .map_err(|_| Status::internal("lock"))?
            .push(request.clone());
        Ok(Response::new(wire::ProjectReply {
            project: request.project,
            state: if request.archived {
                "archived"
            } else {
                "active"
            }
            .into(),
            ..project_reply("project".into())
        }))
    }
    async fn delete_project(
        &self,
        _request: Request<wire::DeleteProjectRequest>,
    ) -> Result<Response<wire::OkReply>, Status> {
        Err(Status::unimplemented("unused"))
    }
}
#[tokio::test]
async fn unknown_outcome_preserves_original_publication()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let service = Service::default();
    let requests = service.requests.clone();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(wire::geo_ledger_server::GeoLedgerServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let client = Client::connect(
        format!("http://{address}"),
        "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await?;
    let mut draft = client.create_workspace("project").await?;
    let error = draft
        .publish("roads updated")
        .await
        .err()
        .ok_or("expected uncertain failure")?;
    assert!(error.uncertain);
    assert_eq!(error.code, "unavailable");
    assert!(draft.publish("changed").await.is_err());
    assert!(draft.delete("dataset", "one").await.is_err());
    assert_eq!(draft.info().version, 4);
    let original = draft.pending_publication().ok_or("lost intent")?.clone();
    for code in ["unauthenticated", "permission_denied", "not_found"] {
        let error = draft
            .publish("roads updated")
            .await
            .err()
            .ok_or("expected rejection")?;
        assert_eq!(error.code, code);
        assert!(!error.uncertain);
        assert_eq!(draft.pending_publication(), Some(&original));
        assert!(draft.publish("changed").await.is_err());
    }
    assert_eq!(draft.publish("roads updated").await?.revision, 2);
    assert_eq!(draft.pending_publication(), Some(&original));
    {
        let seen = requests.lock().map_err(|_| "lock")?;
        assert_eq!(seen.len(), 5);
        assert!(seen.iter().all(|request| request == &seen[0]));
    }
    assert_eq!(draft.info().status, "published");
    assert_eq!(draft.info().version, 5);
    let mut rejected = client.create_workspace("project").await?;
    assert!(rejected.publish("original").await.is_err());
    assert!(rejected.pending_publication().is_some());
    for code in [
        "conflict",
        "unauthenticated",
        "permission_denied",
        "not_found",
    ] {
        let error = rejected
            .publish("original")
            .await
            .err()
            .ok_or("expected rejection")?;
        assert_eq!(error.code, code);
        assert!(rejected.pending_publication().is_none());
        assert_eq!(rejected.info().version, 4);
    }
    server.abort();
    Ok(())
}

#[tokio::test]
async fn plaintext_to_remote_hosts_requires_explicit_opt_in() {
    let refused = geoledger_client::Client::connect(
        "http://geoledger.example:7882",
        "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await
    .err()
    .map(|e| e.code);
    assert_eq!(refused.as_deref(), Some("invalid_argument"));
    assert!(geoledger_client::is_loopback_host("127.0.0.1"));
    assert!(geoledger_client::is_loopback_host("[::1]"));
    assert!(geoledger_client::is_loopback_host("LOCALHOST"));
    assert!(!geoledger_client::is_loopback_host("10.0.0.1"));
}

#[tokio::test]
async fn register_table_preserves_create_dataset_wire_contract()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let service = Service::default();
    let registrations = service.registrations.clone();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(wire::geo_ledger_server::GeoLedgerServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let client = Client::connect(
        format!("http://{address}"),
        "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await?;
    let source = PostgisTable {
        schema: "business".into(),
        table: "roads".into(),
        id_column: "road_id".into(),
        geometry_column: "shape".into(),
    };
    let dataset = client.register_table("project", "roads", &source).await?;
    assert_eq!(dataset.id, "dataset");
    assert_eq!(dataset.geometry_type, "line");
    assert_eq!(dataset.coordinate_dimension, 3);
    assert_eq!(dataset.crs, "EPSG:3857");
    assert_eq!(dataset.postgis_table, Some(source));
    let denied = client
        .register_table(
            "project",
            "denied",
            dataset.postgis_table.as_ref().ok_or("source")?,
        )
        .await
        .err()
        .ok_or("expected permission error")?;
    assert_eq!(denied.code, "permission_denied");
    assert!(!denied.uncertain);
    {
        let seen = registrations.lock().map_err(|_| "lock")?;
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[0],
            wire::DatasetRequest {
                project: "project".into(),
                name: "roads".into(),
                postgis_table: dataset.postgis_table.map(|table| wire::PostgisTable {
                    schema: table.schema,
                    table: table.table,
                    id_column: table.id_column,
                    geometry_column: table.geometry_column,
                }),
                ..Default::default()
            }
        );
    }
    server.abort();
    Ok(())
}

#[tokio::test]
async fn schema_queries_and_save_intent_preserve_presence()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use geoledger_client::{AddField, SchemaEdit, SchemaQuery};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let service = Service::default();
    let saves = service.saves.clone();
    let schemas = service.schemas.clone();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(wire::geo_ledger_server::GeoLedgerServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let client = Client::connect(
        format!("http://{address}"),
        "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await?;
    let schema = client
        .schema(
            "p",
            "d",
            SchemaQuery {
                revision: Some(7),
                workspace: None,
            },
        )
        .await?;
    assert_eq!(schema.revision, 7);
    assert_eq!(schema.workspace_version, None);
    assert!(!schema.fields[0].editable);
    assert_eq!(schema.fields[0].kind, "int8");
    let mut draft = client.create_workspace("p").await?;
    assert_eq!(draft.schema("d").await?.workspace_version, Some(4));
    draft
        .save_changes(
            &[],
            Some(&[SchemaEdit {
                dataset: "d".into(),
                add: vec![AddField {
                    name: "note".into(),
                    kind: "string".into(),
                }],
                drop: vec![],
            }]),
        )
        .await?;
    draft.save_batch(&[]).await?;
    draft.save_changes(&[], Some(&[])).await?;
    let seen = saves.lock().map_err(|_| "lock")?;
    assert_eq!(seen[0].expected_workspace_version, 4);
    assert_eq!(
        seen[0].schema_edits.as_ref().ok_or("schema")?.edits[0].add[0].r#type,
        "string"
    );
    assert!(seen[1].schema_edits.is_none());
    assert!(
        seen[2]
            .schema_edits
            .as_ref()
            .ok_or("clear")?
            .edits
            .is_empty()
    );
    assert_eq!(seen[2].expected_workspace_version, 6);
    assert_eq!(schemas.lock().map_err(|_| "lock")?[0].revision, Some(7));
    server.abort();
    Ok(())
}

const API_KEY: &str = "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct TestServer {
    endpoint: String,
    task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}
impl TestServer {
    async fn start(
        service: Service,
        max_request_bytes: usize,
        max_response_bytes: usize,
    ) -> TestResult<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(
                    wire::geo_ledger_server::GeoLedgerServer::new(service)
                        .max_decoding_message_size(max_request_bytes)
                        .max_encoding_message_size(max_response_bytes),
                )
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
        });
        Ok(Self { endpoint, task })
    }
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn project_reply(name: String) -> wire::ProjectReply {
    wire::ProjectReply {
        project: "project".into(),
        name,
        head: 1,
        role: "owner".into(),
        state: "active".into(),
    }
}

fn publication_reply(version: i64) -> wire::PublishReply {
    wire::PublishReply {
        workspace: "workspace".into(),
        revision: 2,
        version,
        status: "published".into(),
        changes: 1,
    }
}

fn publication_attempt_rejected(code: tonic::Code) -> Status {
    let message = "publication source data was rejected";
    let detail = wire::ErrorDetail {
        code: "publication_attempt_rejected".into(),
        message: message.into(),
        request_id: "request".into(),
        conflicts: None,
    };
    let envelope = wire::RpcStatus {
        code: code as i32,
        message: message.into(),
        details: vec![wire::ErrorAny {
            type_url: "type.googleapis.com/geoledger.v1.ErrorDetail".into(),
            value: detail.encode_to_vec(),
        }],
    };
    Status::with_details(code, message, envelope.encode_to_vec().into())
}

#[tokio::test]
async fn archive_json_requires_explicit_boolean_before_rpc() -> TestResult {
    use serde_json::json;
    let service = Service::default();
    let archives = service.archives.clone();
    let server = TestServer::start(service, usize::MAX, usize::MAX).await?;
    let client = Client::connect(&server.endpoint, API_KEY).await?;
    for input in [
        json!({"project":"project"}),
        json!({"project":"project","archive":true}),
        json!({"project":"project","archived":true,"archive":true}),
        json!({"project":"project","archived":null}),
        json!({"project":"project","archived":"false"}),
        json!({"archived":false}),
    ] {
        let error = client
            .execute("archive_project", input)
            .await
            .err()
            .ok_or("invalid archive input was accepted")?;
        assert_eq!(error.code, "invalid_argument");
        assert!(!error.uncertain);
        assert!(archives.lock().map_err(|_| "lock")?.is_empty());
    }
    for archived in [true, false] {
        let result = client
            .execute(
                "archive_project",
                json!({"project":"project","archived":archived}),
            )
            .await?;
        assert_eq!(
            result["state"],
            if archived { "archived" } else { "active" }
        );
        let typed = client.archive_project("project", archived).await?;
        assert_eq!(typed.state, if archived { "archived" } else { "active" });
    }
    let seen = archives.lock().map_err(|_| "lock")?;
    assert_eq!(seen.len(), 4);
    assert_eq!(
        seen.iter()
            .map(|request| request.archived)
            .collect::<Vec<_>>(),
        [true, true, false, false]
    );
    assert!(seen.iter().all(|request| request.project == "project"));
    Ok(())
}

#[test]
fn message_limits_require_positive_byte_counts() -> TestResult {
    for (encoding, decoding) in [(0, 1), (1, 0), (0, 0)] {
        assert_eq!(
            MessageLimits::new(encoding, decoding)
                .err()
                .ok_or("zero limit was accepted")?
                .code,
            "invalid_argument"
        );
    }
    let limits = MessageLimits::new(1, usize::MAX)?;
    assert_eq!(limits.max_encoding_bytes(), 1);
    assert_eq!(limits.max_decoding_bytes(), usize::MAX);
    assert_eq!(MessageLimits::default().max_encoding_bytes(), usize::MAX);
    assert_eq!(MessageLimits::default().max_decoding_bytes(), usize::MAX);
    Ok(())
}

#[tokio::test]
async fn message_limits_enforce_exact_protobuf_boundaries_and_survive_clone() -> TestResult {
    let service = Service::default();
    let projects = service.projects.clone();
    let server = TestServer::start(service, usize::MAX, usize::MAX).await?;
    let name = "roads";
    let request_bytes = wire::NameRequest { name: name.into() }.encoded_len();
    let response_bytes = project_reply(name.into()).encoded_len();
    // Preserve source compatibility for existing callers using a complete literal.
    let options = ConnectOptions {
        timeout: Duration::from_secs(30),
        allow_insecure: false,
        ca_pem: None,
    };
    let outbound = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        options.clone(),
        MessageLimits::new(request_bytes - 1, response_bytes)?,
    )
    .await?;
    let smaller_name = "road";
    assert_eq!(
        wire::NameRequest {
            name: smaller_name.into()
        }
        .encoded_len(),
        request_bytes - 1
    );
    for client in [&outbound, &outbound.clone()] {
        let accepted_before = projects.lock().map_err(|_| "lock")?.len();
        let error = client
            .create_project(name)
            .await
            .err()
            .ok_or("expected encoding limit")?;
        // A client-side tonic encoding error is an HTTP body error, which the
        // HTTP/2 transport can wrap as Unavailable. Its downcast classification
        // is not stable; verify the byte boundary and delivery independently.
        assert!(
            matches!(error.code.as_str(), "resource_exhausted" | "unavailable"),
            "{error:?}"
        );
        assert!(error.uncertain, "{error:?}");
        assert_eq!(projects.lock().map_err(|_| "lock")?.len(), accepted_before);
        // The same client remains usable at its exact encoding limit. Together
        // with the service count, this rules out a broken connection fixture
        // being mistaken for successful enforcement of the outbound budget.
        assert_eq!(
            client.create_project(smaller_name).await?.name,
            smaller_name
        );
        assert_eq!(
            projects.lock().map_err(|_| "lock")?.len(),
            accepted_before + 1
        );
    }
    {
        let seen = projects.lock().map_err(|_| "lock")?;
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|request| request.name == smaller_name));
    }

    let inbound = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        options.clone(),
        MessageLimits::new(request_bytes, response_bytes - 1)?,
    )
    .await?;
    for client in [&inbound, &inbound.clone()] {
        let error = client
            .create_project(name)
            .await
            .err()
            .ok_or("expected decoding limit")?;
        assert_eq!(error.code, "resource_exhausted");
        assert!(error.uncertain);
    }
    assert_eq!(projects.lock().map_err(|_| "lock")?.len(), 4);

    let exact = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        options,
        MessageLimits::new(request_bytes, response_bytes)?,
    )
    .await?;
    assert_eq!(exact.create_project(name).await?.name, name);
    assert_eq!(exact.clone().create_project(name).await?.name, name);
    assert_eq!(projects.lock().map_err(|_| "lock")?.len(), 6);
    Ok(())
}

#[tokio::test]
async fn raised_limits_and_legacy_connections_accept_large_feature_pages() -> TestResult {
    use serde_json::json;
    let features: Vec<_> = (0..12)
        .map(|id| {
            json!({
                "type":"Feature", "id":id.to_string(),
                "geometry":{"type":"Point","coordinates":[1.0,2.0]},
                "properties":{"note":"x".repeat(384 * 1024),"exact":18446744073709551615u64}
            })
        })
        .collect();
    let reply = wire::FeaturesReply {
        features: features
            .iter()
            .map(|feature| wire::Feature {
                geojson: feature.to_string(),
            })
            .collect(),
        revision: 1,
        workspace_version: None,
        next_after: None,
        crs: "EPSG:4326".into(),
    };
    let response_bytes = reply.encoded_len();
    assert!(response_bytes > 4 * 1024 * 1024);
    let service = Service {
        feature_reply: Some(reply),
        ..Service::default()
    };
    let server = TestServer::start(service, usize::MAX, usize::MAX).await?;
    let low = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        ConnectOptions::default(),
        MessageLimits::new(1024, response_bytes - 1)?,
    )
    .await?;
    let error = low
        .features("project", "dataset", FeatureQuery::default())
        .await
        .err()
        .ok_or("large page bypassed limit")?;
    assert_eq!(error.code, "resource_exhausted");
    let raised = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        ConnectOptions::default(),
        MessageLimits::new(1024, response_bytes)?,
    )
    .await?;
    let legacy = Client::connect(&server.endpoint, API_KEY).await?;
    for client in [&raised, &legacy] {
        let page = client
            .features("project", "dataset", FeatureQuery::default())
            .await?;
        assert_eq!(page.features, features);
        assert_eq!(page.revision, 1);
        assert!(page.next_after.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn client_limits_do_not_override_server_message_limits() -> TestResult {
    let service = Service::default();
    let projects = service.projects.clone();
    let name = "roads";
    let request_bytes = wire::NameRequest { name: name.into() }.encoded_len();
    let server = TestServer::start(service, request_bytes - 1, usize::MAX).await?;
    let client = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        ConnectOptions::default(),
        MessageLimits::new(1024, 1024)?,
    )
    .await?;
    let error = client
        .create_project(name)
        .await
        .err()
        .ok_or("server limit was bypassed")?;
    assert_eq!(error.code, "resource_exhausted");
    assert!(error.uncertain);
    assert!(projects.lock().map_err(|_| "lock")?.is_empty());
    Ok(())
}

#[tokio::test]
async fn publication_reply_over_decoding_limit_preserves_recovery_intent() -> TestResult {
    let reply = publication_reply(5);
    let response_bytes = reply.encoded_len();
    let service = Service::default();
    let requests = service.requests.clone();
    service
        .publication_replies
        .lock()
        .map_err(|_| "lock")?
        .extend([Ok(reply.clone()), Ok(reply.clone()), Ok(reply)]);
    let server = TestServer::start(service, usize::MAX, usize::MAX).await?;
    let client = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        ConnectOptions::default(),
        MessageLimits::new(1024, response_bytes - 1)?,
    )
    .await?;
    let mut draft = client.create_workspace("project").await?;
    let error = draft
        .publish("roads updated")
        .await
        .err()
        .ok_or("expected oversized reply")?;
    assert_eq!(error.code, "resource_exhausted");
    assert!(error.uncertain);
    let original = draft
        .pending_publication()
        .ok_or("lost publication")?
        .clone();
    assert!(draft.save_batch(&[]).await.is_err());
    assert!(draft.publish("different message").await.is_err());
    assert_eq!(draft.info().version, 4);
    let clone_error = client
        .clone()
        .publish(&original)
        .await
        .err()
        .ok_or("clone lost limit")?;
    assert_eq!(clone_error.code, "resource_exhausted");
    assert!(clone_error.uncertain);
    assert_eq!(draft.pending_publication(), Some(&original));

    let recovery = Client::connect_with_message_limits(
        &server.endpoint,
        API_KEY,
        ConnectOptions::default(),
        MessageLimits::new(1024, response_bytes)?,
    )
    .await?;
    let result = recovery.publish(&original).await?;
    assert_eq!(result.revision, 2);
    assert_eq!(result.version, 5);
    let seen = requests.lock().map_err(|_| "lock")?;
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|request| request == &seen[0]));
    assert_eq!(seen[0].request_id, original.request_id);
    Ok(())
}

#[tokio::test]
async fn first_publication_attempt_rejection_allows_workspace_correction() -> TestResult {
    let service = Service::default();
    let requests = service.requests.clone();
    service
        .publication_replies
        .lock()
        .map_err(|_| "lock")?
        .extend([
            Err(publication_attempt_rejected(tonic::Code::Unavailable)),
            Ok(publication_reply(6)),
        ]);
    let server = TestServer::start(service, usize::MAX, usize::MAX).await?;
    let client = Client::connect(&server.endpoint, API_KEY).await?;
    let mut draft = client.create_workspace("project").await?;
    let error = draft
        .publish("rejected input")
        .await
        .err()
        .ok_or("expected rejection")?;
    assert_eq!(error.code, "publication_attempt_rejected");
    assert!(!error.uncertain);
    assert!(draft.pending_publication().is_none());
    draft.delete("dataset", "rejected feature").await?;
    assert_eq!(draft.info().version, 5);
    assert_eq!(draft.publish("corrected input").await?.version, 6);
    let seen = requests.lock().map_err(|_| "lock")?;
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0].request_id, seen[1].request_id);
    assert_eq!(seen[0].expected_workspace_version, 4);
    assert_eq!(seen[1].expected_workspace_version, 5);
    assert_eq!(seen[1].message, "corrected input");
    Ok(())
}

#[tokio::test]
async fn retry_rejections_preserve_prior_unknown_publication() -> TestResult {
    let service = Service::default();
    let requests = service.requests.clone();
    service
        .publication_replies
        .lock()
        .map_err(|_| "lock")?
        .extend([
            Err(Status::unavailable("response lost")),
            Err(publication_attempt_rejected(tonic::Code::Unavailable)),
            Err(publication_attempt_rejected(tonic::Code::Unavailable)),
            // The stage code cannot release a retry even if a peer changes status.
            Err(publication_attempt_rejected(tonic::Code::InvalidArgument)),
            Err(Status::invalid_argument("input rejected")),
            Ok(publication_reply(5)),
        ]);
    let server = TestServer::start(service, usize::MAX, usize::MAX).await?;
    let client = Client::connect(&server.endpoint, API_KEY).await?;
    let mut draft = client.create_workspace("project").await?;
    assert!(draft.publish("original").await.is_err());
    let original = draft
        .pending_publication()
        .ok_or("lost publication")?
        .clone();
    // The lower-level API has no first-attempt context and stays conservative.
    let raw_error = client
        .publish(&original)
        .await
        .err()
        .ok_or("expected rejection")?;
    assert_eq!(raw_error.code, "publication_attempt_rejected");
    assert!(raw_error.uncertain);
    for (code, uncertain) in [
        ("publication_attempt_rejected", true),
        ("publication_attempt_rejected", false),
        ("invalid_argument", false),
    ] {
        let error = draft
            .publish("original")
            .await
            .err()
            .ok_or("expected retry rejection")?;
        assert_eq!(error.code, code);
        assert_eq!(error.uncertain, uncertain);
        assert_eq!(draft.pending_publication(), Some(&original));
        assert!(draft.save_batch(&[]).await.is_err());
        assert!(draft.publish("changed").await.is_err());
    }
    assert_eq!(draft.publish("original").await?.revision, 2);
    assert_eq!(draft.pending_publication(), Some(&original));
    let seen = requests.lock().map_err(|_| "lock")?;
    assert_eq!(seen.len(), 6);
    assert!(seen.iter().all(|request| request == &seen[0]));
    Ok(())
}

#[tokio::test]
async fn first_invalid_argument_still_releases_publication() -> TestResult {
    let service = Service::default();
    service
        .publication_replies
        .lock()
        .map_err(|_| "lock")?
        .push_back(Err(Status::invalid_argument("input rejected")));
    let server = TestServer::start(service, usize::MAX, usize::MAX).await?;
    let client = Client::connect(&server.endpoint, API_KEY).await?;
    let mut draft = client.create_workspace("project").await?;
    let error = draft
        .publish("original")
        .await
        .err()
        .ok_or("expected rejection")?;
    assert_eq!(error.code, "invalid_argument");
    assert!(!error.uncertain);
    assert!(draft.pending_publication().is_none());
    draft.save_batch(&[]).await?;
    Ok(())
}
