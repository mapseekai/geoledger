//! GeoLedger client: ordinary business methods, with a private network transport.
mod models;
use geoledger_rpc::v1 as pb;
pub use models::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, time::Duration};
use tonic::{
    Request, Status,
    metadata::MetadataValue,
    service::Interceptor,
    transport::{Channel, Endpoint},
};

/// Stable SDK error. No transport-specific error inspection is required.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
    pub conflicts: Option<Box<Conflicts>>,
    /// A write may have completed. Confirm its state or retry the same publication.
    pub uncertain: bool,
}
impl Error {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_argument".into(),
            message: message.into(),
            request_id: None,
            conflicts: None,
            uncertain: false,
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(e.to_string())
    }
}
impl From<Status> for Error {
    fn from(status: Status) -> Self {
        use prost::Message;
        let detail = pb::RpcStatus::decode(status.details())
            .ok()
            .and_then(|envelope| {
                envelope
                    .details
                    .into_iter()
                    .find(|d| d.type_url == "type.googleapis.com/geoledger.v1.ErrorDetail")
            })
            .and_then(|d| pb::ErrorDetail::decode(d.value.as_slice()).ok());
        let code = match status.code() {
            tonic::Code::Unauthenticated => "unauthenticated",
            tonic::Code::PermissionDenied => "permission_denied",
            tonic::Code::NotFound => "not_found",
            tonic::Code::Aborted | tonic::Code::AlreadyExists | tonic::Code::FailedPrecondition => {
                "conflict"
            }
            tonic::Code::InvalidArgument | tonic::Code::OutOfRange => "invalid_argument",
            tonic::Code::DeadlineExceeded => "timeout",
            tonic::Code::Cancelled => "cancelled",
            tonic::Code::ResourceExhausted => "resource_exhausted",
            _ => "unavailable",
        };
        let uncertain = matches!(
            status.code(),
            tonic::Code::DeadlineExceeded
                | tonic::Code::Cancelled
                | tonic::Code::Unavailable
                | tonic::Code::Unknown
                | tonic::Code::Internal
                | tonic::Code::DataLoss
                | tonic::Code::ResourceExhausted
        );
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
            },
            None => Self {
                code: code.into(),
                message: status.message().into(),
                request_id: None,
                conflicts: None,
                uncertain,
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
    }
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|_| response_error())
}
fn normalize(value: &mut Value) -> Result<(), Error> {
    fn feature(v: &mut Value) -> Result<(), Error> {
        if !v.is_null() {
            let raw = v
                .get("geojson")
                .and_then(Value::as_str)
                .ok_or_else(response_error)?;
            *v = serde_json::from_str(raw).map_err(|_| response_error())?
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
                *v = serde_json::from_str(raw).map_err(|_| response_error())?
            }
        }
    }
    Ok(())
}

