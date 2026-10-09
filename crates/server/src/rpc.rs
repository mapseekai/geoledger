use crate::*;
use geoledger_rpc::v1 as pb;
use prost::Message;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use tonic::{Code, Request, Response, Status};
pub fn grpc(service: Service) -> pb::geo_ledger_server::GeoLedgerServer<Service> {
    pb::geo_ledger_server::GeoLedgerServer::new(service)
        .max_decoding_message_size(usize::MAX)
        .max_encoding_message_size(usize::MAX)
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
/// Peer IP for both plain TCP and TLS connections.
fn peer<T>(request: &Request<T>) -> Option<std::net::IpAddr> {
    request
        .remote_addr()
        .or_else(|| request.extensions().get::<tls::PeerAddr>().map(|p| p.0))
        .map(|a| a.ip())
}
fn incoming_id<T>(request: &Request<T>) -> String {
    crate::limits::request_id(
        request
            .metadata()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
    )
}
fn with_id<T>(mut response: Response<T>, id: &str) -> Response<T> {
    if let Ok(id) = id.parse() {
        response.metadata_mut().insert("x-request-id", id);
    }
    response
}
impl Service {
    async fn call<I: Serialize, O: DeserializeOwned>(
        &self,
        request: Request<I>,
        op: &str,
    ) -> std::result::Result<Response<O>, Status> {
        let mut call = Call::new("grpc", op, incoming_id(&request), peer(&request));
        let result = self.handle(&mut call, request).await;
        let response = self.finish(&call, result).map_err(status)?;
        Ok(with_id(Response::new(response), &call.request_id))
    }
    async fn handle<I: Serialize, O: DeserializeOwned>(
        &self,
        call: &mut Call,
        request: Request<I>,
    ) -> Result<O> {
        let headers = request.metadata().clone().into_headers();
        let timeout = request
            .metadata()
            .get("grpc-timeout")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_timeout);
        let permit = self.admit(call, &headers)?;
        let value = serde_json::to_value(request.into_inner()).map_err(|_| bad())?;
        let value = input(value)?;
        let mut result = self.execute(call, value, timeout, permit).await?;
        result = match call.operation.as_str() {
            "list_projects" => json!({"projects":result}),
            "list_datasets" => json!({"datasets":result}),
            "list_workspaces" => json!({"workspaces":result}),
            "history" => json!({"commits":result}),
            "list_members" => json!({"members":result}),
            "features" if result["type"] == "Feature" => {
                json!({"revision":result["revision"],"workspace_version":result["workspace_version"],"features":[result]})
            }
            _ => result,
        };
        serde_json::from_value(output(result))
            .map_err(|_| Error::new(500, "response contract violation"))
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
    type FeaturesStreamStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = std::result::Result<pb::DataChunk, Status>> + Send>,
    >;

    async fn save_stream(
        &self,
        request: Request<tonic::Streaming<pb::DataChunk>>,
    ) -> std::result::Result<Response<pb::SaveReply>, Status> {
        let mut call = Call::new("grpc", "save", incoming_id(&request), peer(&request));
        let result = async {
            // Authenticate and reserve capacity before accepting the upload.
            let permit = self.admit(&mut call, &request.metadata().clone().into_headers())?;
            let timeout = request
                .metadata()
                .get("grpc-timeout")
                .and_then(|v| v.to_str().ok())
                .and_then(parse_timeout)
                .unwrap_or(self.limits.request_timeout)
                .min(self.limits.request_timeout);
            let started = std::time::Instant::now();
            let mut stream = request.into_inner();
            let bytes = tokio::time::timeout(timeout, async {
                let mut bytes = Vec::new();
                let mut total = None;
                while let Some(chunk) = stream
                    .message()
                    .await
                    .map_err(|_| Error::new(400, "upload interrupted"))?
                {
                    if total.is_none() {
                        total = Some(chunk.total_bytes);
                    } else if chunk.total_bytes != 0 {
                        return Err(bad());
                    }
                    bytes.extend_from_slice(&chunk.data);
                    if Some(bytes.len() as u64) > total {
                        return Err(bad());
                    }
                }
                if total != Some(bytes.len() as u64) {
                    return Err(Error::new(400, "incomplete upload"));
                }
                Ok::<_, Error>(bytes)
            })
            .await
            .map_err(|_| Error::new(408, "upload timeout"))??;
            let request = pb::SaveRequest::decode(bytes.as_slice()).map_err(|_| bad())?;
            let value = input(serde_json::to_value(request).map_err(|_| bad())?)?;
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(Error::new(408, "upload timeout"));
            }
            let result = self.execute(&call, value, Some(remaining), permit).await?;
            serde_json::from_value(output(result))
                .map_err(|_| Error::new(500, "response contract violation"))
        }
        .await;
        let response = self.finish(&call, result).map_err(status)?;
        Ok(with_id(Response::new(response), &call.request_id))
    }

    async fn features_stream(
        &self,
        request: Request<pb::FeaturesRequest>,
    ) -> std::result::Result<Response<Self::FeaturesStreamStream>, Status> {
        // Run one Application query so all chunks describe the same snapshot.
        let response: Response<pb::FeaturesReply> = self.call(request, "features").await?;
        let (metadata, value, extensions) = response.into_parts();
        let bytes = value.encode_to_vec();
        let mut offset = 0;
        let mut emitted = false;
        let chunks = std::iter::from_fn(move || {
            if emitted && offset == bytes.len() {
                return None;
            }
            let end = (offset + 64 * 1024).min(bytes.len());
            let chunk = pb::DataChunk {
                data: bytes[offset..end].to_vec(),
                total_bytes: if emitted { 0 } else { bytes.len() as u64 },
            };
            emitted = true;
            offset = end;
            Some(Ok(chunk))
        });
        let stream: Self::FeaturesStreamStream = Box::pin(tokio_stream::iter(chunks));
        Ok(Response::from_parts(metadata, stream, extensions))
    }

    async fn info(
        &self,
        request: Request<pb::Empty>,
    ) -> std::result::Result<Response<pb::InfoReply>, Status> {
        let mut call = Call::new("grpc", "info", incoming_id(&request), peer(&request));
        let admitted = self
            .admit(&mut call, &request.metadata().clone().into_headers())
            .map(drop);
        self.finish(&call, admitted).map_err(status)?;
        Ok(with_id(
            Response::new(pb::InfoReply {
                version: env!("CARGO_PKG_VERSION").into(),
                backend: self.app.backend().into(),
                format_version: geoledger_engine::FORMAT_VERSION as u32,
                max_request_bytes: 0, // No application-level message byte cap.
                max_feature_bytes: 0, // No independent per-feature byte limit.
            }),
            &call.request_id,
        ))
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
    async fn rename_project(
        &self,
        request: tonic::Request<pb::RenameProjectRequest>,
    ) -> std::result::Result<tonic::Response<pb::ProjectReply>, tonic::Status> {
        self.call(request, "rename_project").await
    }
    async fn rename_dataset(
        &self,
        request: tonic::Request<pb::RenameDatasetRequest>,
    ) -> std::result::Result<tonic::Response<pb::DatasetReply>, tonic::Status> {
        self.call(request, "rename_dataset").await
    }
    async fn delete_dataset(
        &self,
        request: tonic::Request<pb::DeleteDatasetRequest>,
    ) -> std::result::Result<tonic::Response<pb::OkReply>, tonic::Status> {
        self.call(request, "delete_dataset").await
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
    async fn list_members(
        &self,
        request: tonic::Request<pb::ProjectPageRequest>,
    ) -> std::result::Result<tonic::Response<pb::MembersReply>, tonic::Status> {
        self.call(request, "list_members").await
    }
    async fn remove_member(
        &self,
        request: tonic::Request<pb::MemberRefRequest>,
    ) -> std::result::Result<tonic::Response<pb::OkReply>, tonic::Status> {
        self.call(request, "remove_member").await
    }
    async fn archive_project(
        &self,
        request: tonic::Request<pb::ArchiveProjectRequest>,
    ) -> std::result::Result<tonic::Response<pb::ProjectReply>, tonic::Status> {
        self.call(request, "archive_project").await
    }
    async fn delete_project(
        &self,
        request: tonic::Request<pb::DeleteProjectRequest>,
    ) -> std::result::Result<tonic::Response<pb::OkReply>, tonic::Status> {
        self.call(request, "delete_project").await
    }
}
