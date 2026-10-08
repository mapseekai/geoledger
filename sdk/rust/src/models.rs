//! Transport-independent GeoLedger business models.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    pub backend: String,
    pub format_version: u32,
    pub max_request_bytes: u32,
    pub max_feature_bytes: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    #[serde(rename = "project")]
    pub id: String,
    pub name: String,
    pub head: i64,
    pub role: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dataset {
    #[serde(rename = "dataset")]
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    #[serde(rename = "workspace")]
    pub id: String,
    pub base_revision: i64,
    pub version: i64,
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveResult {
    pub version: i64,
    pub changes: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscardResult {
    pub version: i64,
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeaturePage {
    pub features: Vec<Value>,
    pub revision: i64,
    pub workspace_version: Option<i64>,
    pub next_after: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub cursor: String,
    pub dataset: String,
    pub feature_id: String,
    pub base: Option<Value>,
    pub draft: Option<Value>,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    pub base_revision: i64,
    pub version: i64,
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub cursor: String,
    pub dataset: String,
    pub feature_id: String,
    pub fields: Vec<String>,
    pub base: Option<Value>,
    pub current: Option<Value>,
    pub draft: Option<Value>,
    pub reason: String,
    pub resolved_against_revision: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Conflicts {
    pub head: i64,
    pub version: i64,
    pub total: u64,
    pub next_after: Option<String>,
    pub truncated: bool,
    pub conflicts: Vec<Conflict>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    pub revision: i64,
    pub subject: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommitChanges {
    pub revision: i64,
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub subject: String,
    pub action: String,
    #[serde(rename = "detail_json")]
    pub detail: Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditPage {
    pub events: Vec<AuditEvent>,
    pub next_after: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicationResult {
    pub revision: i64,
    pub workspace: String,
    pub version: i64,
    pub status: String,
    pub changes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolutionResult {
    pub head: i64,
    pub version: i64,
    pub remaining_conflicts: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RebaseResult {
    pub base_revision: i64,
    pub version: i64,
    pub changes: u64,
}

/// GeoJSON is ordinary JSON. `None` is an explicit deletion.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub dataset: String,
    pub feature_id: String,
    #[serde(deserialize_with = "required_feature")]
    pub feature: Option<Value>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Page {
    pub after: String,
    pub limit: Option<i64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeatureQuery {
    pub workspace: Option<String>,
    pub revision: Option<i64>,
    pub feature_id: Option<String>,
    pub bbox: Vec<f64>,
    pub after: String,
    pub limit: Option<i64>,
}
/// Serializable publication intent. Preserve this value for recovery across processes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    pub project: String,
    pub workspace: String,
    pub expected_workspace_version: i64,
    pub request_id: String,
    pub message: String,
}

fn required_feature<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Option::<Value>::deserialize(deserializer)
}
