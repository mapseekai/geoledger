#![forbid(unsafe_code)]

mod workspace;
use workspace::*;
mod feature;
mod geometry;
pub use feature::Feature;
use feature::*;
mod query;
use query::*;
mod publication;
use publication::*;
mod codec;
mod errors;
#[cfg(any(test, feature = "fuzzing"))]
#[doc(hidden)]
pub mod fuzzing;
mod session;
use geoledger_core::{Cell, Record, merge::merge_record};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use session::Row;
pub mod repository;
use repository::{RepositoryTransaction, StorageBackend};
pub use session::pgtls::{PgTlsSummary, SslMode, summarize as postgres_tls_summary};

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use uuid::Uuid;

pub const FORMAT_VERSION: i32 = 7;
pub type Result<T> = std::result::Result<T, Error>;
pub use errors::Error;
pub use session::portable::{DataSummary, EXPORT_VERSION, TableSummary};
fn bad() -> Error {
    Error::new(400, "invalid request")
}
fn missing() -> Error {
    Error::new(404, "resource unavailable")
}
fn stale() -> Error {
    Error::new(409, "stale version or workspace not open")
}
fn decode<T: DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(Error::invalid_json)
}
fn text(s: &str, max: usize) -> Result<()> {
    if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
        Err(bad())
    } else {
        Ok(())
    }
}
fn id(s: &str) -> Result<()> {
    Uuid::parse_str(s).map(|_| ()).map_err(|_| bad())
}

