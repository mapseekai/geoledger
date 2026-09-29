//! Versioned column identity. V1 columns receive deterministic legacy identities;
//! V2 preserves them through rename and assigns fresh identities to new columns.
use crate::{Cell, Field, ObjectId, ObjectStore, Record, Result, Schema, load, save};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Persisted schema key: retain it across the GeoLedger product rename.
pub const COLUMN_ID: &str = "spatial-version.column-id";

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
        .unwrap_or_else(|| format!("legacy:{}", field.name))
}

pub fn with_identities(mut schema: Schema) -> Schema {
    for field in &mut schema.fields {
        let id = column_id(field);
        field.metadata.insert(COLUMN_ID.into(), id);
    }
    schema.version = 2;
    schema.fields.sort_by(|a, b| a.name.cmp(&b.name));
    schema
}

pub fn store(store: &dyn ObjectStore, schema: &Schema) -> Result<ObjectId> {
    save(
        store,
        if schema.version == 2 {
            "schema/v2"
        } else {
            "schema/v1"
        },
        schema,
    )
}

pub fn read(store: &dyn ObjectStore, id: &ObjectId) -> Result<Schema> {
    let (schema, version): (Schema, u32) =
        load(store, "schema/v1", id)
            .map(|schema| (schema, 1))
            .or_else(|_| load(store, "schema/v2", id).map(|schema| (schema, 2)))?;
    if schema.version != version {
        return Err(crate::Error::Unsupported("schema format".into()));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for f in &schema.fields {
        if !names.insert(&f.name) || !ids.insert(column_id(f)) {
            return Err(crate::Error::Storage(
                "duplicate schema field name or identity".into(),
            ));
        }
        if schema.version == 2 && !f.metadata.contains_key(COLUMN_ID) {
            return Err(crate::Error::Storage("v2 field is missing identity".into()));
        }
    }
    Ok(schema)
}

/// Compare physical definitions, excluding versioning IDs and physical column order.
pub fn equivalent(a: &Schema, b: &Schema) -> bool {
    fn normalized(s: &Schema) -> Schema {
        let mut s = s.clone();
        s.version = 1;
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
        version: 2,
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
