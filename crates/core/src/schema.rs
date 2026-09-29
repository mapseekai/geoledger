//! Stable GeoLedger column identities shared by schema history and field evolution.
use crate::{
    Cell, FORMAT_VERSION, Field, ObjectId, ObjectStore, Record, Result, Schema, load, save,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const COLUMN_ID: &str = "geoledger.column-id";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SchemaEdit {
    Add {
        name: String,
        data_type: String,
    },
    Drop {
        name: String,
        #[serde(default)]
        discard: bool,
    },
    Rename {
        name: String,
        new_name: String,
    },
    AlterType {
        name: String,
        data_type: String,
    },
}

pub fn column_id(field: &Field) -> String {
    field
        .metadata
        .get(COLUMN_ID)
        .cloned()
        .unwrap_or_else(|| format!("gl:field:{}", field.name))
}

pub fn with_identities(mut schema: Schema) -> Schema {
    for field in &mut schema.fields {
        let id = column_id(field);
        field.metadata.insert(COLUMN_ID.into(), id);
    }
    schema.version = FORMAT_VERSION;
    schema.fields.sort_by(|a, b| a.name.cmp(&b.name));
    schema
}

pub fn store(store: &dyn ObjectStore, schema: &Schema) -> Result<ObjectId> {
    validate_format(schema)?;
    save(store, "schema/v3", schema)
}

pub fn read(store: &dyn ObjectStore, id: &ObjectId) -> Result<Schema> {
    let schema: Schema = load(store, "schema/v3", id)?;
    validate_format(&schema)?;
    Ok(schema)
}

pub fn validate_format(schema: &Schema) -> Result<()> {
    if schema.version != FORMAT_VERSION {
        return Err(crate::Error::Unsupported("GeoLedger schema format".into()));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for field in &schema.fields {
        let id = field
            .metadata
            .get(COLUMN_ID)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| crate::Error::Storage("schema field is missing identity".into()))?;
        if !names.insert(&field.name) || !ids.insert(id) {
            return Err(crate::Error::Storage(
                "duplicate schema field name or identity".into(),
            ));
        }
    }
    Ok(())
}

/// Compare physical definitions, excluding versioning IDs and physical column order.
pub fn equivalent(a: &Schema, b: &Schema) -> bool {
    fn normalized(s: &Schema) -> Schema {
        let mut s = s.clone();
        s.version = FORMAT_VERSION;
        for field in &mut s.fields {
            field.metadata.remove(COLUMN_ID);
        }
        s.fields.sort_by(|a, b| a.name.cmp(&b.name));
        s
    }
    normalized(a) == normalized(b)
}

/// Precomputed column mapping; missing fields and explicit NULL remain distinct
/// until adapter-provided defaults have been applied.
pub struct Projection {
    fields: Vec<(String, Option<String>, Cell)>,
}
impl Projection {
    pub fn new(from: &Schema, to: &Schema, defaults: &BTreeMap<String, Cell>) -> Self {
        let by_id: BTreeMap<_, _> = from
            .fields
            .iter()
            .map(|f| (column_id(f), &f.name))
            .collect();
        Self {
            fields: to
                .fields
                .iter()
                .map(|f| {
                    (
                        f.name.clone(),
                        by_id.get(&column_id(f)).map(|n| (*n).clone()),
                        defaults.get(&f.name).cloned().unwrap_or(Cell::Null),
                    )
                })
                .collect(),
        }
    }
    pub fn apply(&self, record: &Record) -> Record {
        Record {
            key: record.key.clone(),
            fields: self
                .fields
                .iter()
                .map(|(name, source, default)| {
                    let value = source
                        .as_ref()
                        .and_then(|n| record.fields.get(n))
                        .unwrap_or(default);
                    (name.clone(), value.clone())
                })
                .collect(),
        }
    }
}
pub fn project(record: &Record, from: &Schema, to: &Schema) -> Record {
    Projection::new(from, to, &BTreeMap::new()).apply(record)
}

pub fn merge(base: &Schema, ours: &Schema, theirs: &Schema) -> Result<Schema> {
    use crate::Error;
    if base.primary_key != ours.primary_key || base.primary_key != theirs.primary_key {
        return Err(Error::Conflict("primary-key schema conflict".into()));
    }
    let maps = [base, ours, theirs].map(|s| {
        s.fields
            .iter()
            .map(|f| (column_id(f), f))
            .collect::<BTreeMap<_, _>>()
    });
    let keys = maps
        .iter()
        .flat_map(|m| m.keys())
        .collect::<std::collections::BTreeSet<_>>();
    let mut fields = Vec::new();
    for key in keys {
        let (b, o, t) = (maps[0].get(key), maps[1].get(key), maps[2].get(key));
        let selected = if o == t || t == b {
            o
        } else if o == b {
            t
        } else {
            return Err(Error::Conflict(format!(
                "schema conflict for column {key}; align the schemas before merging"
            )));
        };
        if let Some(f) = selected {
            fields.push((**f).clone());
        }
    }
    let mut names = std::collections::BTreeSet::new();
    if fields.iter().any(|f| !names.insert(f.name.clone())) {
        return Err(Error::Conflict("schema column name collision".into()));
    }
    let mut metadata = BTreeMap::new();
    for key in base
        .metadata
        .keys()
        .chain(ours.metadata.keys())
        .chain(theirs.metadata.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let (b, o, t) = (
            base.metadata.get(key),
            ours.metadata.get(key),
            theirs.metadata.get(key),
        );
        let selected = if o == t || t == b {
            o
        } else if o == b {
            t
        } else {
            return Err(Error::Conflict(format!("schema metadata conflict: {key}")));
        };
        if let Some(value) = selected {
            metadata.insert(key.clone(), value.clone());
        }
    }
    fields.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Schema {
        version: FORMAT_VERSION,
        kind: if fields.iter().any(|f| f.geometry) {
            crate::DatasetKind::Vector
        } else {
            crate::DatasetKind::Table
        },
        primary_key: ours.primary_key.clone(),
        fields,
        metadata,
    })
}
