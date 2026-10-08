use crate::*;
use geoledger_rpc::v1 as pb;
use prost::Message;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use tonic::{Code, Request, Response, Status};
pub fn grpc(service: Service) -> pb::geo_ledger_server::GeoLedgerServer<Service> {
    pb::geo_ledger_server::GeoLedgerServer::new(service)
        .max_decoding_message_size(MAX_BYTES)
        .max_encoding_message_size(MAX_BYTES)
}
fn input(mut value: Value) -> Result<Value> {
    if let Some(o) = value.as_object_mut() {
        // Optional protobuf fields retain presence; omitted limits use application defaults.
        for key in ["limit", "workspace", "revision", "feature_id"] {
            if o.get(key).is_some_and(Value::is_null) {
                o.remove(key);
            }
        }
        if o.get("bbox")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            o.remove("bbox");
        }
        for key in ["edits", "resolutions"] {
            if let Some(items) = o.get_mut(key).and_then(Value::as_array_mut) {
                for item in items {
                    if !item["feature"].is_null() {
                        let s = item["feature"]["geojson"].as_str().ok_or_else(bad)?;
                        item["feature"] = geoledger_engine::parse_json(s.as_bytes())?;
                    }
                }
            }
        }
    }
    Ok(value)
}
fn output(value: Value) -> Value {
    match value {
        Value::Object(mut o) if o.get("type").and_then(Value::as_str) == Some("Feature") => {
            o.remove("revision");
            o.remove("workspace_version");
            json!({"geojson":Value::Object(o).to_string()})
        }
        Value::Object(mut o) => {
            if let Some(detail) = o.remove("detail") {
                o.insert("detail_json".into(), Value::String(detail.to_string()));
            }
            Value::Object(o.into_iter().map(|(k, v)| (k, output(v))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(output).collect()),
        v => v,
    }
}
fn status(error: Error) -> Status {
    let code = match error.status {
        400 | 422 => Code::InvalidArgument,
        401 => Code::Unauthenticated,
        403 => Code::PermissionDenied,
        404 => Code::NotFound,
        409 => Code::Aborted,
        413 | 429 => Code::ResourceExhausted,
        408 | 504 => Code::DeadlineExceeded,
        503 => Code::Unavailable,
        _ => Code::Internal,
    };
    let e = &error.body["error"];
    let mut detail = pb::ErrorDetail {
        code: e["code"].as_str().unwrap_or("internal").into(),
        message: e["message"].as_str().unwrap_or("operation failed").into(),
        request_id: e["request_id"].as_str().unwrap_or("").into(),
        conflicts: if error.body.get("conflicts").is_some() {
            serde_json::from_value(output(error.body.clone())).ok()
        } else {
            None
        },
    };
    // Trailers are HTTP/2 metadata, not a normal gRPC response body. Keep the
    // encoded detail below 4 KiB so base64 expansion and the other headers fit
    // clients' common 8 KiB metadata limit. Large conflicts remain available
    // through the paginated Conflicts RPC; the summary preserves their context.
    if detail.encoded_len() > 4096
        && let Some(conflicts) = &mut detail.conflicts
    {
        conflicts.conflicts.clear();
        conflicts.next_after = None;
        conflicts.truncated = true;
    }
    let envelope = pb::RpcStatus {
        code: code as i32,
        message: detail.message.clone(),
        details: vec![pb::ErrorAny {
            type_url: "type.googleapis.com/geoledger.v1.ErrorDetail".into(),
            value: detail.encode_to_vec(),
        }],
    };
    let mut s = Status::with_details(
        code,
        detail.message.clone(),
        envelope.encode_to_vec().into(),
    );
    if let Ok(id) = detail.request_id.parse() {
        s.metadata_mut().insert("x-request-id", id);
    }
    s
}
impl Service {
    async fn call<I: Serialize, O: DeserializeOwned>(
        &self,
        request: Request<I>,
        op: &str,
    ) -> std::result::Result<Response<O>, Status> {
        let headers = request.metadata().clone().into_headers();
        let subject = self.authenticate(&headers).map_err(status)?;
        let timeout = request
            .metadata()
            .get("grpc-timeout")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_timeout);
        let permit = self.permit().map_err(status)?;
        let value = serde_json::to_value(request.into_inner())
            .map_err(|_| Status::invalid_argument("invalid request"))?;
        let value = input(value).map_err(status)?;
        let mut result = self
            .execute(subject, op, value, timeout, permit)
            .await
            .map_err(status)?;
        result = match op {
            "list_projects" => json!({"projects":result}),
            "list_datasets" => json!({"datasets":result}),
            "list_workspaces" => json!({"workspaces":result}),
            "history" => json!({"commits":result}),
            "features" if result["type"] == "Feature" => {
                json!({"revision":result["revision"],"workspace_version":result["workspace_version"],"features":[result]})
            }
            _ => result,
        };
        let response = serde_json::from_value(output(result))
            .map_err(|_| Status::internal("response contract violation"))?;
        let mut response = Response::new(response);
        response.metadata_mut().insert(
            "x-request-id",
            uuid::Uuid::new_v4()
                .to_string()
                .parse()
                .map_err(|_| Status::internal("request id"))?,
        );
        Ok(response)
    }
}
fn parse_timeout(s: &str) -> Option<Duration> {
    let (n, unit) = s.split_at_checked(s.len().checked_sub(1)?)?;
    if n.len() > 8 {
        return None;
    }
    let n: u64 = n.parse().ok()?;
    match unit {
        "H" => Some(Duration::from_secs(n.checked_mul(3600)?)),
        "M" => Some(Duration::from_secs(n.checked_mul(60)?)),
        "S" => Some(Duration::from_secs(n)),
        "m" => Some(Duration::from_millis(n)),
        "u" => Some(Duration::from_micros(n)),
        "n" => Some(Duration::from_nanos(n)),
        _ => None,
    }
}
#[tonic::async_trait]
impl pb::geo_ledger_server::GeoLedger for Service {
    async fn info(
        &self,
        request: Request<pb::Empty>,
    ) -> std::result::Result<Response<pb::InfoReply>, Status> {
        self.authenticate(&request.metadata().clone().into_headers())
            .map_err(status)?;
        Ok(Response::new(pb::InfoReply {
            version: env!("CARGO_PKG_VERSION").into(),
            backend: self.app.backend().into(),
            format_version: geoledger_engine::FORMAT_VERSION as u32,
            max_request_bytes: MAX_BYTES as u32,
            max_feature_bytes: 16384,
        }))
    }
    async fn create_project(
        &self,
        request: tonic::Request<pb::NameRequest>,
    ) -> std::result::Result<tonic::Response<pb::ProjectReply>, tonic::Status> {
        self.call(request, "create_project").await
    }
    async fn list_projects(
        &self,
        request: tonic::Request<pb::PageRequest>,
    ) -> std::result::Result<tonic::Response<pb::ProjectsReply>, tonic::Status> {
        self.call(request, "list_projects").await
    }
    async fn get_project(
        &self,
        request: tonic::Request<pb::ProjectRequest>,
    ) -> std::result::Result<tonic::Response<pb::ProjectReply>, tonic::Status> {
        self.call(request, "get_project").await
    }
    async fn set_member(
        &self,
        request: tonic::Request<pb::MemberRequest>,
    ) -> std::result::Result<tonic::Response<pb::OkReply>, tonic::Status> {
        self.call(request, "set_member").await
    }
    async fn create_dataset(
        &self,
        request: tonic::Request<pb::DatasetRequest>,
    ) -> std::result::Result<tonic::Response<pb::DatasetReply>, tonic::Status> {
        self.call(request, "create_dataset").await
    }
    async fn list_datasets(
        &self,
        request: tonic::Request<pb::ProjectPageRequest>,
    ) -> std::result::Result<tonic::Response<pb::DatasetsReply>, tonic::Status> {
        self.call(request, "list_datasets").await
    }
    async fn create_workspace(
        &self,
        request: tonic::Request<pb::ProjectRequest>,
    ) -> std::result::Result<tonic::Response<pb::WorkspaceReply>, tonic::Status> {
        self.call(request, "create_workspace").await
    }
    async fn list_workspaces(
        &self,
        request: tonic::Request<pb::ProjectPageRequest>,
    ) -> std::result::Result<tonic::Response<pb::WorkspacesReply>, tonic::Status> {
        self.call(request, "list_workspaces").await
    }
    async fn get_workspace(
        &self,
        request: tonic::Request<pb::WorkspaceRequest>,
    ) -> std::result::Result<tonic::Response<pb::WorkspaceReply>, tonic::Status> {
        self.call(request, "get_workspace").await
    }
    async fn save(
        &self,
        request: tonic::Request<pb::SaveRequest>,
    ) -> std::result::Result<tonic::Response<pb::SaveReply>, tonic::Status> {
        self.call(request, "save").await
    }
    async fn discard(
        &self,
        request: tonic::Request<pb::VersionRequest>,
    ) -> std::result::Result<tonic::Response<pb::DiscardReply>, tonic::Status> {
        self.call(request, "discard").await
    }
    async fn features(
        &self,
        request: tonic::Request<pb::FeaturesRequest>,
    ) -> std::result::Result<tonic::Response<pb::FeaturesReply>, tonic::Status> {
        self.call(request, "features").await
    }
    async fn diff(
        &self,
        request: tonic::Request<pb::DiffRequest>,
    ) -> std::result::Result<tonic::Response<pb::DiffReply>, tonic::Status> {
        self.call(request, "diff").await
    }
    async fn conflicts(
        &self,
        request: tonic::Request<pb::DiffRequest>,
    ) -> std::result::Result<tonic::Response<pb::ConflictsReply>, tonic::Status> {
        self.call(request, "conflicts").await
    }
    async fn history(
        &self,
        request: tonic::Request<pb::HistoryRequest>,
    ) -> std::result::Result<tonic::Response<pb::HistoryReply>, tonic::Status> {
        self.call(request, "history").await
    }
    async fn commit(
        &self,
        request: tonic::Request<pb::CommitRequest>,
    ) -> std::result::Result<tonic::Response<pb::CommitReply>, tonic::Status> {
        self.call(request, "commit").await
    }
    async fn audit(
        &self,
        request: tonic::Request<pb::HistoryRequest>,
    ) -> std::result::Result<tonic::Response<pb::AuditReply>, tonic::Status> {
        self.call(request, "audit").await
    }
    async fn publish(
        &self,
        request: tonic::Request<pb::PublishRequest>,
    ) -> std::result::Result<tonic::Response<pb::PublishReply>, tonic::Status> {
        self.call(request, "publish").await
    }
    async fn resolve(
        &self,
        request: tonic::Request<pb::ResolveRequest>,
    ) -> std::result::Result<tonic::Response<pb::ResolveReply>, tonic::Status> {
        self.call(request, "resolve").await
    }
    async fn rebase(
        &self,
        request: tonic::Request<pb::ResolveRequest>,
    ) -> std::result::Result<tonic::Response<pb::RebaseReply>, tonic::Status> {
        self.call(request, "rebase").await
    }
    async fn restore(
        &self,
        request: tonic::Request<pb::RestoreRequest>,
    ) -> std::result::Result<tonic::Response<pb::WorkspaceReply>, tonic::Status> {
        self.call(request, "restore").await
    }
}
