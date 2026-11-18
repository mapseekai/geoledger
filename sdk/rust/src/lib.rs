//! GeoLedger client: ordinary business methods, with a private network transport.
mod input;
mod models;
use geoledger_rpc::v1 as pb;
pub use models::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tonic::{
    Request, Status,
    metadata::MetadataValue,
    service::Interceptor,
    transport::{Channel, Endpoint},
};

/// Stable SDK error. No transport-specific error inspection is required.
///
/// [`std::error::Error::source`] exposes a sanitized local diagnostic chain:
/// JSON category/position, gRPC code, or transport stage and I/O error kind.
/// Raw server payloads, endpoints and credentials are never copied into that chain.
/// Diagnostics survive cloning but are omitted from serialization and are absent
/// after deserialization. Use [`Error::new`] instead of a struct literal.
#[derive(Clone, Debug, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
    pub conflicts: Option<Box<Conflicts>>,
    /// A write may have completed. Confirm its state or retry the same publication.
    pub uncertain: bool,
    #[serde(skip)]
    #[source]
    diagnostic: Option<Arc<Diagnostic>>,
}
/// Deliberately stores only classifications, never the original error text.
#[derive(Debug, thiserror::Error)]
#[error("{description}")]
struct Diagnostic {
    description: String,
    #[source]
    cause: Option<Arc<Diagnostic>>,
}
impl Diagnostic {
    fn json(error: &serde_json::Error) -> Self {
        Self {
            description: format!(
                "JSON {:?} at line {} column {}",
                error.classify(),
                error.line(),
                error.column()
            ),
            cause: None,
        }
    }
    fn transport(stage: &'static str, error: &(dyn std::error::Error + 'static)) -> Self {
        // Traverse without formatting opaque causes. Bound traversal even for a
        // third-party error with a cyclic or unexpectedly deep source chain.
        let mut next = Some(error);
        let mut cause = None;
        for _ in 0..16 {
            let Some(error) = next else { break };
            if let Some(io) = error.downcast_ref::<std::io::Error>() {
                cause = Some(Arc::new(Self {
                    description: format!("I/O {:?}", io.kind()),
                    cause: None,
                }));
                break;
            }
            next = error.source();
        }
        Self {
            description: stage.into(),
            cause,
        }
    }
}
impl Error {
    /// Construct a business error without a local diagnostic source.
    /// Optional public fields may then be set by the caller; `uncertain` defaults
    /// to false. This constructor replaces external struct-literal construction.
    ///
    /// ```
    /// use geoledger_client::Error;
    /// let mut error = Error::new("unavailable", "service unavailable");
    /// error.uncertain = true;
    /// assert_eq!(error.code, "unavailable");
    /// ```
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            request_id: None,
            conflicts: None,
            uncertain: false,
            diagnostic: None,
        }
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_argument", message)
    }
    fn diagnosed(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostic = Some(Arc::new(diagnostic));
        self
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid("invalid JSON value").diagnosed(Diagnostic::json(&e))
    }
}
impl From<Status> for Error {
    fn from(status: Status) -> Self {
        use prost::Message;
        let detail = pb::RpcStatus::decode(status.details())
            .ok()
            .filter(|envelope| envelope.code == status.code() as i32)
            .and_then(|envelope| {
                envelope
                    .details
                    .into_iter()
                    .find(|d| d.type_url == "type.googleapis.com/geoledger.v1.ErrorDetail")
            })
            .and_then(|d| pb::ErrorDetail::decode(d.value.as_slice()).ok())
            .filter(|detail| !detail.code.trim().is_empty());
        let code = match status.code() {
            tonic::Code::Unauthenticated => "unauthenticated",
            tonic::Code::PermissionDenied => "permission_denied",
            tonic::Code::NotFound => "not_found",
            tonic::Code::Aborted | tonic::Code::AlreadyExists | tonic::Code::FailedPrecondition => {
                "conflict"
            }
            tonic::Code::InvalidArgument => "invalid_argument",
            // Tonic uses an undetailed OutOfRange for both encoding and decoding
            // message limits. It does not identify which peer rejected the message.
            tonic::Code::OutOfRange => "resource_exhausted",
            tonic::Code::DeadlineExceeded => "deadline_exceeded",
            tonic::Code::Cancelled => "cancelled",
            tonic::Code::ResourceExhausted => "resource_exhausted",
            _ => "unavailable",
        };
        // A proxy can reject a retry while an earlier publication still runs.
        // Undetailed or malformed conflict-family statuses prove no business state.
        // Only this exact server response permanently invalidates an earlier
        // expected version. A partial detail or a mismatched transport status
        // can describe this retry, but cannot settle a previous publication.
        let invalid_version_proof = detail.as_ref().is_some_and(|d| {
            d.code == "workspace_version_conflict"
                && (status.code() != tonic::Code::Aborted
                    || d.message.trim().is_empty()
                    || d.request_id.trim().is_empty()
                    || d.conflicts.is_some())
        });
        let uncertain = invalid_version_proof
            || matches!(
                status.code(),
                tonic::Code::DeadlineExceeded
                    | tonic::Code::Cancelled
                    | tonic::Code::Unavailable
                    | tonic::Code::Unknown
                    | tonic::Code::Internal
                    | tonic::Code::DataLoss
                    | tonic::Code::ResourceExhausted
                    | tonic::Code::OutOfRange
            )
            || (detail.is_none()
                && matches!(
                    status.code(),
                    tonic::Code::Aborted
                        | tonic::Code::AlreadyExists
                        | tonic::Code::FailedPrecondition
                ));
        let diagnostic = Some(Arc::new(Diagnostic {
            description: format!("gRPC {:?}", status.code()),
            cause: None,
        }));
        match detail {
            Some(d) => Self {
                code: d.code,
                message: d.message,
                request_id: Some(d.request_id),
                conflicts: d
                    .conflicts
                    .and_then(|c| serde_json::to_value(c).ok())
                    .and_then(|mut v| {
                        normalize(&mut v).ok()?;
                        decode(v).ok().map(Box::new)
                    }),
                uncertain,
                diagnostic,
            },
            None => Self {
                code: code.into(),
                message: status.message().into(),
                request_id: None,
                conflicts: None,
                uncertain,
                diagnostic,
            },
        }
    }
}
#[derive(Clone)]
struct Authentication {
    value: MetadataValue<tonic::metadata::Ascii>,
    timeout: Duration,
}
impl Interceptor for Authentication {
    fn call(&mut self, mut r: Request<()>) -> Result<Request<()>, Status> {
        r.metadata_mut().insert("authorization", self.value.clone());
        r.set_timeout(self.timeout);
        Ok(r)
    }
}
type Transport = pb::geo_ledger_client::GeoLedgerClient<
    tonic::service::interceptor::InterceptedService<Channel, Authentication>,
