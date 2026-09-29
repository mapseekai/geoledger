use crate::{Error, ObjectId, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current immutable object, schema and PostGIS tracking format.
pub const FORMAT_VERSION: u32 = 3;

/// Current local repository state; conflict records live exclusively in the index.
pub const STATE_VERSION: u32 = 5;

/// A record key may identify a feature today and a tile/chunk in a future adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub key: String,
    pub fields: BTreeMap<String, Cell>,
}

impl Record {
    /// Approximate owned payload bytes for I/O batching, not allocator accounting.
    pub fn payload_bytes(&self) -> usize {
        self.key.len()
            + self
                .fields
                .iter()
                .map(|(name, value)| {
                    name.len()
                        + match value {
                            Cell::Text(v) | Cell::Geometry(v) => v.len(),
                            Cell::Blob(_) => 64,
                            Cell::Null => 0,
                        }
                })
                .sum::<usize>()
    }
}

/// Scalar text is interpreted using the field's explicit codec, never as f64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Cell {
    Null,
    Text(String),
    Geometry(String), // Lowercase hex, XDR EWKB. SRID and Z/M are preserved.
    Blob(ObjectId),   // Extension point; not accepted by the PostGIS v1 adapter.
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetKind {
    Vector,
    Table,
    Raster,
    PointCloud,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub logical_type: String,
    pub codec: String,
    pub nullable: bool,
    pub geometry: bool,
    /// Native details belong to adapters, not to the merge engine.
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schema {
    pub version: u32,
    pub kind: DatasetKind,
    pub primary_key: String,
    pub fields: Vec<Field>,
    pub metadata: BTreeMap<String, String>,
}

impl Schema {
    pub fn validate(&self, record: &Record) -> Result<()> {
        if record.fields.len() != self.fields.len() {
            return Err(Error::Invalid("record fields do not match schema".into()));
        }
        if record.fields.get(&self.primary_key) != Some(&Cell::Text(record.key.clone())) {
            return Err(Error::Invalid(
                "record key must equal its primary-key field".into(),
            ));
        }
        for field in &self.fields {
            let cell = record
                .fields
                .get(&field.name)
                .ok_or_else(|| Error::Invalid(format!("missing field {}", field.name)))?;
            match cell {
                Cell::Null if field.nullable => {}
                Cell::Text(_) if !field.geometry => {}
                Cell::Geometry(hex) if field.geometry => {
                    if !hex.starts_with("00")
                        || hex.len() < 18
                        || hex.len() % 2 != 0
                        || !hex
                            .bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                    {
                        return Err(Error::Invalid(
                            "geometry must be lowercase XDR EWKB hex".into(),
                        ));
                    }
                }
                _ => return Err(Error::Invalid(format!("invalid value for {}", field.name))),
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dataset {
    pub schema: ObjectId,
    pub root: Option<ObjectId>,
    pub records: u64,
}

pub type Snapshot = BTreeMap<String, Dataset>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub version: u32,
    pub parents: Vec<ObjectId>,
    pub root: ObjectId,
    pub author: String,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub provider: String,
    pub schema_name: String,
    pub table_name: String,
    pub schema: Schema,
    /// Working-copy column positions, not part of the historical schema encoding.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub column_ids: BTreeMap<i16, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryState {
    pub version: u32,
    pub repository_id: String,
    pub branch: String,
    pub branches: BTreeMap<String, ObjectId>,
    pub bindings: BTreeMap<String, Binding>,
    pub merging: Option<MergeState>,
}

impl RepositoryState {
    pub fn head(&self) -> Result<&ObjectId> {
        self.branches
            .get(&self.branch)
            .ok_or_else(|| Error::Storage("HEAD points to a missing branch".into()))
    }
    pub fn resolve(&self, reference: &str) -> Result<ObjectId> {
        if reference == "HEAD" {
            return self.head().cloned();
        }
        if let Some(id) = self.branches.get(reference) {
            return Ok(id.clone());
        }
        ObjectId::parse(reference)
            .map_err(|_| Error::NotFound(format!("branch or full commit ID {reference}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub dataset: String,
    pub key: String,
    pub before: Option<Record>,
    pub after: Option<Record>,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub dataset: String,
    pub key: String,
    pub base: Option<Record>,
    pub ours: Option<Record>,
    pub theirs: Option<Record>,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeState {
    pub base: ObjectId,
    pub ours: ObjectId,
    pub theirs: ObjectId,
    pub parents: Vec<ObjectId>,
    pub snapshot: Snapshot,
    pub author: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOperation {
    pub id: String,
    pub before_head: ObjectId,
    pub after: RepositoryState,
}

pub fn validate_branch(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || name == "HEAD"
        || name.ends_with(".lock")
        || name
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !"-_/ .".contains(c))
        || name.contains(' ')
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(Error::Invalid("invalid branch name".into()));
    }
    Ok(())
}
