//! A deterministic, content-addressed treap. Mutations copy a search path,
//! unchanged subtrees retain their object IDs. BulkBuilder imports sorted keys
//! without persisting the intermediate roots of N individual insertions.
use crate::{Error, ObjectId, ObjectStore, Result, load, save};
use serde::{Deserialize, Serialize};

const MAX_DEPTH: usize = 512;
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Node {
    key: String,
    value: ObjectId,
    left: Option<ObjectId>,
    right: Option<ObjectId>,
}
fn node(store: &dyn ObjectStore, id: &ObjectId) -> Result<Node> {
    load(store, "tree-node/v3", id)
}
fn write(store: &dyn ObjectStore, n: &Node) -> Result<ObjectId> {
    save(store, "tree-node/v3", n)
}
fn priority(key: &str) -> ([u8; 32], &str) {
    (*blake3::hash(key.as_bytes()).as_bytes(), key)
}
fn depth_guard(depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        Err(Error::Storage("tree depth limit exceeded".into()))
    } else {
        Ok(())
    }
}
pub(crate) fn validate_key(key: &str) -> Result<()> {
    if key.len() > 8192 {
        return Err(Error::Invalid("record key exceeds 8192 bytes".into()));
    }
    Ok(())
}

pub fn get(
    store: &dyn ObjectStore,
    root: Option<&ObjectId>,
    key: &str,
) -> Result<Option<ObjectId>> {
    let mut cursor = root.cloned();
    let mut depth = 0;
    while let Some(id) = cursor {
        depth_guard(depth)?;
        depth += 1;
        let n = node(store, &id)?;
        match key.cmp(&n.key) {
            std::cmp::Ordering::Equal => return Ok(Some(n.value)),
            std::cmp::Ordering::Less => cursor = n.left,
            std::cmp::Ordering::Greater => cursor = n.right,
        }
    }
    Ok(None)
}

pub fn set(
    store: &dyn ObjectStore,
    root: Option<&ObjectId>,
    key: &str,
    value: Option<&ObjectId>,
) -> Result<Option<ObjectId>> {
    validate_key(key)?;
    change(store, root, key, value, 0)
}
fn change(
    store: &dyn ObjectStore,
    root: Option<&ObjectId>,
    key: &str,
    value: Option<&ObjectId>,
    depth: usize,
) -> Result<Option<ObjectId>> {
    depth_guard(depth)?;
    let Some(id) = root else {
        return value
            .map(|value| {
                write(
                    store,
                    &Node {
                        key: key.into(),
                        value: value.clone(),
                        left: None,
                        right: None,
                    },
                )
            })
            .transpose();
    };
    let mut n = node(store, id)?;
    if key == n.key {
        return match value {
            Some(value) if *value == n.value => Ok(Some(id.clone())),
            Some(value) => {
                n.value = value.clone();
                Ok(Some(write(store, &n)?))
            }
            None => join(store, n.left.as_ref(), n.right.as_ref(), depth + 1),
        };
    }
    if let Some(value) = value
        && priority(key) < priority(&n.key)
    {
        let (left, right) = split(store, Some(id), key, depth + 1)?;
        return Ok(Some(write(
            store,
            &Node {
                key: key.into(),
                value: value.clone(),
                left,
                right,
            },
        )?));
    }
    if key < n.key.as_str() {
        let child = change(store, n.left.as_ref(), key, value, depth + 1)?;
        if child == n.left {
            return Ok(Some(id.clone()));
        }
        n.left = child;
    } else {
        let child = change(store, n.right.as_ref(), key, value, depth + 1)?;
        if child == n.right {
            return Ok(Some(id.clone()));
        }
        n.right = child;
    }
    Ok(Some(write(store, &n)?))
}
fn split(
    store: &dyn ObjectStore,
    root: Option<&ObjectId>,
    key: &str,
    depth: usize,
) -> Result<(Option<ObjectId>, Option<ObjectId>)> {
    depth_guard(depth)?;
    let Some(id) = root else {
        return Ok((None, None));
    };
    let mut n = node(store, id)?;
    if n.key.as_str() < key {
        let (left, right) = split(store, n.right.as_ref(), key, depth + 1)?;
        n.right = left;
        Ok((Some(write(store, &n)?), right))
    } else {
        let (left, right) = split(store, n.left.as_ref(), key, depth + 1)?;
        n.left = right;
        Ok((left, Some(write(store, &n)?)))
    }
}
fn join(
    store: &dyn ObjectStore,
    left: Option<&ObjectId>,
    right: Option<&ObjectId>,
    depth: usize,
) -> Result<Option<ObjectId>> {
    depth_guard(depth)?;
    let (Some(l), Some(r)) = (left, right) else {
        return Ok(left.or(right).cloned());
    };
    let mut a = node(store, l)?;
    let mut b = node(store, r)?;
    if priority(&a.key) < priority(&b.key) {
        a.right = join(store, a.right.as_ref(), Some(r), depth + 1)?;
        Ok(Some(write(store, &a)?))
    } else {
        b.left = join(store, Some(l), b.left.as_ref(), depth + 1)?;
        Ok(Some(write(store, &b)?))
    }
}