/// Independent central application. Each operation owns one connection and transaction.
/// Configuration deliberately has no Debug implementation: it contains a DSN.
#[derive(Clone)]
pub struct Application {
    storage: std::sync::Arc<dyn StorageBackend>,
    timeout: Duration,
    policy: std::sync::Arc<Policy>,
}
/// Platform-level authorization that is not tied to project membership.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    /// Subjects that may administer any project (members, archive, delete)
    /// and create projects regardless of the creation policy or quota.
    /// Administrators gain no data access until they add themselves as members.
    pub admins: BTreeSet<String>,
    /// Only administrators may create projects.
    pub admin_only_project_creation: bool,
    /// Maximum projects (not deleted) a non-administrator may own.
    pub max_owned_projects: Option<u32>,
}
#[derive(Clone)]
pub enum Storage {
    Sqlite(std::path::PathBuf),
    Postgis(String),
}
/// Tunable limits for the built-in SQL backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageOptions {
    /// Maximum concurrent PostgreSQL connections.
    pub pool_size: usize,
    /// PostgreSQL `statement_timeout` for every session.
    pub statement_timeout: Duration,
    /// PostgreSQL `lock_timeout` for every session.
    pub lock_timeout: Duration,
}
impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            pool_size: 20,
            statement_timeout: Duration::from_secs(30),
            lock_timeout: Duration::from_secs(10),
        }
    }
}
/// Connection pool occupancy, exported as metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    pub capacity: usize,
    pub active: usize,
    pub idle: usize,
    /// Acquisitions that gave up because no connection became free before the deadline.
    pub wait_timeouts: u64,
}
type Transaction = Box<dyn RepositoryTransaction>;
impl Application {
    pub fn new(storage: Storage) -> Self {
        Self::with_options(storage, StorageOptions::default())
    }
    pub fn with_options(storage: Storage, options: StorageOptions) -> Self {
        Self::with_backend(std::sync::Arc::new(session::SqlStorage {
            storage,
            pool: std::sync::Arc::new(session::postgres::Pool::new(StorageOptions {
                pool_size: options.pool_size.max(1),
                ..options
            })),
        }))
    }
    pub fn pool_stats(&self) -> Option<PoolStats> {
        self.storage.pool_stats()
    }
    pub fn with_backend(storage: std::sync::Arc<dyn StorageBackend>) -> Self {
        Self {
            storage,
            timeout: Duration::from_secs(30),
            policy: Default::default(),
        }
    }
    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = std::sync::Arc::new(policy);
        self
    }
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
    /// Stream a logical export of the whole database (consistent snapshot; the
    /// server may keep running).
    pub fn export_data(&self, out: &mut dyn std::io::Write) -> Result<DataSummary> {
        self.storage.export(out, self.timeout)
    }
    /// Row counts and digests of the live database, comparable with an export.
    pub fn data_summary(&self) -> Result<DataSummary> {
        self.storage.export(&mut std::io::sink(), self.timeout)
    }
    /// Import an export into a new or empty database; all-or-nothing.
    pub fn import_data(&self, input: &mut dyn std::io::BufRead) -> Result<DataSummary> {
        self.storage.import(input, self.timeout)
    }
    /// Online consistent backup into a new file (SQLite).
    pub fn backup(&self, target: &std::path::Path) -> Result<()> {
        self.storage.backup(target, self.timeout)
    }
    /// Validate an export file's structure, digests and checksum without a database.
    pub fn read_export(input: &mut dyn std::io::BufRead) -> Result<DataSummary> {
        session::portable::read(input, |_, _, _| Ok(()))
    }
    /// Integrity and format check of a SQLite database file, e.g. a backup.
    pub fn verify_sqlite_file(path: &std::path::Path, timeout: Duration) -> Result<()> {
        session::verify_sqlite_file(path, timeout)
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn backend(&self) -> &'static str {
        self.storage.name()
    }
    pub fn migrate(&self) -> Result<()> {
        self.storage.initialize(self.timeout)
    }
    pub fn check_schema(&self) -> Result<()> {
        self.storage.health(self.timeout)
    }
    pub fn execute(&self, subject: &str, operation: &str, input: Value) -> Result<Value> {
        self.execute_encoded(subject, operation, input)
            .map(|(value, _)| value)
    }
    fn execute_encoded(
        &self,
        subject: &str,
        operation: &str,
        input: Value,
    ) -> Result<(Value, Vec<u8>)> {
        text(subject, 128)?;
        let input = codec::parse(&codec::encode(&input)?)?;
        let command: Command = decode(json!({"operation":operation,"input":input}))?;
        let read_only = matches!(
            operation,
            "list_projects"
                | "get_project"
                | "list_datasets"
                | "list_workspaces"
                | "get_workspace"
                | "features"
                | "diff"
                | "conflicts"
                | "history"
                | "audit"
                | "commit"
                | "list_members"
        );
        let policy = self.policy.as_ref();
        let mut t = self.storage.begin(read_only, self.timeout)?;
        let result = match command {
            Command::CreateProject(r) => create_project(&mut t, subject, policy, r),
            Command::ListProjects(r) => list_projects(&mut t, subject, r),
            Command::GetProject(r) => get_project(&mut t, subject, policy, r),
            Command::SetMember(r) => set_member(&mut t, subject, policy, r),
            Command::RemoveMember(r) => remove_member(&mut t, subject, policy, r),
            Command::ListMembers(r) => list_members(&mut t, subject, policy, r),
            Command::ArchiveProject(r) => archive_project(&mut t, subject, policy, r),
            Command::DeleteProject(r) => delete_project(&mut t, subject, policy, r),
            Command::RenameProject(r) => rename_project(&mut t, subject, policy, r),
            Command::RenameDataset(r) => rename_dataset(&mut t, subject, r),
            Command::DeleteDataset(r) => delete_dataset(&mut t, subject, policy, r),
            Command::CreateDataset(r) => create_dataset(&mut t, subject, r),
            Command::ListDatasets(r) => list_datasets(&mut t, subject, r),
            Command::CreateWorkspace(r) => create_workspace(&mut t, subject, r),
            Command::ListWorkspaces(r) => list_workspaces(&mut t, subject, r),
            Command::GetWorkspace(r) => get_workspace(&mut t, subject, r),
            Command::Save(r) => save(&mut t, subject, r),
            Command::Discard(r) => discard(&mut t, subject, r),
            Command::Features(r) => features(&mut t, subject, r),
            Command::Diff(r) => diff(&mut t, subject, r),
            Command::Conflicts(r) => list_conflicts(&mut t, subject, r),
            Command::Resolve(r) => resolve(&mut t, subject, r),
            Command::History(r) => history(&mut t, subject, r),
            Command::Audit(r) => audit_events(&mut t, subject, r),
            Command::Commit(r) => commit_detail(&mut t, subject, r),
            Command::Publish(r) => publish(&mut t, subject, r),
            Command::Rebase(r) => rebase(&mut t, subject, r),
            Command::Restore(r) => restore(&mut t, subject, r),
        }?;
        let encoded = codec::encode(&result)?;
        t.commit()?;
        Ok((result, encoded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codec_and_field_merge() -> Result<()> {
        let b = Stored {
            properties: serde_json::from_value(
                json!({"geometry":null,"a":1,"b":true,"~/x":[1,false]}),
            )
            .map_err(|_| bad())?,
            geometry: Some("abcd".into()),
        };
        assert_eq!(Stored::from_record(b.record("id"))?, b);
        let mut ours = b.clone();
        ours.properties.insert("a".into(), json!(2));
        let mut theirs = b.clone();
        theirs.properties.remove("geometry");
        theirs.properties.insert("b".into(), json!(false));
        let merged = merge_record(
            Some(&b.record("id")),
            Some(&ours.record("id")),
            Some(&theirs.record("id")),
        )
        .map_err(|_| bad())?
        .ok_or_else(bad)?;
        let merged = Stored::from_record(merged)?;
        assert_eq!(merged.properties["a"], 2);
        assert_eq!(merged.properties["b"], false);
        assert!(!merged.properties.contains_key("geometry"));
        theirs.properties.insert("a".into(), json!(3));
        assert_eq!(
            merge_record(
                Some(&b.record("id")),
                Some(&ours.record("id")),
                Some(&theirs.record("id"))
            )
            .err(),
            Some(vec!["/properties/a".into()])
        );
        assert!(merge_record(Some(&b.record("id")), None, Some(&theirs.record("id"))).is_err());
        Ok(())
    }
    #[test]
    fn validates_geometry_shape_and_identity() {
        assert!(decode::<Edit>(json!({"dataset":"d","feature_id":"f"})).is_err());
        assert!(decode::<Edit>(json!({"dataset":"d","feature_id":"f","feature":null})).is_ok());
        assert!(geometry_shape(&json!({"type":"Point","coordinates":[1,2]})).is_ok());
        for g in [
            json!({"type":"Point","coordinates":[1,2,3,4]}),
            json!({"type":"Point","coordinates":["NaN",2]}),
            json!({"type":"Point","coordinates":[1,2],"crs":{}}),
        ] {
            assert!(geometry_shape(&g).is_err());
        }
        assert!(decode::<Publish>(json!({"project":"p","workspace":"w","expected_workspace_version":0,"request_id":Uuid::new_v4(),"message":"x","author":"forged"})).is_err());
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", content = "input", rename_all = "snake_case")]
enum Command {
    CreateProject(Name),
    ListProjects(Page),
    GetProject(Project),
    SetMember(Member),
    RemoveMember(MemberRef),
    ListMembers(ProjectPage),
    ArchiveProject(Archive),
    DeleteProject(DeleteProject),
    RenameProject(NamedProject),
    RenameDataset(RenameDataset),
    DeleteDataset(DeleteDataset),
    CreateDataset(CreateDataset),
    ListDatasets(ProjectPage),
    CreateWorkspace(Project),
    ListWorkspaces(ProjectPage),
    GetWorkspace(Workspace),
    Save(Save),
    Discard(Version),
    Features(Features),
    Diff(Diff),
    Conflicts(Diff),
    Resolve(Rebase),
    History(History),
    Audit(Audit),
    Commit(Commit),
    Publish(Publish),
    Rebase(Rebase),
    Restore(Restore),
}

pub fn parse_json(bytes: &[u8]) -> Result<Value> {
    codec::parse(bytes)
}
