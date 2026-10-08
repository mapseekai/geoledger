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
mod session;
use geoledger_core::{Cell, Record, merge::merge_record};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use session::Row;
pub mod repository;
use repository::{RepositoryTransaction, StorageBackend};

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use uuid::Uuid;

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const FORMAT_VERSION: i32 = 4;
pub type Result<T> = std::result::Result<T, Error>;
pub use errors::Error;
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
}
#[derive(Clone)]
pub enum Storage {
    Sqlite(std::path::PathBuf),
    Postgis(String),
}
struct Transaction {
    inner: Box<dyn RepositoryTransaction>,
    cache: std::collections::HashMap<String, Value>,
}
impl std::ops::Deref for Transaction {
    type Target = dyn RepositoryTransaction;
    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}
impl std::ops::DerefMut for Transaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}
impl Transaction {
    fn geometry_cache(&mut self) -> &mut std::collections::HashMap<String, Value> {
        &mut self.cache
    }
    fn commit(self) -> Result<()> {
        self.inner.commit()
    }
}
impl Application {
    pub fn new(storage: Storage) -> Self {
        Self::with_backend(std::sync::Arc::new(session::SqlStorage {
            storage,
            pool: Default::default(),
        }))
    }
    pub fn with_backend(storage: std::sync::Arc<dyn StorageBackend>) -> Self {
        Self {
            storage,
            timeout: Duration::from_secs(30),
        }
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
        );
        let mut t = Transaction {
            inner: self.storage.begin(read_only, self.timeout)?,
            cache: Default::default(),
        };
        let result = match command {
            Command::CreateProject(r) => create_project(&mut t, subject, r),
            Command::ListProjects(r) => list_projects(&mut t, subject, r),
            Command::GetProject(r) => get_project(&mut t, subject, r),
            Command::SetMember(r) => set_member(&mut t, subject, r),
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
    CreateDataset(NamedProject),
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
    if bytes.len() > MAX_BYTES {
        return Err(Error::new(413, "request too large"));
    }
    codec::parse(bytes)
}
