use crate::{Commit, Error, ObjectId, ObjectStore, Result, load};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    if a == b {
        commit(store, a)?;
        return Ok(a.clone());
    }
    // Each node is decoded once. Reachability bits propagate at most twice per
    // edge, even when paths reconverge or the two heads share a long history.
    let mut nodes = BTreeMap::<ObjectId, (Vec<ObjectId>, u8)>::new();
    let mut pending = VecDeque::from([(a.clone(), 1u8), (b.clone(), 2u8)]);
    while let Some((id, side)) = pending.pop_front() {
        if !nodes.contains_key(&id) {
            if nodes.len() >= MAX_COMMITS {
                return Err(Error::Unsupported(
                    "history exceeds traversal budget".into(),
                ));
            }
            nodes.insert(id.clone(), (commit(store, &id)?.parents, 0));
        }
        if let Some((parents, seen)) = nodes.get_mut(&id) {
            let new = side & !*seen;
            if new != 0 {
                *seen |= new;
                pending.extend(parents.iter().map(|p| (p.clone(), new)));
            }
        }
    }
    let mut best: BTreeSet<_> = nodes
        .iter()
        .filter(|(_, (_, seen))| *seen == 3)
        .map(|(id, _)| id.clone())
        .collect();
    // Every parent of a common ancestor is itself common; removing immediate
    // parents across this set removes all older common ancestors in one pass.
    for (parents, seen) in nodes.values() {
        if *seen == 3 {
            for parent in parents {
                best.remove(parent);
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
