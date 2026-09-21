//! A deterministic, content-addressed treap. Mutations copy a search path,
//! unchanged subtrees retain their object IDs. BulkBuilder imports sorted keys
//! without persisting the intermediate roots of N individual insertions.
use crate::{Error, ObjectId, ObjectStore, Result, load, save};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_DEPTH: usize = 512;
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Node {
    key: String,
    value: ObjectId,
    left: Option<ObjectId>,
    right: Option<ObjectId>,
}
fn node(store: &dyn ObjectStore, id: &ObjectId) -> Result<Node> {
    load(store, "tree-node/v1", id)
}
fn write(store: &dyn ObjectStore, n: &Node) -> Result<ObjectId> {
    save(store, "tree-node/v1", n)
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
    if key.len() > 8192 {
        return Err(Error::Invalid("record key exceeds 8192 bytes".into()));
    }
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
    diff_inner(store, before, after, &mut out, 0)?;
    Ok(out)
}
fn diff_inner(
    store: &dyn ObjectStore,
    before: Option<&ObjectId>,
    after: Option<&ObjectId>,
    out: &mut Vec<Delta>,
    depth: usize,
) -> Result<()> {
    depth_guard(depth)?;
    if before == after {
        return Ok(());
    }
    if let (Some(a), Some(b)) = (before, after) {
        let a = node(store, a)?;
        let b = node(store, b)?;
        if a.key == b.key {
            diff_inner(store, a.left.as_ref(), b.left.as_ref(), out, depth + 1)?;
            if a.value != b.value {
                out.push(Delta {
                    key: a.key,
                    before: Some(a.value),
                    after: Some(b.value),
                });
            }
            diff_inner(store, a.right.as_ref(), b.right.as_ref(), out, depth + 1)?;
            return Ok(());
        }
    }
    // Rotated roots use a bounded-by-subtree comparison in this MVP. A future
    // range-aware iterator can eliminate this materialization without a format change.
    let mut values = BTreeMap::<String, (Option<ObjectId>, Option<ObjectId>)>::new();
    visit(store, before, &mut |k, id| {
        values.entry(k.into()).or_default().0 = Some(id.clone());
        Ok(())
    })?;
    visit(store, after, &mut |k, id| {
        values.entry(k.into()).or_default().1 = Some(id.clone());
        Ok(())
    })?;
    for (key, (before, after)) in values {
        if before != after {
            out.push(Delta { key, before, after });
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct BulkBuilder {
    stack: Vec<Node>,
    previous: Option<String>,
}
impl BulkBuilder {
    pub fn push(&mut self, store: &dyn ObjectStore, key: String, value: ObjectId) -> Result<()> {
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
