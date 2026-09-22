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
    Ok(diff_limited(store, before, after, usize::MAX)?.0)
}

/// Count every tree delta, but only decode record payloads for the requested page.
/// Traversal streams deltas; only the requested record payloads are retained.
pub fn diff_limited(
    store: &dyn ObjectStore,
    before: &Snapshot,
    after: &Snapshot,
    limit: usize,
) -> Result<(Vec<Change>, usize)> {
    diff_page(store, before, after, limit, usize::MAX)
}

/// Byte-bounded preview. The last decoded record may exceed the budget;
/// exact total counting still visits every tree delta.
pub fn diff_page(
    store: &dyn ObjectStore,
    before: &Snapshot,
    after: &Snapshot,
    limit: usize,
    byte_limit: usize,
) -> Result<(Vec<Change>, usize)> {
    let mut bytes = 0usize;
    let names: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    let mut result = Vec::new();
    let mut total = 0;
    for name in names {
        let a = before.get(name);
        let b = after.get(name);
        tree::visit_diff(
            store,
            a.and_then(|d| d.root.as_ref()),
            b.and_then(|d| d.root.as_ref()),
            &mut |d| {
                total += 1;
                if result.len() < limit && bytes < byte_limit {
                    let change = decode_change(store, name, d)?;
                    bytes = bytes
                        .saturating_add(change.before.as_ref().map_or(0, Record::payload_bytes))
                        .saturating_add(change.after.as_ref().map_or(0, Record::payload_bytes));
                    result.push(change);
                }
                Ok(())
            },
        )?;
    }

    Ok((result, total))
}

fn decode_change(store: &dyn ObjectStore, name: &str, d: tree::Delta) -> Result<Change> {
    let before: Option<Record> = d
        .before
        .as_ref()
        .map(|id| load(store, "record/v1", id))
        .transpose()?;
    let after: Option<Record> = d
        .after
        .as_ref()
        .map(|id| load(store, "record/v1", id))
        .transpose()?;
    let fields = changed_fields(before.as_ref(), after.as_ref());
    Ok(Change {
        dataset: name.into(),
        key: d.key,
        before,
        after,
        fields,
    })
}
/// Visit one decoded change at a time, retaining no complete diff in memory.
pub fn visit_diff(
    store: &dyn ObjectStore,
    before: &Snapshot,
    after: &Snapshot,
    visit: &mut dyn FnMut(Change) -> Result<()>,
) -> Result<()> {
    for name in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        tree::visit_diff(
            store,
            before.get(name).and_then(|d| d.root.as_ref()),
            after.get(name).and_then(|d| d.root.as_ref()),
            &mut |d| visit(decode_change(store, name, d)?),
        )?;
    }
    Ok(())
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
    three_way_with_defaults(
        store,
        base,
        ours,
        theirs,
        &mut |_, _| Ok(Default::default()),
    )
}

type ProjectionDefaults<'a> = dyn FnMut(&crate::Schema, &crate::Schema) -> Result<std::collections::BTreeMap<String, crate::Cell>>
    + 'a;

pub fn three_way_with_defaults(
    store: &dyn ObjectStore,
    base: &Snapshot,
    ours: &Snapshot,
    theirs: &Snapshot,
    defaults: &mut ProjectionDefaults<'_>,
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
            let (Some(b), Some(o), Some(t)) = (b, o, t) else {
                return Err(Error::Conflict(format!("schema/dataset conflict: {name}")));
            };
            let (bs, os, ts) = (
                crate::schema::read(store, &b.schema)?,
                crate::schema::read(store, &o.schema)?,
                crate::schema::read(store, &t.schema)?,
            );
            let target = crate::schema::merge(&bs, &os, &ts)?;
            let bp = crate::schema::Projection::new(&bs, &target, &defaults(&bs, &target)?);
            let op = crate::schema::Projection::new(&os, &target, &defaults(&os, &target)?);
            let tp = crate::schema::Projection::new(&ts, &target, &defaults(&ts, &target)?);
            let target_ids: BTreeSet<_> =
                target.fields.iter().map(crate::schema::column_id).collect();
            let mut merged = o.clone();
            merged.schema = crate::schema::store(store, &target)?;
            if os != target {
                let mut builder = tree::BulkBuilder::default();
                tree::visit(store, o.root.as_ref(), &mut |key, id| {
                    let r: Record = load(store, "record/v1", id)?;
                    let r = op.apply(&r);
                    builder.push(store, key.into(), save(store, "record/v1", &r)?)
                })?;
                merged.root = builder.finish(store)?;
            }
            let mut keys = BTreeSet::new();
            for side in [o, t] {
                keys.extend(
                    tree::diff(store, b.root.as_ref(), side.root.as_ref())?
                        .into_iter()
                        .map(|d| d.key),
                );
            }
            for key in keys {
                let br = record(store, Some(b), &key)?;
                let or = record(store, Some(o), &key)?;
                let tr = record(store, Some(t), &key)?;
                for f in bs
                    .fields
                    .iter()
                    .filter(|f| !target_ids.contains(&crate::schema::column_id(f)))
                {
                    let id = crate::schema::column_id(f);
                    for (schema, row) in [(&os, &or), (&ts, &tr)] {
                        if let (Some(field), Some(row)) = (
                            schema
                                .fields
                                .iter()
                                .find(|f| crate::schema::column_id(f) == id),
                            row,
                        ) && match br.as_ref() {
                            Some(base) => row.fields.get(&field.name) != base.fields.get(&f.name),
                            None => row
                                .fields
                                .get(&field.name)
                                .is_some_and(|v| !matches!(v, crate::Cell::Null)),
                        } {
                            return Err(Error::Conflict(format!(
                                "column deletion versus data edit: {name}.{}",
                                f.name
                            )));
                        }
                    }
                }
                let br = br.as_ref().map(|r| bp.apply(r));
                let or = or.as_ref().map(|r| op.apply(r));
                let tr = tr.as_ref().map(|r| tp.apply(r));
                match merge_record(br.as_ref(), or.as_ref(), tr.as_ref()) {
                    Ok(value) => update(store, &mut merged, &key, value.as_ref())?,
                    Err(fields) => conflicts.push(Conflict {
                        dataset: name.clone(),
                        key,
                        base: br,
                        ours: or,
                        theirs: tr,
                        fields,
                    }),
                }
            }
            result.insert(name.clone(), merged);
            continue;
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
