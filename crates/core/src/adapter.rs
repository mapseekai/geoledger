//! Adapters own I/O, schema codecs, locking and transaction semantics. The core
//! only knows records, object IDs and stable dataset keys.
use crate::{Binding, Cell, ObjectId, Record, Result, Schema};
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
    fn column_ids(&mut self, _binding: &Binding) -> Result<BTreeMap<i16, String>> {
        Ok(BTreeMap::new())
    }
    fn current_schema(&mut self, binding: &Binding) -> Result<Schema> {
        self.inspect(&binding.schema_name, &binding.table_name)
    }
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
    /// Keyset pagination; implementations should avoid materializing the entire dirty set.
    fn dirty_keys_page(
        &mut self,
        dataset: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let mut keys = self.dirty_keys(dataset)?;
        keys.sort();
        keys.dedup();
        Ok(keys
            .into_iter()
            .filter(|k| after.is_none_or(|a| k.as_str() > a))
            .take(limit)
            .collect())
    }
    fn projection_defaults(
        &mut self,
        _from: &Schema,
        _to: &Schema,
    ) -> Result<BTreeMap<String, Cell>> {
        Ok(BTreeMap::new())
    }
    fn normalize_many(&mut self, binding: &Binding, records: &[Record]) -> Result<Vec<Record>> {
        records.iter().map(|r| self.normalize(binding, r)).collect()
    }
    fn read(&mut self, binding: &Binding, key: &str) -> Result<Option<Record>>;
    fn read_many(
        &mut self,
        binding: &Binding,
        keys: &[String],
    ) -> Result<BTreeMap<String, Record>> {
        let mut records = BTreeMap::new();
        for key in keys {
            if let Some(record) = self.read(binding, key)? {
                records.insert(key.clone(), record);
            }
        }
        Ok(records)
    }
    /// Return a nonempty prefix of the requested keys, including missing rows.
    /// At most the final row may take the batch above the payload-byte budget.
    fn read_many_bounded(
        &mut self,
        binding: &Binding,
        keys: &[String],
        bytes: usize,
    ) -> Result<Vec<(String, Option<Record>)>> {
        let mut batch = Vec::new();
        let mut size = 0;
        for key in keys {
            let record = self.read(binding, key)?;
            size += key.len() + record.as_ref().map_or(0, Record::payload_bytes);
            batch.push((key.clone(), record));
            if size >= bytes {
                break;
            }
        }
        Ok(batch)
    }
    fn write_many(&mut self, binding: &Binding, values: &[(&str, Option<&Record>)]) -> Result<()> {
        for (key, value) in values {
            self.write(binding, key, *value)?;
        }
        Ok(())
    }
    fn edit_schema(
        &mut self,
        _binding: &Binding,
        _edit: &crate::schema::SchemaEdit,
    ) -> Result<Schema> {
        Err(crate::Error::Unsupported(
            "schema editing for this provider".into(),
        ))
    }
    /// Empty the table with triggers enabled, then install a historical definition.
    fn replace_schema(&mut self, _binding: &Binding, _target: &Schema) -> Result<()> {
        Err(crate::Error::Unsupported(
            "schema checkout for this provider".into(),
        ))
    }
    fn normalize(&mut self, binding: &Binding, record: &Record) -> Result<Record>;
    fn write(&mut self, binding: &Binding, key: &str, value: Option<&Record>) -> Result<()>;
    fn clear_dirty(&mut self) -> Result<()>;
    fn mark(&mut self, operation: &str, head: &ObjectId) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
    // Dropping an uncommitted transaction MUST roll it back.
}
