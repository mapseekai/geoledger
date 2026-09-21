use crate::{
    Change, Conflict, Dataset, Error, ObjectId, ObjectStore, Record, Result, Snapshot, load, save,
    tree,
};
use std::collections::BTreeSet;

pub fn record(
    store: &dyn ObjectStore,
    dataset: Option<&Dataset>,
    key: &str,
) -> Result<Option<Record>> {
    let id = tree::get(store, dataset.and_then(|d| d.root.as_ref()), key)?;
    id.as_ref()
        .map(|id| load(store, "record/v1", id))
        .transpose()
}
pub fn update(
    store: &dyn ObjectStore,
    dataset: &mut Dataset,
    key: &str,
    value: Option<&Record>,
) -> Result<()> {
    if value.is_some_and(|r| r.key != key) {
        return Err(Error::Invalid("record key mismatch".into()));
    }
    let old = tree::get(store, dataset.root.as_ref(), key)?;
    let new = value.map(|r| save(store, "record/v1", r)).transpose()?;
    if old.is_none() && new.is_some() {
        dataset.records += 1;
    }
    if old.is_some() && new.is_none() {
        dataset.records = dataset
            .records
            .checked_sub(1)
            .ok_or_else(|| Error::Storage("invalid record count".into()))?;
    }
    dataset.root = tree::set(store, dataset.root.as_ref(), key, new.as_ref())?;
    Ok(())
}
pub fn changed_fields(a: Option<&Record>, b: Option<&Record>) -> Vec<String> {
    let keys: BTreeSet<_> = a
        .into_iter()
        .flat_map(|r| r.fields.keys())
        .chain(b.into_iter().flat_map(|r| r.fields.keys()))
        .cloned()
        .collect();
    keys.into_iter()
        .filter(|k| a.and_then(|r| r.fields.get(k)) != b.and_then(|r| r.fields.get(k)))
        .collect()
}
pub fn diff(store: &dyn ObjectStore, before: &Snapshot, after: &Snapshot) -> Result<Vec<Change>> {
    let names: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    let mut result = Vec::new();
    for name in names {
        let a = before.get(name);
        let b = after.get(name);
        if let (Some(a), Some(b)) = (a, b)
            && a.schema != b.schema
        {
            return Err(Error::Unsupported(format!("schema changed: {name}")));
        }
        for d in tree::diff(
            store,
            a.and_then(|d| d.root.as_ref()),
            b.and_then(|d| d.root.as_ref()),
        )? {
            let before = d
                .before
                .as_ref()
                .map(|id| load::<Record>(store, "record/v1", id))
                .transpose()?;
            let after = d
                .after
                .as_ref()
                .map(|id| load::<Record>(store, "record/v1", id))
                .transpose()?;
            let fields = changed_fields(before.as_ref(), after.as_ref());
            result.push(Change {
                dataset: name.clone(),
                key: d.key,
                before,
                after,
                fields,
            });
        }
    }
    Ok(result)
}

/// Row identity is the PK. Geometry is an atomic field, never merged by vertex.
/// Identical concurrent changes are not conflicts.
pub fn merge_record(
    base: Option<&Record>,
    ours: Option<&Record>,
    theirs: Option<&Record>,
) -> std::result::Result<Option<Record>, Vec<String>> {
    if ours == theirs {
        return Ok(ours.cloned());
    }
    if ours == base {
        return Ok(theirs.cloned());
    }
    if theirs == base {
        return Ok(ours.cloned());
    }
    let (Some(base), Some(ours), Some(theirs)) = (base, ours, theirs) else {
        return Err(vec!["*".into()]);
    };
    let keys: BTreeSet<_> = base
        .fields
        .keys()
        .chain(ours.fields.keys())
        .chain(theirs.fields.keys())
        .collect();
    let mut fields = ours.fields.clone();
    let mut conflicts = Vec::new();
    for k in keys {
        let (b, o, t) = (base.fields.get(k), ours.fields.get(k), theirs.fields.get(k));
        let selected = if o == t || t == b {
            o
        } else if o == b {
            t
        } else {
            conflicts.push(k.clone());
            continue;
        };
        match selected {
            Some(v) => {
                fields.insert(k.clone(), v.clone());
            }
            None => {
                fields.remove(k);
            }
        }
    }
    if conflicts.is_empty() {
        Ok(Some(Record {
            key: ours.key.clone(),
            fields,
        }))
    } else {
        Err(conflicts)
    }
}

pub fn three_way(
    store: &dyn ObjectStore,
    base: &Snapshot,
    ours: &Snapshot,
    theirs: &Snapshot,
) -> Result<(Snapshot, Vec<Conflict>)> {
    let names: BTreeSet<_> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    let mut result = ours.clone();
    let mut conflicts = Vec::new();
    for name in names {
        let (b, o, t) = (base.get(name), ours.get(name), theirs.get(name));
        if o == t || t == b {
            continue;
        }
        if o == b {
            match t {
                Some(d) => {
                    result.insert(name.clone(), d.clone());
                }
                None => {
                    result.remove(name);
                }
            }
            continue;
        }
        let schemas: BTreeSet<&ObjectId> =
            [b, o, t].into_iter().flatten().map(|d| &d.schema).collect();
        if schemas.len() != 1 {
            return Err(Error::Unsupported(format!("schema merge: {name}")));
        }
        if b.is_some() && (o.is_none() || t.is_none()) {
            return Err(Error::Unsupported(format!(
                "dataset deletion versus modification: {name}"
            )));
        }
        let Some(schema) = schemas.into_iter().next() else {
            continue;
        };
        let mut merged = o.cloned().unwrap_or(Dataset {
            schema: schema.clone(),
            root: None,
            records: 0,
        });
        let mut keys = BTreeSet::new();
        for side in [o, t] {
            keys.extend(
                tree::diff(
                    store,
                    b.and_then(|d| d.root.as_ref()),
                    side.and_then(|d| d.root.as_ref()),
                )?
                .into_iter()
                .map(|d| d.key),
            );
        }
        for key in keys {
            let (base, ours, theirs) = (
                record(store, b, &key)?,
                record(store, o, &key)?,
                record(store, t, &key)?,
            );
            match merge_record(base.as_ref(), ours.as_ref(), theirs.as_ref()) {
                Ok(value) => update(store, &mut merged, &key, value.as_ref())?,
                Err(fields) => conflicts.push(Conflict {
                    dataset: name.clone(),
                    key,
                    base,
                    ours,
                    theirs,
                    fields,
                }),
            }
        }
        result.insert(name.clone(), merged);
    }
    Ok((result, conflicts))
}
