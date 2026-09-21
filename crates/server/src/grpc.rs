use crate::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, Service, proto::*, public_message};
use spatial_version::Command;
use spatial_version_core::Error;
use tonic::{Request, Response, Status};

pub struct Rpc {
    service: Service,
}
pub fn server(service: Service) -> spatial_version_server::SpatialVersionServer<Rpc> {
    spatial_version_server::SpatialVersionServer::new(Rpc { service })
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES)
}
impl Rpc {
    fn check<T>(&self, r: &Request<T>) -> Result<(), Status> {
        if self.service.authorized(
            r.metadata()
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
        ) {
            Ok(())
        } else {
            Err(Status::unauthenticated("Bearer token required"))
        }
    }
    async fn run(&self, command: Command) -> Result<Response<JsonReply>, Status> {
        let value = self.service.execute(command).await.map_err(map_error)?;
        Ok(Response::new(JsonReply {
            json: value.to_string(),
        }))
    }
}
fn map_error(e: Error) -> Status {
    let message = format!("{}: {}", e.code(), public_message(&e));
    match e {
        Error::Invalid(_) | Error::Json(_) => Status::invalid_argument(message),
        Error::NotFound(_) => Status::not_found(message),
        Error::Dirty | Error::Conflict(_) | Error::Recovery(_) => {
            Status::failed_precondition(message)
        }
        Error::Busy => Status::resource_exhausted(message),
        Error::Unsupported(_) => Status::unimplemented(message),
        _ => Status::internal(message),
    }
}
fn option(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}
fn author(value: String) -> String {
    if value.is_empty() {
        "unknown".into()
    } else {
        value
    }
}
fn reference(value: String) -> String {
    if value.is_empty() {
        "HEAD".into()
    } else {
        value
    }
}
fn limit(value: u32) -> usize {
    if value == 0 { 100 } else { value as usize }
}

#[tonic::async_trait]
impl spatial_version_server::SpatialVersion for Rpc {
    async fn execute(&self, r: Request<ExecuteRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        let command = serde_json::from_str(&p.command_json)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        self.run(command).await
    }
    async fn status(&self, r: Request<StatusRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        self.run(Command::Status {
            limit: limit(r.into_inner().limit),
        })
        .await
    }
    async fn import(&self, r: Request<ImportRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Import {
            dataset: p.dataset,
            schema: if p.schema.is_empty() {
                "public".into()
            } else {
                p.schema
            },
            table: p.table,
            author: author(p.author),
            message: option(p.message),
        })
        .await
    }
    async fn commit(&self, r: Request<CommitRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Commit {
            message: p.message,
            author: author(p.author),
        })
        .await
    }
    async fn log(&self, r: Request<LogRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Log {
            reference: reference(p.reference),
            limit: limit(p.limit),
        })
        .await
    }
    async fn diff(&self, r: Request<DiffRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Diff {
            from: option(p.from),
            to: option(p.to),
            limit: limit(p.limit),
        })
        .await
    }
    async fn branch(&self, r: Request<BranchRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Branch {
            name: p.name,
            from: reference(p.from),
        })
        .await
    }
    async fn switch(&self, r: Request<ReferenceRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        self.run(Command::Switch {
            branch: r.into_inner().reference,
        })
        .await
    }
    async fn merge(&self, r: Request<MergeRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Merge {
            source: p.reference,
            author: author(p.author),
            message: option(p.message),
        })
        .await
    }
    async fn revert(&self, r: Request<MergeRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Revert {
            target: p.reference,
            author: author(p.author),
            message: option(p.message),
        })
        .await
    }
    async fn reset(&self, r: Request<ResetRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        let p = r.into_inner();
        self.run(Command::Reset {
            target: p.target,
            hard: p.hard,
        })
        .await
    }
    async fn restore(&self, r: Request<RestoreRequest>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        self.run(Command::Restore {
            discard: r.into_inner().discard,
        })
        .await
    }
    async fn r#continue(&self, r: Request<Empty>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        self.run(Command::MergeContinue).await
    }
    async fn abort(&self, r: Request<Empty>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        self.run(Command::MergeAbort).await
    }
    async fn recover(&self, r: Request<Empty>) -> Result<Response<JsonReply>, Status> {
        self.check(&r)?;
        self.run(Command::Recover).await
    }
}