>;
/// Cloneable API-Key client. Clones share the underlying connection and may issue
/// independent requests concurrently. Dropping the last clone releases its channel.
#[derive(Clone)]
pub struct Client {
    rpc: Transport,
}
fn response_error() -> Error {
    Error {
        code: "invalid_response".into(),
        message: "invalid data received from server".into(),
        request_id: None,
        conflicts: None,
        uncertain: true,
        diagnostic: None,
    }
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value)
        .map_err(|error| response_error().diagnosed(Diagnostic::json(&error)))
}
fn normalize(value: &mut Value) -> Result<(), Error> {
    fn feature(v: &mut Value) -> Result<(), Error> {
        if !v.is_null() {
            let raw = v
                .get("geojson")
                .and_then(Value::as_str)
                .ok_or_else(response_error)?;
            *v = serde_json::from_str(raw)
                .map_err(|error| response_error().diagnosed(Diagnostic::json(&error)))?
        }
        Ok(())
    }
    if let Some(rows) = value.get_mut("features").and_then(Value::as_array_mut) {
        for row in rows {
            feature(row)?
        }
    }
    for key in ["changes", "conflicts"] {
        if let Some(rows) = value.get_mut(key).and_then(Value::as_array_mut) {
            for row in rows {
                for key in ["base", "draft", "before", "after", "current"] {
                    if let Some(v) = row.get_mut(key) {
                        feature(v)?
                    }
                }
            }
        }
    }
    if let Some(rows) = value.get_mut("events").and_then(Value::as_array_mut) {
        for row in rows {
            if let Some(v) = row.get_mut("detail_json") {
                let raw = v.as_str().ok_or_else(response_error)?;
                *v = serde_json::from_str(raw)
                    .map_err(|error| response_error().diagnosed(Diagnostic::json(&error)))?
            }
        }
    }
    Ok(())
}

/// Connection options. Plaintext `http://` is accepted only for loopback hosts unless
/// `allow_insecure` is set (or `GL_ALLOW_INSECURE_TRANSPORT=true`), because the bearer
/// api_key would otherwise cross the network unencrypted.
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub timeout: Duration,
    pub allow_insecure: bool,
    /// Extra PEM CA bundle trusted for `https://` endpoints (private PKI).
    pub ca_pem: Option<Vec<u8>>,
    /// Per-message outbound and inbound Protobuf byte limits.
    pub message_limits: MessageLimits,
}
impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            allow_insecure: false,
            ca_pem: None,
            message_limits: MessageLimits::default(),
        }
    }
}

/// Per-message Protobuf byte limits for one client and its clones.
///
/// Both limits must be in `1..=usize::MAX`. They count serialized message bytes,
/// excluding the gRPC frame header; this client does not enable compression.
/// [`MessageLimits::default`] uses 64 MiB for both directions. gRPC framing limits and the
/// server's own limits still apply. These are not process-memory limits: encoding,
/// JSON conversion, and concurrent requests may allocate additional memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageLimits {
    max_encoding_bytes: usize,
    max_decoding_bytes: usize,
}
impl MessageLimits {
    /// Set the largest outbound and inbound message sizes, respectively, in bytes.
    /// Returns `invalid_argument` if either limit is zero.
    ///
    /// ```
    /// use geoledger_client::MessageLimits;
    /// let limits = MessageLimits::new(16 * 1024 * 1024, 64 * 1024 * 1024)?;
    /// assert_eq!(limits.max_decoding_bytes(), 64 * 1024 * 1024);
    /// # Ok::<(), geoledger_client::Error>(())
    /// ```
    pub fn new(max_encoding_bytes: usize, max_decoding_bytes: usize) -> Result<Self, Error> {
        if max_encoding_bytes == 0 || max_decoding_bytes == 0 {
            return Err(Error::invalid(
                "message limits must be positive byte counts",
            ));
        }
        Ok(Self {
            max_encoding_bytes,
            max_decoding_bytes,
        })
    }
    pub fn max_encoding_bytes(self) -> usize {
        self.max_encoding_bytes
    }
    pub fn max_decoding_bytes(self) -> usize {
        self.max_decoding_bytes
    }
}
impl Default for MessageLimits {
    fn default() -> Self {
        Self {
            max_encoding_bytes: 64 * 1024 * 1024,
            max_decoding_bytes: 64 * 1024 * 1024,
        }
    }
}

