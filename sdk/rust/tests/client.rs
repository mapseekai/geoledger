use geoledger_client::Client;
use geoledger_rpc::v1 as wire;
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};
#[derive(Clone, Default)]
struct Service {
    requests: Arc<Mutex<Vec<wire::PublishRequest>>>,
}
#[tonic::async_trait]
impl wire::geo_ledger_server::GeoLedger for Service {
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
    async fn create_dataset(
        &self,
        _request: Request<wire::DatasetRequest>,
    ) -> Result<Response<wire::DatasetReply>, Status> {
        Err(Status::unimplemented("unused"))
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
        _request: Request<wire::SaveRequest>,
    ) -> Result<Response<wire::SaveReply>, Status> {
        Err(Status::unimplemented("unused"))
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
    let client = Client::connect(format!("http://{address}"), "test-only").await?;
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
    assert_eq!(draft.publish("roads updated").await?.revision, 2);
    assert_eq!(draft.pending_publication(), Some(&original));
    let seen = requests.lock().map_err(|_| "lock")?;
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0], seen[1]);
    assert_eq!(draft.info().status, "published");
    assert_eq!(draft.info().version, 5);
    server.abort();
    Ok(())
}