pub fn visit(
    store: &dyn ObjectStore,
    root: Option<&ObjectId>,
    f: &mut dyn FnMut(&str, &ObjectId) -> Result<()>,
) -> Result<()> {
    fn walk(
        store: &dyn ObjectStore,
        id: &ObjectId,
        depth: usize,
        f: &mut dyn FnMut(&str, &ObjectId) -> Result<()>,
    ) -> Result<()> {
        depth_guard(depth)?;
        let n = node(store, id)?;
        if let Some(left) = n.left {
            walk(store, &left, depth + 1, f)?;
        }
        f(&n.key, &n.value)?;
        if let Some(right) = n.right {
            walk(store, &right, depth + 1, f)?;
        }
        Ok(())
    }
    if let Some(id) = root {
        walk(store, id, 0, f)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Delta {
    pub key: String,
    pub before: Option<ObjectId>,
    pub after: Option<ObjectId>,
}

pub fn diff(
    store: &dyn ObjectStore,
    before: Option<&ObjectId>,
    after: Option<&ObjectId>,
) -> Result<Vec<Delta>> {
    let mut out = Vec::new();
    visit_diff(store, before, after, &mut |delta| {
        out.push(delta);
        Ok(())
    })?;
    Ok(out)
}

enum DiffTask {
    Tree(ObjectId, usize),
    Entry(String, ObjectId),
}
fn expand(stack: &mut Vec<DiffTask>, n: Node, depth: usize) -> Result<()> {
    depth_guard(depth)?;
    stack.pop();
    if let Some(id) = n.right {
        stack.push(DiffTask::Tree(id, depth + 1));
    }
    stack.push(DiffTask::Entry(n.key, n.value));
    if let Some(id) = n.left {
        stack.push(DiffTask::Tree(id, depth + 1));
    }
    Ok(())
}

/// Stream sorted deltas with O(tree depth) traversal memory. Equal subtrees are
/// skipped before decoding, including when an insertion rotates the root.
pub fn visit_diff(
    store: &dyn ObjectStore,
    before: Option<&ObjectId>,
    after: Option<&ObjectId>,
    visit: &mut dyn FnMut(Delta) -> Result<()>,
) -> Result<()> {
    let mut diff = Diff::new(store, before, after);
    while let Some(delta) = diff.next()? {
        visit(delta)?;
    }
    Ok(())
}

/// Resumable sorted traversal; retains only the two search stacks.
pub(crate) struct Diff<'a> {
    store: &'a dyn ObjectStore,
    before: Vec<DiffTask>,
    after: Vec<DiffTask>,
}
impl<'a> Diff<'a> {
    pub(crate) fn new(
        store: &'a dyn ObjectStore,
        before: Option<&ObjectId>,
        after: Option<&ObjectId>,
    ) -> Self {
        Self {
            store,
            before: before
                .into_iter()
                .map(|id| DiffTask::Tree(id.clone(), 0))
                .collect(),
            after: after
                .into_iter()
                .map(|id| DiffTask::Tree(id.clone(), 0))
                .collect(),
        }
    }
    pub(crate) fn next(&mut self) -> Result<Option<Delta>> {
        let store = self.store;
        let a = &mut self.before;
        let b = &mut self.after;
        loop {
            match (a.last(), b.last()) {
                (None, None) => return Ok(None),
                (Some(DiffTask::Tree(x, _)), Some(DiffTask::Tree(y, _))) if x == y => {
                    a.pop();
                    b.pop();
                }
                (Some(DiffTask::Tree(x, dx)), Some(DiffTask::Tree(y, dy))) => {
                    depth_guard(*dx)?;
                    depth_guard(*dy)?;
                    let (x, y, dx, dy) = (node(store, x)?, node(store, y)?, *dx, *dy);
                    // Expand the root with the earlier treap priority first. Keeping
                    // the other subtree intact allows shared children to align.
                    let order = priority(&x.key).cmp(&priority(&y.key));
                    if !order.is_gt() {
                        expand(a, x, dx)?;
                    }
                    if !order.is_lt() {
                        expand(b, y, dy)?;
                    }
                }
                (Some(DiffTask::Tree(id, depth)), _) => {
                    let (n, depth) = (node(store, id)?, *depth);
                    expand(a, n, depth)?;
                }
                (_, Some(DiffTask::Tree(id, depth))) => {
                    let (n, depth) = (node(store, id)?, *depth);
                    expand(b, n, depth)?;
                }
                (Some(DiffTask::Entry(x, _)), Some(DiffTask::Entry(y, _))) if x == y => {
                    if let (Some(DiffTask::Entry(key, before)), Some(DiffTask::Entry(_, after))) =
                        (a.pop(), b.pop())
                        && before != after
                    {
                        return Ok(Some(Delta {
                            key,
                            before: Some(before),
                            after: Some(after),
                        }));
                    }
                }
                (Some(DiffTask::Entry(x, _)), Some(DiffTask::Entry(y, _))) if x < y => {
                    if let Some(DiffTask::Entry(key, value)) = a.pop() {
                        return Ok(Some(Delta {
                            key,
                            before: Some(value),
                            after: None,
                        }));
                    }
                }
                (Some(DiffTask::Entry(_, _)), None) => {
                    if let Some(DiffTask::Entry(key, value)) = a.pop() {
                        return Ok(Some(Delta {
                            key,
                            before: Some(value),
                            after: None,
                        }));
                    }
                }
                _ => {
                    if let Some(DiffTask::Entry(key, value)) = b.pop() {
                        return Ok(Some(Delta {
                            key,
                            before: None,
                            after: Some(value),
                        }));
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub struct BulkBuilder {
    stack: Vec<Node>,
    previous: Option<String>,
}
impl BulkBuilder {
    pub fn push(&mut self, store: &dyn ObjectStore, key: String, value: ObjectId) -> Result<()> {
        validate_key(&key)?;
        if self.previous.as_ref().is_some_and(|p| p >= &key) {
            return Err(Error::Invalid(
                "bulk keys must be strictly increasing (UTF-8 byte order)".into(),
            ));
        }
        self.previous = Some(key.clone());
        let mut left = None;
        while self
            .stack
            .last()
            .is_some_and(|n| priority(&n.key) > priority(&key))
        {
            let Some(mut n) = self.stack.pop() else {
                break;
            };
            n.right = left;
            left = Some(write(store, &n)?);
        }
        self.stack.push(Node {
            key,
            value,
            left,
            right: None,
        });
        depth_guard(self.stack.len())
    }
    pub fn finish(mut self, store: &dyn ObjectStore) -> Result<Option<ObjectId>> {
        let mut root = None;
        while let Some(mut n) = self.stack.pop() {
            n.right = root;
            root = Some(write(store, &n)?);
        }
        Ok(root)
    }
}

/// Compact validation cache scoped to one fsck. Schema IDs are part of every
/// cache key: a shared record/tree must be checked again under another schema.
#[derive(Default)]
pub struct Validator {
    trees: std::collections::BTreeMap<(ObjectId, ObjectId), CheckedTree>,
    records: std::collections::BTreeMap<(ObjectId, ObjectId), String>,
}
#[derive(Clone)]
struct CheckedTree {
    min: String,
    max: String,
    root_key: String,
    count: u64,
    height: usize,
}
impl Validator {
    pub fn dataset(&mut self, store: &dyn ObjectStore, dataset: &crate::Dataset) -> Result<()> {
        let schema = crate::schema::read(store, &dataset.schema)?;
        let count = match &dataset.root {
            None => 0,
            Some(root) => {
                self.walk(
                    store,
                    &dataset.schema,
                    &schema,
                    root,
                    0,
                    &mut std::collections::BTreeSet::new(),
                )?
                .count
            }
        };
        if count != dataset.records {
            return Err(Error::Storage("dataset record count mismatch".into()));
        }
        Ok(())
    }
    fn walk(
        &mut self,
        store: &dyn ObjectStore,
        schema_id: &ObjectId,
        schema: &crate::Schema,
        id: &ObjectId,
        depth: usize,
        active: &mut std::collections::BTreeSet<ObjectId>,
    ) -> Result<CheckedTree> {
        depth_guard(depth)?;
        let identity = (schema_id.clone(), id.clone());
        if let Some(cached) = self.trees.get(&identity) {
            depth_guard(depth + cached.height - 1)?;
            return Ok(cached.clone());
        }
        if !active.insert(id.clone()) {
            return Err(Error::Storage("tree cycle".into()));
        }
        let n = node(store, id)?;
        if n.key.len() > 8192 {
            return Err(Error::Storage("oversized tree key".into()));
        }
        let record_identity = (schema_id.clone(), n.value.clone());
        if let Some(key) = self.records.get(&record_identity) {
            if key != &n.key {
                return Err(Error::Storage("tree key/record key mismatch".into()));
            }
        } else {
            let record: crate::Record = load(store, "record/v3", &n.value)?;
            if record.key != n.key {
                return Err(Error::Storage("tree key/record key mismatch".into()));
            }
            schema
                .validate(&record)
                .map_err(|e| Error::storage_source("invalid stored record", e))?;
            self.records.insert(record_identity, record.key);
        }
        let mut checked = CheckedTree {
            min: n.key.clone(),
            max: n.key.clone(),
            root_key: n.key.clone(),
            count: 1,
            height: 1,
        };
        for (child, left) in [(&n.left, true), (&n.right, false)] {
            if let Some(child) = child {
                let child = self.walk(store, schema_id, schema, child, depth + 1, active)?;
                if (left && child.max >= n.key)
                    || (!left && child.min <= n.key)
                    || priority(&child.root_key) <= priority(&n.key)
                {
                    return Err(Error::Storage(
                        "tree ordering/priority invariant violated".into(),
                    ));
                }
                checked.count = checked
                    .count
                    .checked_add(child.count)
                    .ok_or_else(|| Error::Storage("tree count overflow".into()))?;
                checked.height = checked.height.max(child.height + 1);
                if left {
                    checked.min = child.min;
                } else {
                    checked.max = child.max;
                }
            }
        }
        active.remove(id);
        self.trees.insert(identity, checked.clone());
        Ok(checked)
    }
}