/// True for `localhost` and loopback IP literals.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
fn insecure_opt_in() -> bool {
    std::env::var("GL_ALLOW_INSECURE_TRANSPORT")
        .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

impl Client {
    /// Connect using explicit TLS, timeout, and per-message byte limits.
    ///
    /// Use [`ConnectOptions::default`] for a 30-second request timeout and 64 MiB
    /// limits in each direction. Connection establishment is bounded to ten seconds.
    /// Plaintext is restricted to loopback unless explicitly allowed. Invalid
    /// configuration returns `invalid_argument`; connection failures return `unavailable`.
    /// The limits are retained by every client clone. A rejected response can
    /// arrive after a write commits, so message-limit errors remain uncertain.
    /// Retry an identical pending publication, using a new client with a larger
    /// decoding limit if needed. This does not raise the server's own limits.
    pub async fn connect(
        endpoint: impl Into<String>,
        api_key: &str,
        options: ConnectOptions,
    ) -> Result<Self, Error> {
        let timeout = options.timeout;
        let message_limits = options.message_limits;
        let endpoint = endpoint.into();
        let mut e = Endpoint::from_shared(endpoint).map_err(|error| {
            Error::invalid("endpoint must be http(s)://host:port")
                .diagnosed(Diagnostic::transport("invalid endpoint", &error))
        })?;
        if !matches!(e.uri().scheme_str(), Some("http" | "https"))
            || e.uri().authority().is_none()
            || e.uri()
                .authority()
                .is_some_and(|a| a.as_str().contains('@'))
            || !matches!(e.uri().path(), "" | "/")
            || e.uri().query().is_some()
        {
            return Err(Error::invalid("endpoint must be http(s)://host:port"));
        }
        if api_key.len() != 68
            || !api_key.starts_with("glk_")
            || !api_key[4..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || timeout.is_zero()
        {
            return Err(Error::invalid("API Key and positive timeout required"));
        }
        if e.uri().scheme_str() == Some("http")
            && !e.uri().host().is_some_and(is_loopback_host)
            && !options.allow_insecure
            && !insecure_opt_in()
        {
            return Err(Error::invalid(
                "refusing to send credentials over plaintext http to a non-loopback host; use https or opt in with allow_insecure / GL_ALLOW_INSECURE_TRANSPORT=true",
            ));
        }
        e = e.connect_timeout(Duration::from_secs(10)).timeout(timeout);
        if e.uri().scheme_str() == Some("https") {
            let tls = tonic::transport::ClientTlsConfig::new();
            let tls = if let Some(pem) = options.ca_pem {
                tls.ca_certificate(tonic::transport::Certificate::from_pem(pem))
            } else {
                tls.with_native_roots()
            };
            e = e.tls_config(tls).map_err(|error| {
                Error::invalid("TLS configuration failed")
                    .diagnosed(Diagnostic::transport("TLS configuration failed", &error))
            })?
        }
        let auth = Authentication {
            value: format!("Bearer {api_key}")
                .parse()
                .map_err(|_| Error::invalid("invalid api_key"))?,
            timeout,
        };
        let channel = e.connect().await.map_err(|error| {
            Error::new("unavailable", "could not connect to GeoLedger").diagnosed(
                Diagnostic::transport("connection failed (DNS, TCP or TLS)", &error),
            )
        })?;
        Ok(Self {
            rpc: pb::geo_ledger_client::GeoLedgerClient::with_interceptor(channel, auth)
                .max_decoding_message_size(message_limits.max_decoding_bytes)
                .max_encoding_message_size(message_limits.max_encoding_bytes),
        })
    }
    /// Advanced business-JSON entry point, also used by the CLI. GeoJSON is a plain object.
    /// Requires the current operation's explicit required fields and JSON types.
    /// Unknown request fields are rejected before sending the RPC. Feature properties
    /// remain ordinary, unrestricted JSON values.
    pub async fn execute(&self, operation: &str, mut value: Value) -> Result<Value, Error> {
        input::validate(operation, &value)?;
        for key in ["edits", "resolutions"] {
            if let Some(edits) = value.get_mut(key).and_then(Value::as_array_mut) {
                for edit in edits {
                    if edit.get("feature").is_none() {
                        return Err(Error::invalid(
                            "edit requires an explicit feature; use null for deletion",
                        ));
                    }
                    if let Some(feature) = edit.get_mut("feature")
                        && !feature.is_null()
                    {
                        *feature = json!({"geojson":serde_json::to_string(feature)?})
                    }
                }
            }
        }
        if operation == "save"
            && let Some(schema) = value.get_mut("schema_edits")
            && !schema.is_null()
        {
            if !schema.is_array() {
                return Err(Error::invalid("schema_edits must be an array"));
            }
            let edits = schema.take();
            *schema = json!({"edits":edits});
        }
        let mut result = self.invoke(operation, value).await?;
        normalize(&mut result)?;
        Ok(result)
    }
    async fn invoke(&self, operation: &str, value: Value) -> Result<Value, Error> {
        let mut rpc = self.rpc.clone();
        Ok(match operation {
            "info" => serde_json::to_value(rpc.info(pb::Empty {}).await?.into_inner())?,
            "create_project" => serde_json::to_value(
                rpc.create_project(serde_json::from_value::<pb::NameRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "list_projects" => serde_json::to_value(
                rpc.list_projects(serde_json::from_value::<pb::PageRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "get_project" => serde_json::to_value(
                rpc.get_project(serde_json::from_value::<pb::ProjectRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "set_member" => serde_json::to_value(
                rpc.set_member(serde_json::from_value::<pb::MemberRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "rename_project" => serde_json::to_value(
                rpc.rename_project(serde_json::from_value::<pb::RenameProjectRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "rename_dataset" => serde_json::to_value(
                rpc.rename_dataset(serde_json::from_value::<pb::RenameDatasetRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "delete_dataset" => serde_json::to_value(
                rpc.delete_dataset(serde_json::from_value::<pb::DeleteDatasetRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "create_dataset" => serde_json::to_value(
                rpc.create_dataset(serde_json::from_value::<pb::DatasetRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "list_datasets" => serde_json::to_value(
                rpc.list_datasets(serde_json::from_value::<pb::ProjectPageRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "create_workspace" => serde_json::to_value(
                rpc.create_workspace(serde_json::from_value::<pb::ProjectRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "list_workspaces" => serde_json::to_value(
                rpc.list_workspaces(serde_json::from_value::<pb::ProjectPageRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "get_workspace" => serde_json::to_value(
                rpc.get_workspace(serde_json::from_value::<pb::WorkspaceRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "save" => serde_json::to_value(
                rpc.save(serde_json::from_value::<pb::SaveRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "discard" => serde_json::to_value(
                rpc.discard(serde_json::from_value::<pb::VersionRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "schema" => serde_json::to_value(
                rpc.schema(serde_json::from_value::<pb::SchemaRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "features" => serde_json::to_value(
                rpc.features(serde_json::from_value::<pb::FeaturesRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "diff" => serde_json::to_value(
                rpc.diff(serde_json::from_value::<pb::DiffRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "conflicts" => serde_json::to_value(
                rpc.conflicts(serde_json::from_value::<pb::DiffRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "history" => serde_json::to_value(
                rpc.history(serde_json::from_value::<pb::HistoryRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "workspace_summary" => serde_json::to_value(
                rpc.workspace_summary(serde_json::from_value::<pb::WorkspaceRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "commit_summary" => serde_json::to_value(
                rpc.commit_summary(serde_json::from_value::<pb::SummaryCommitRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "commit" => serde_json::to_value(
                rpc.commit(serde_json::from_value::<pb::CommitRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "audit" => serde_json::to_value(
                rpc.audit(serde_json::from_value::<pb::HistoryRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "publish" => {
                let request: pb::PublishRequest = serde_json::from_value(value)?;
                let workspace = request.workspace.clone();
                let next_version = request.expected_workspace_version.checked_add(1);
                let receipt = rpc.publish(request).await?.into_inner();
                // A decodable protobuf can still be empty or belong to a
                // different workspace. It does not settle this publication.
                if workspace.is_empty()
                    || receipt.workspace != workspace
                    || receipt.status != "published"
                    || receipt.revision <= 0
                    || receipt.version <= 0
                    || Some(receipt.version) != next_version
                {
                    let mut error = Error::new("invalid_response", "invalid publication receipt");
                    error.uncertain = true;
                    return Err(error);
                }
                serde_json::to_value(receipt)?
            }
            "resolve" => serde_json::to_value(
                rpc.resolve(serde_json::from_value::<pb::ResolveRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "rebase" => serde_json::to_value(
                rpc.rebase(serde_json::from_value::<pb::ResolveRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "restore" => serde_json::to_value(
                rpc.restore(serde_json::from_value::<pb::RestoreRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "list_members" => serde_json::to_value(
                rpc.list_members(serde_json::from_value::<pb::ProjectPageRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "remove_member" => serde_json::to_value(
                rpc.remove_member(serde_json::from_value::<pb::MemberRefRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "archive_project" => serde_json::to_value(
                rpc.archive_project(serde_json::from_value::<pb::ArchiveProjectRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            "delete_project" => serde_json::to_value(
                rpc.delete_project(serde_json::from_value::<pb::DeleteProjectRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
            _ => return Err(Error::invalid(format!("unknown operation: {operation}"))),
        })
    }
}

impl Client {
    pub async fn info(&self) -> Result<ServerInfo, Error> {
        decode(self.execute("info", json!({})).await?)
    }
    pub async fn create_project(&self, name: &str) -> Result<Project, Error> {
        decode(self.execute("create_project", json!({"name":name})).await?)
    }
    pub async fn project(&self, project: &str) -> Result<Project, Error> {
        decode(
            self.execute("get_project", json!({"project":project}))
                .await?,
        )
    }
    /// List one page of accessible projects, ordered by project ID. Pass the last
    /// returned ID as `Page::after`; an empty page ends traversal.
    pub async fn projects(&self, page: Page) -> Result<Vec<Project>, Error> {
        decode(
            self.execute("list_projects", serde_json::to_value(page)?)
                .await?["projects"]
                .take(),
        )
    }
    /// Set a user’s project role, subject to server authorization and last-owner
    /// invariants. Permission or validation failures are returned as SDK errors.
    pub async fn set_member(&self, project: &str, subject: &str, role: &str) -> Result<(), Error> {
        self.execute(
            "set_member",
            json!({"project":project,"subject":subject,"role":role}),
        )
        .await?;
        Ok(())
    }
    /// Active members ordered by subject. Continue with the last member's subject
    /// in `Page::after`; an empty page ends traversal.
    pub async fn members(&self, project: &str, page: Page) -> Result<Vec<Member>, Error> {
        decode(
            self.execute(
                "list_members",
                json!({"project":project,"after":page.after,"limit":page.limit}),
            )
            .await?["members"]
                .take(),
        )
    }
    /// Remove a member (owners and administrators), or leave the project when
    /// `subject` is the caller. The last owner cannot be removed.
    pub async fn remove_member(&self, project: &str, subject: &str) -> Result<(), Error> {
        self.execute(
            "remove_member",
            json!({"project":project,"subject":subject}),
        )
        .await?;
        Ok(())
    }
    /// Archive (read-only) or reactivate a project.
    pub async fn archive_project(&self, project: &str, archived: bool) -> Result<Project, Error> {
        decode(
            self.execute(
                "archive_project",
                json!({"project":project,"archived":archived}),
            )
            .await?,
        )
    }
    /// Delete all project data and retain its audit tombstone. `confirm_name` must equal the project name.
    pub async fn delete_project(&self, project: &str, confirm_name: &str) -> Result<(), Error> {
        self.execute(
            "delete_project",
            json!({"project":project,"confirm_name":confirm_name}),
        )
        .await?;
        Ok(())
    }
    pub async fn rename_project(&self, project: &str, name: &str) -> Result<Project, Error> {
        decode(
            self.execute("rename_project", json!({"project":project,"name":name}))
                .await?,
        )
    }
    pub async fn rename_dataset(
        &self,
        project: &str,
        dataset: &str,
        name: &str,
    ) -> Result<Dataset, Error> {
        decode(
            self.execute(
                "rename_dataset",
                json!({"project":project,"dataset":dataset,"name":name}),
            )
            .await?,
        )
    }
    pub async fn delete_dataset(
        &self,
        project: &str,
        dataset: &str,
        confirm_name: &str,
    ) -> Result<(), Error> {
        self.execute(
            "delete_dataset",
            json!({"project":project,"dataset":dataset,"confirm_name":confirm_name}),
        )
        .await?;
        Ok(())
    }
    pub async fn create_dataset(
        &self,
        project: &str,
        name: &str,
        geometry_type: &str,
    ) -> Result<Dataset, Error> {
        self.create_dataset_with_dimension(project, name, geometry_type, 2)
            .await
    }
    pub async fn create_dataset_with_dimension(
        &self,
        project: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: u32,
    ) -> Result<Dataset, Error> {
        self.create_dataset_with_crs(
            project,
            name,
            geometry_type,
            coordinate_dimension,
            "EPSG:4326",
        )
        .await
    }
    /// Store coordinates unchanged in the declared immutable dataset CRS.
    pub async fn create_dataset_with_crs(
        &self,
        project: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: u32,
        crs: &str,
    ) -> Result<Dataset, Error> {
        decode(
            self.execute(
                "create_dataset",
                json!({"project":project,"name":name,"geometry_type":geometry_type,"coordinate_dimension":coordinate_dimension,"crs":crs}),
            )
            .await?,
        )
    }
    /// Register an existing PostGIS table, creating its initial published revision.
    pub async fn register_table(
        &self,
        project: &str,
        name: &str,
        table: &PostgisTable,
    ) -> Result<Dataset, Error> {
        decode(
            self.execute(
                "create_dataset",
                json!({"project":project,"name":name,"postgis_table":table}),
            )
            .await?,
        )
    }
    pub async fn datasets(&self, project: &str, page: Page) -> Result<Vec<Dataset>, Error> {
        decode(
            self.execute(
                "list_datasets",
                json!({"project":project,"after":page.after,"limit":page.limit}),
            )
            .await?["datasets"]
                .take(),
        )
    }
    pub async fn create_workspace(&self, project: &str) -> Result<Workspace, Error> {
        let info = decode(
            self.execute("create_workspace", json!({"project":project}))
                .await?,
        )?;
        Ok(Workspace {
            client: self.clone(),
            project: project.into(),
            info,
            pending: None,
        })
    }
    pub async fn workspace(&self, project: &str, workspace: &str) -> Result<Workspace, Error> {
        let info = decode(
            self.execute(
                "get_workspace",
                json!({"project":project,"workspace":workspace}),
            )
            .await?,
        )?;
        Ok(Workspace {
            client: self.clone(),
            project: project.into(),
            info,
            pending: None,
        })
    }
    pub async fn workspaces(&self, project: &str, page: Page) -> Result<Vec<WorkspaceInfo>, Error> {
        decode(
            self.execute(
                "list_workspaces",
                json!({"project":project,"after":page.after,"limit":page.limit}),
            )
            .await?["workspaces"]
                .take(),
        )
    }
    pub async fn features(
        &self,
        project: &str,
        dataset: &str,
        query: FeatureQuery,
    ) -> Result<FeaturePage, Error> {
        let mut input = serde_json::to_value(query)?;
        input["project"] = json!(project);
        input["dataset"] = json!(dataset);
        decode(self.execute("features", input).await?)
    }
    /// Read field definitions from one published revision or workspace version.
    pub async fn schema(
        &self,
        project: &str,
        dataset: &str,
        query: SchemaQuery,
    ) -> Result<DatasetSchema, Error> {
        if query.workspace.is_some() && query.revision.is_some() {
            return Err(Error::invalid(
                "schema query cannot combine workspace and revision",
            ));
        }
        let mut input = serde_json::to_value(query)?;
        input["project"] = json!(project);
        input["dataset"] = json!(dataset);
        decode(self.execute("schema", input).await?)
    }
    /// Read commits in ascending revision order. `next_after` is absent at the
    /// current snapshot end; later publications may appear in a later request.
    pub async fn history(
        &self,
        project: &str,
        after: i64,
        limit: Option<i64>,
    ) -> Result<HistoryPage, Error> {
        decode(
            self.execute(
                "history",
                json!({"project":project,"after":after,"limit":limit}),
            )
            .await?,
        )
    }
    pub async fn workspace_summary(
        &self,
        project: &str,
        workspace: &str,
    ) -> Result<ChangeSummary, Error> {
        decode(
            self.execute(
                "workspace_summary",
                json!({"project":project,"workspace":workspace}),
            )
            .await?,
        )
    }
    pub async fn commit_summary(
        &self,
        project: &str,
        revision: i64,
    ) -> Result<ChangeSummary, Error> {
        decode(
            self.execute(
                "commit_summary",
                json!({"project":project,"revision":revision}),
            )
            .await?,
        )
    }
    /// Read one page of a commit's changes. Continue with `next_after` in
    /// `Page::after`; `None` ends traversal.
    pub async fn commit(
        &self,
        project: &str,
        revision: i64,
        page: Page,
    ) -> Result<CommitChanges, Error> {
        decode(self.execute("commit",json!({"project":project,"revision":revision,"after":page.after,"limit":page.limit})).await?)
    }
    pub async fn audit(
        &self,
        project: &str,
        after: i64,
        limit: Option<i64>,
    ) -> Result<AuditPage, Error> {
        decode(
            self.execute(
                "audit",
                json!({"project":project,"after":after,"limit":limit}),
            )
            .await?,
        )
    }
    /// Submit an immutable publication intent. On an uncertain failure or local
    /// cancellation, retry this exact intent, including its request ID and version.
    /// The server replays a committed receipt rather than publishing twice.
    pub async fn publish(&self, publication: &Publication) -> Result<PublicationResult, Error> {
        decode(
            self.execute("publish", serde_json::to_value(publication)?)
                .await?,
        )
    }
    pub async fn restore(&self, project: &str, revision: i64) -> Result<Workspace, Error> {
        let info = decode(
            self.execute("restore", json!({"project":project,"revision":revision}))
                .await?,
        )?;
        Ok(Workspace {
            client: self.clone(),
            project: project.into(),
            info,
            pending: None,
        })
    }
}
/// A personal draft. Mutating methods keep its optimistic version up to date.
/// Keep this handle (or persist pending_publication) when a publication outcome is uncertain.
/// Mutations require exclusive `&mut self`; do not create independent handles to
/// coordinate concurrent writes without handling optimistic-version conflicts.
/// Dropping a future stops local waiting, not necessarily server-side work.
pub struct Workspace {
    client: Client,
    project: String,
    info: WorkspaceInfo,
    pending: Option<Publication>,
}
impl Workspace {
    pub fn id(&self) -> &str {
        &self.info.id
    }
    pub fn info(&self) -> &WorkspaceInfo {
        &self.info
    }
    pub fn pending_publication(&self) -> Option<&Publication> {
        self.pending.as_ref()
    }
    fn editable(&self) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::invalid(
                "retry the pending publication before changing this workspace",
            ));
        }
        if self.info.status != "open" {
            return Err(Error::invalid("workspace is closed"));
        }
        Ok(())
    }
    fn input(&self) -> Value {
        json!({"project":self.project,"workspace":self.info.id,"expected_workspace_version":self.info.version})
    }
    /// Reload the server’s workspace version and status. A pending publication
    /// remains intact; refreshing does not resolve its uncertain outcome.
    pub async fn refresh(&mut self) -> Result<&WorkspaceInfo, Error> {
        self.info = decode(
            self.client
                .execute(
                    "get_workspace",
                    json!({"project":self.project,"workspace":self.info.id}),
                )
                .await?,
        )?;
        Ok(&self.info)
    }
    pub async fn save(&mut self, dataset: &str, feature: Value) -> Result<SaveResult, Error> {
        let id = feature
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid("GeoJSON Feature requires a string id"))?
            .to_owned();
        self.save_batch(&[Edit {
            dataset: dataset.into(),
            feature_id: id,
            feature: Some(feature),
        }])
        .await
    }
    pub async fn delete(&mut self, dataset: &str, id: &str) -> Result<SaveResult, Error> {
        self.save_batch(&[Edit {
            dataset: dataset.into(),
            feature_id: id.into(),
            feature: None,
        }])
        .await
    }
    /// Save an atomic batch against this workspace’s current optimistic version.
    /// A pending publication or closed workspace rejects edits. Cancellation may
    /// leave the server changed; refresh state before deciding whether to retry.
    pub async fn save_batch(&mut self, edits: &[Edit]) -> Result<SaveResult, Error> {
        self.save_changes(edits, None).await
    }
    /// Save features and the complete schema intent atomically. None preserves
    /// existing schema intent; Some(&[]) clears it relative to the base revision.
    pub async fn save_changes(
        &mut self,
        edits: &[Edit],
        schema_edits: Option<&[SchemaEdit]>,
    ) -> Result<SaveResult, Error> {
        self.editable()?;
        let mut input = self.input();
        input["edits"] = serde_json::to_value(edits)?;
        if let Some(schema) = schema_edits {
            input["schema_edits"] = serde_json::to_value(schema)?;
        }
        let result: SaveResult = decode(self.client.execute("save", input).await?)?;
        self.info.version = result.version;
        Ok(result)
    }
    pub async fn features(
        &self,
        dataset: &str,
        mut query: FeatureQuery,
    ) -> Result<FeaturePage, Error> {
        if query.revision.is_some() {
            return Err(Error::invalid(
                "draft queries cannot specify a published revision",
            ));
        }
        query.workspace = Some(self.info.id.clone());
        self.client.features(&self.project, dataset, query).await
    }
    pub async fn schema(&self, dataset: &str) -> Result<DatasetSchema, Error> {
        self.client
            .schema(
                &self.project,
                dataset,
                SchemaQuery {
                    workspace: Some(self.info.id.clone()),
                    revision: None,
                },
            )
            .await
    }
    /// Read draft changes. Continue with `next_after` in `Page::after`;
    /// `None` ends traversal. Verify `version` is unchanged between pages.
    pub async fn diff(&self, page: Page) -> Result<Diff, Error> {
        decode(self.client.execute("diff",json!({"project":self.project,"workspace":self.info.id,"after":page.after,"limit":page.limit})).await?)
    }
    /// Read conflicts. Continue with [`Conflicts::next_after`] in `Page::after`;
    /// `None` means there is no continuation page.
    pub async fn conflicts(&self, page: Page) -> Result<Conflicts, Error> {
        decode(self.client.execute("conflicts",json!({"project":self.project,"workspace":self.info.id,"after":page.after,"limit":page.limit})).await?)
    }
    /// Publish the draft, retaining the immutable intent before awaiting the RPC.
    ///
    /// After cancellation or an uncertain error, retry with the identical message
    /// or recover the intent through [`Workspace::pending_publication`]. Pending
    /// publication blocks edits. A successful retry replays the committed receipt.
    /// A definitive first-attempt rejection permits correcting this workspace.
    /// After an earlier unknown attempt, only a definitive, validated business
    /// `workspace_version_conflict` can release the pending intent. Other errors,
    /// including merge conflicts, do not prove the earlier attempt cannot commit.
    pub async fn publish(&mut self, message: &str) -> Result<PublicationResult, Error> {
        let was_pending = self.pending.is_some();
        if let Some(p) = &self.pending {
            if p.message != message {
                return Err(Error::invalid(
                    "retry the pending publication with the original message",
                ));
            }
        } else {
            self.editable()?;
            self.pending = Some(Publication {
                project: self.project.clone(),
                workspace: self.info.id.clone(),
                expected_workspace_version: self.info.version,
                request_id: uuid::Uuid::new_v4().to_string(),
                message: message.into(),
            })
        }
        let publication = self
            .pending
            .as_ref()
            .ok_or_else(|| Error::invalid("missing publication"))?;
        match self.client.publish(publication).await {
            Ok(result) => {
                self.info.version = result.version;
                self.info.status = result.status.clone();
                Ok(result)
            }
            Err(e) => {
                // A merge conflict can disappear if HEAD changes again. Only an
                // authoritative workspace version/closed-state rejection proves
                // that an earlier publication with this version cannot still commit.
                if !e.uncertain && (!was_pending || e.code == "workspace_version_conflict") {
                    self.pending = None
                }
                Err(e)
            }
        }
    }
    pub async fn resolve(
        &mut self,
        head: i64,
        resolutions: &[Edit],
    ) -> Result<ResolutionResult, Error> {
        self.editable()?;
        let mut input = self.input();
        input["expected_head"] = json!(head);
        input["resolutions"] = serde_json::to_value(resolutions)?;
        let r: ResolutionResult = decode(self.client.execute("resolve", input).await?)?;
        self.info.version = r.version;
        Ok(r)
    }
    pub async fn rebase(&mut self, head: i64, resolutions: &[Edit]) -> Result<RebaseResult, Error> {
        self.editable()?;
        let mut input = self.input();
        input["expected_head"] = json!(head);
        input["resolutions"] = serde_json::to_value(resolutions)?;
        let r: RebaseResult = decode(self.client.execute("rebase", input).await?)?;
        self.info.version = r.version;
        self.info.base_revision = r.base_revision;
        Ok(r)
    }
    pub async fn discard(&mut self) -> Result<DiscardResult, Error> {
        self.editable()?;
        let r: DiscardResult = decode(self.client.execute("discard", self.input()).await?)?;
        self.info.version = r.version;
        self.info.status = r.status.clone();
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_page_continuations_survive_wire_conversion() -> Result<(), Box<dyn std::error::Error>>
    {
        for cursor in [None, Some("dataset/key/with/slashes".to_owned())] {
            let diff: Diff = decode(serde_json::to_value(pb::DiffReply {
                next_after: cursor.clone(),
                ..Default::default()
            })?)?;
            let commit: CommitChanges = decode(serde_json::to_value(pb::CommitReply {
                next_after: cursor.clone(),
                ..Default::default()
            })?)?;
            assert_eq!(diff.next_after, cursor);
            assert_eq!(commit.next_after, cursor);
        }
        for cursor in [None, Some(9007199254740993_i64)] {
            let history: HistoryPage = decode(serde_json::to_value(pb::HistoryReply {
                next_after: cursor,
                ..Default::default()
            })?)?;
            let audit: AuditPage = decode(serde_json::to_value(pb::AuditReply {
                next_after: cursor,
                ..Default::default()
            })?)?;
            assert_eq!(history.next_after, cursor);
            assert_eq!(audit.next_after, cursor);
        }
        Ok(())
    }

    #[test]
    fn diagnostics_survive_clone_but_not_wire_roundtrip() -> Result<(), Box<dyn std::error::Error>>
    {
        use std::error::Error as _;
        let original =
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "secret endpoint/key");
        let error = Error::new("unavailable", "could not connect to GeoLedger")
            .diagnosed(Diagnostic::transport("connection failed", &original));
        let clone = error.clone();
        assert_eq!(
            clone.to_string(),
            "unavailable: could not connect to GeoLedger"
        );
        let source = clone.source().ok_or("missing diagnostic")?;
        assert_eq!(source.to_string(), "connection failed");
        assert_eq!(
            source.source().ok_or("missing I/O kind")?.to_string(),
            "I/O ConnectionRefused"
        );
        assert!(!format!("{clone:?}").contains("secret"));
        assert!(!clone.to_string().contains("secret"));
        let mut cause = clone.source();
        while let Some(source) = cause {
            assert!(!source.to_string().contains("secret"));
            assert!(!format!("{source:?}").contains("secret"));
            cause = source.source();
        }
        let encoded = serde_json::to_value(&clone)?;
        assert_eq!(
            encoded,
            json!({"code":"unavailable","message":"could not connect to GeoLedger","request_id":null,"conflicts":null,"uncertain":false})
        );
        let decoded: Error = serde_json::from_value(encoded)?;
        assert!(decoded.source().is_none());
        assert_eq!(decoded.to_string(), clone.to_string());
        assert!(
            Error::new("invalid_argument", "invalid input")
                .source()
                .is_none()
        );
        Ok(())
    }
    #[test]
    fn json_and_rpc_sources_exclude_payload_text() -> Result<(), Box<dyn std::error::Error>> {
        use std::error::Error as _;
        let raw = serde_json::from_str::<u32>("\"secret payload\"")
            .err()
            .ok_or("expected JSON error")?;
        let error = Error::from(raw);
        assert_eq!(error.message, "invalid JSON value");
        assert!(
            error
                .source()
                .ok_or("missing JSON source")?
                .to_string()
                .starts_with("JSON Data at line 1 column")
        );
        assert!(!format!("{error:?}").contains("secret"));
        assert!(!error.to_string().contains("secret"));
        let status = Error::from(Status::unavailable("remote public error message"));
        assert_eq!(status.message, "remote public error message");
        assert_eq!(
            status.source().ok_or("missing RPC source")?.to_string(),
            "gRPC Unavailable"
        );
        let converted = decode::<u32>(json!("private stored value"))
            .err()
            .ok_or("expected decode error")?;
        assert_eq!(converted.code, "invalid_response");
        assert!(converted.source().is_some());
        assert!(!format!("{converted:?}").contains("private stored"));
        Ok(())
    }
    #[test]
    fn out_of_range_preserves_uncertainty_and_structured_business_code() {
        use prost::Message;
        let error = Error::from(Status::out_of_range("range rejected"));
        assert_eq!(error.code, "resource_exhausted");
        assert!(error.uncertain);
        let detail = pb::ErrorDetail {
            code: "business_range".into(),
            message: "business range rejected".into(),
            request_id: "request".into(),
            conflicts: None,
        };
        let envelope = pb::RpcStatus {
            code: tonic::Code::OutOfRange as i32,
            message: detail.message.clone(),
            details: vec![pb::ErrorAny {
                type_url: "type.googleapis.com/geoledger.v1.ErrorDetail".into(),
                value: detail.encode_to_vec(),
            }],
        };
        let error = Error::from(Status::with_details(
            tonic::Code::OutOfRange,
            "range rejected",
            envelope.encode_to_vec().into(),
        ));
        assert_eq!(error.code, "business_range");
        assert_eq!(error.message, "business range rejected");
        assert_eq!(error.request_id.as_deref(), Some("request"));
        assert!(error.uncertain);
    }
    #[test]
    fn undetailed_deadline_uses_the_current_business_code() {
        let error = Error::from(Status::deadline_exceeded("request expired"));
        assert_eq!(error.code, "deadline_exceeded");
        assert!(error.uncertain);
    }
    #[test]
    fn response_conversion_preserves_opaque_properties_and_numbers() -> Result<(), Error> {
        let original = json!({"type":"Feature","id":"one","properties":{"nested":{"geojson":"ordinary text"},"detail_json":"plain","exact":18446744073709551615u64},"geometry":null});
        let mut reply = json!({"crs":"EPSG:4326","features":[{"geojson":original.to_string()}],"revision":1,"workspace_version":null,"next_after":null});
        normalize(&mut reply)?;
        let page: FeaturePage = decode(reply)?;
        assert_eq!(page.features[0], original);
        assert!(serde_json::from_value::<Edit>(json!({"dataset":"d","feature_id":"one"})).is_err());
        assert!(
            serde_json::from_value::<Edit>(
                json!({"dataset":"d","feature_id":"one","feature":null})
            )
            .is_ok()
        );
        Ok(())
    }
}
