use crate::{Error, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ObjectId(String);

impl ObjectId {
    pub fn parse(s: &str) -> Result<Self> {
        if s.len() != 64
            || !s
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(Error::Invalid(
                "object ID must contain 64 lowercase hex characters".into(),
            ));
        }
        Ok(Self(s.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl TryFrom<String> for ObjectId {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
}
impl From<ObjectId> for String {
    fn from(value: ObjectId) -> Self {
        value.0
    }
}

pub fn digest(kind: &str, bytes: &[u8]) -> ObjectId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"geoledger\0object-v3\0");
    hasher.update(&(kind.len() as u64).to_be_bytes());
    hasher.update(kind.as_bytes());
    hasher.update(bytes);
    ObjectId(hasher.finalize().to_hex().to_string())
}

pub trait ObjectStore {
    fn put(&self, kind: &str, bytes: &[u8]) -> Result<ObjectId>;
    fn get(&self, id: &ObjectId, expected_kind: &str) -> Result<Vec<u8>>;
}

/// JSON maps are sorted by serde_json's default map implementation; schema maps
/// are BTreeMaps. The codec is explicitly versioned, not advertised as RFC 8785.
pub fn save<T: Serialize>(store: &dyn ObjectStore, kind: &str, value: &T) -> Result<ObjectId> {
    store.put(kind, &serde_json::to_vec(value)?)
}
pub fn load<T: DeserializeOwned>(store: &dyn ObjectStore, kind: &str, id: &ObjectId) -> Result<T> {
    serde_json::from_slice(&store.get(id, kind)?)
        .map_err(|e| Error::storage_source(format!("decode {kind} object {id}"), e))
}

#[derive(Default)]
pub struct MemoryStore(RefCell<BTreeMap<ObjectId, (String, Vec<u8>)>>);
impl MemoryStore {
    pub fn object_count(&self) -> usize {
        self.0.borrow().len()
    }
}
impl ObjectStore for MemoryStore {
    fn put(&self, kind: &str, bytes: &[u8]) -> Result<ObjectId> {
        let id = digest(kind, bytes);
        self.0
            .borrow_mut()
            .entry(id.clone())
            .or_insert((kind.into(), bytes.into()));
        Ok(id)
    }
    fn get(&self, id: &ObjectId, kind: &str) -> Result<Vec<u8>> {
        let entries = self.0.borrow();
        let (actual, bytes) = entries
            .get(id)
            .ok_or_else(|| Error::NotFound(id.to_string()))?;
        if actual != kind || digest(actual, bytes) != *id {
            return Err(Error::Storage("object type or checksum mismatch".into()));
        }
        Ok(bytes.clone())
    }
}
