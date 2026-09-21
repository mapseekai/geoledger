use crate::{Commit, Error, ObjectId, ObjectStore, Result, load};
use std::collections::{BTreeSet, VecDeque};
const MAX_COMMITS: usize = 100_000;

pub fn commit(store: &dyn ObjectStore, id: &ObjectId) -> Result<Commit> {
    let c: Commit = load(store, "commit/v1", id)?;
    if c.version != 1 {
        return Err(Error::Unsupported("commit format version".into()));
    }
    Ok(c)
}
pub fn ancestors(store: &dyn ObjectStore, start: &ObjectId) -> Result<BTreeSet<ObjectId>> {
    let mut visited = BTreeSet::new();
    let mut pending = VecDeque::from([start.clone()]);
    while let Some(id) = pending.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if visited.len() > MAX_COMMITS {
            return Err(Error::Unsupported(
                "history exceeds traversal budget".into(),
            ));
        }
        pending.extend(commit(store, &id)?.parents);
    }
    Ok(visited)
}
pub fn is_ancestor(
    store: &dyn ObjectStore,
    ancestor: &ObjectId,
    descendant: &ObjectId,
) -> Result<bool> {
    Ok(ancestors(store, descendant)?.contains(ancestor))
}
pub fn merge_base(store: &dyn ObjectStore, a: &ObjectId, b: &ObjectId) -> Result<ObjectId> {
    let aa = ancestors(store, a)?;
    let bb = ancestors(store, b)?;
    let mut best: BTreeSet<_> = aa.intersection(&bb).cloned().collect();
    for candidate in best.clone() {
        for older in ancestors(store, &candidate)? {
            if older != candidate {
                best.remove(&older);
            }
        }
    }
    if best.len() > 1 {
        return Err(Error::Unsupported(
            "multiple best merge bases (criss-cross history)".into(),
        ));
    }
    best.into_iter()
        .next()
        .ok_or_else(|| Error::Conflict("unrelated histories".into()))
}
pub fn log(
    store: &dyn ObjectStore,
    start: &ObjectId,
    limit: usize,
) -> Result<Vec<(ObjectId, Commit)>> {
    let mut result = Vec::new();
    let mut visited = BTreeSet::new();
    let mut pending = VecDeque::from([start.clone()]);
    while let Some(id) = pending.pop_front() {
        if result.len() >= limit.min(1000) {
            break;
        }
        if !visited.insert(id.clone()) {
            continue;
        }
        let c = commit(store, &id)?;
        pending.extend(c.parents.clone());
        result.push((id, c));
    }
    Ok(result)
}
