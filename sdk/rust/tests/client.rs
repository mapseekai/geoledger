use geoledger_client::{Client, PostgisTable};
use geoledger_rpc::v1 as wire;
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};
#[derive(Clone, Default)]
struct Service {
    requests: Arc<Mutex<Vec<wire::PublishRequest>>>,
    registrations: Arc<Mutex<Vec<wire::DatasetRequest>>>,
    saves: Arc<Mutex<Vec<wire::SaveRequest>>>,
    schemas: Arc<Mutex<Vec<wire::SchemaRequest>>>,
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
        _request: Request<wire::NameRequest>,
    ) -> Result<Response<wire::ProjectReply>, Status> {
        Err(Status::unimplemented("unused"))
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
        Err(Status::unimplemented("unused"))
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
        _request: Request<wire::ArchiveProjectRequest>,
    ) -> Result<Response<wire::ProjectReply>, Status> {
        Err(Status::unimplemented("unused"))
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
