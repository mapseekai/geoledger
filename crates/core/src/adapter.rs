//! Adapters own I/O, schema codecs, locking and transaction semantics. The core
//! only knows records, object IDs and stable dataset keys.
use crate::{Binding, ObjectId, Record, Result, Schema};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct DatabaseMarker {
    pub head: String,
    pub operation: Option<String>,
}

pub trait WorkingCopyProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn begin(
        &self,
        repository_id: &str,
        initial_head: &ObjectId,
        bindings: &BTreeMap<String, Binding>,
        extra_table: Option<(&str, &str)>,
    ) -> Result<Box<dyn WorkingCopyTransaction>>;
}

pub trait WorkingCopyTransaction {
    fn marker(&mut self) -> Result<DatabaseMarker>;
    fn inspect(&mut self, schema: &str, table: &str) -> Result<Schema>;
    fn register(&mut self, dataset: &str, binding: &Binding) -> Result<()>;
    fn verify(&mut self, dataset: &str, binding: &Binding) -> Result<()>;
    fn scan(
        &mut self,
        binding: &Binding,
        visit: &mut dyn FnMut(Record) -> Result<()>,
    ) -> Result<()>;
    fn dirty_keys(&mut self, dataset: &str) -> Result<Vec<String>>;
    fn read(&mut self, binding: &Binding, key: &str) -> Result<Option<Record>>;
    fn normalize(&mut self, binding: &Binding, record: &Record) -> Result<Record>;
    fn write(&mut self, binding: &Binding, key: &str, value: Option<&Record>) -> Result<()>;
    fn clear_dirty(&mut self) -> Result<()>;
    fn mark(&mut self, operation: &str, head: &ObjectId) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
    // Dropping an uncommitted transaction MUST roll it back.
}