/// Connection options. Plaintext `http://` is accepted only for loopback hosts unless
/// `allow_insecure` is set (or `GL_ALLOW_INSECURE_TRANSPORT=true`), because the bearer
/// token would otherwise cross the network unencrypted.
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub timeout: Duration,
    pub allow_insecure: bool,
    /// Extra PEM CA bundle trusted for `https://` endpoints (private PKI).
    pub ca_pem: Option<Vec<u8>>,
}
impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            allow_insecure: false,
            ca_pem: None,
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
    pub async fn connect(endpoint: impl Into<String>, token: &str) -> Result<Self, Error> {
        Self::connect_with(endpoint, token, ConnectOptions::default()).await
    }
    pub async fn connect_with_timeout(
        endpoint: impl Into<String>,
        token: &str,
        timeout: Duration,
    ) -> Result<Self, Error> {
        Self::connect_with(
            endpoint,
            token,
            ConnectOptions {
                timeout,
                ..ConnectOptions::default()
            },
        )
        .await
    }
    pub async fn connect_with(
        endpoint: impl Into<String>,
        token: &str,
        options: ConnectOptions,
    ) -> Result<Self, Error> {
        let timeout = options.timeout;
        let endpoint = endpoint.into();
        let mut e = Endpoint::from_shared(endpoint)
            .map_err(|_| Error::invalid("endpoint must be http(s)://host:port"))?;
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
        if token.is_empty() || timeout.is_zero() {
            return Err(Error::invalid("token and positive timeout required"));
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
            let mut tls = tonic::transport::ClientTlsConfig::new().with_native_roots();
            if let Some(pem) = options.ca_pem {
                tls = tls.ca_certificate(tonic::transport::Certificate::from_pem(pem));
            }
            e = e
                .tls_config(tls)
                .map_err(|_| Error::invalid("TLS configuration failed"))?
        }
        let auth = Authentication {
            value: format!("Bearer {token}")
                .parse()
                .map_err(|_| Error::invalid("invalid token"))?,
            timeout,
        };
        let channel = e.connect().await.map_err(|_| Error {
            code: "unavailable".into(),
            message: "could not connect to GeoLedger".into(),
            request_id: None,
            conflicts: None,
            uncertain: false,
        })?;
        Ok(Self {
            rpc: pb::geo_ledger_client::GeoLedgerClient::with_interceptor(channel, auth)
                .max_decoding_message_size(4 * 1024 * 1024)
                .max_encoding_message_size(4 * 1024 * 1024),
        })
    }
    /// Advanced business-JSON entry point, also used by the CLI. GeoJSON is a plain object.
    pub async fn execute(&self, operation: &str, mut value: Value) -> Result<Value, Error> {
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
            "publish" => serde_json::to_value(
                rpc.publish(serde_json::from_value::<pb::PublishRequest>(value)?)
                    .await?
                    .into_inner(),
            )?,
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
    pub async fn projects(&self, page: Page) -> Result<Vec<Project>, Error> {
        decode(
            self.execute("list_projects", serde_json::to_value(page)?)
                .await?["projects"]
                .take(),
        )
    }
    pub async fn set_member(&self, project: &str, subject: &str, role: &str) -> Result<(), Error> {
        self.execute(
            "set_member",
            json!({"project":project,"subject":subject,"role":role}),
        )
        .await?;
        Ok(())
    }
    /// Active members ordered by subject.
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
    /// Permanently hide a project. `confirm_name` must equal the project name.
    pub async fn delete_project(&self, project: &str, confirm_name: &str) -> Result<(), Error> {
        self.execute(
            "delete_project",
            json!({"project":project,"confirm_name":confirm_name}),
        )
        .await?;
        Ok(())
    }
    pub async fn create_dataset(&self, project: &str, name: &str) -> Result<Dataset, Error> {
        decode(
            self.execute("create_dataset", json!({"project":project,"name":name}))
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
    pub async fn history(
        &self,
        project: &str,
        after: i64,
        limit: Option<i64>,
    ) -> Result<Vec<Commit>, Error> {
        decode(
            self.execute(
                "history",
                json!({"project":project,"after":after,"limit":limit}),
            )
            .await?["commits"]
                .take(),
        )
    }
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
    pub async fn save_batch(&mut self, edits: &[Edit]) -> Result<SaveResult, Error> {
        self.editable()?;
        let mut input = self.input();
        input["edits"] = serde_json::to_value(edits)?;
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
    pub async fn diff(&self, page: Page) -> Result<Diff, Error> {
        decode(self.client.execute("diff",json!({"project":self.project,"workspace":self.info.id,"after":page.after,"limit":page.limit})).await?)
    }
    pub async fn conflicts(&self, page: Page) -> Result<Conflicts, Error> {
        decode(self.client.execute("conflicts",json!({"project":self.project,"workspace":self.info.id,"after":page.after,"limit":page.limit})).await?)
    }
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
                if !e.uncertain
                    && (!was_pending
                        || !matches!(
                            e.code.as_str(),
                            "unauthenticated" | "permission_denied" | "not_found"
                        ))
                {
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
    fn response_conversion_preserves_opaque_properties_and_numbers() -> Result<(), Error> {
        let original = json!({"type":"Feature","id":"one","properties":{"nested":{"geojson":"ordinary text"},"detail_json":"plain","exact":18446744073709551615u64},"geometry":null});
        let mut reply = json!({"features":[{"geojson":original.to_string()}],"revision":1,"workspace_version":null,"next_after":null});
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
