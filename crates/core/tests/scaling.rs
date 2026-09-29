#![allow(clippy::unwrap_used)]
use geoledger_core::{graph, object::MemoryStore, tree, *};
use std::{cell::Cell as Counter, collections::BTreeMap};

#[derive(Default)]
struct Counting {
    inner: MemoryStore,
    reads: Counter<usize>,
}
impl ObjectStore for Counting {
    fn put(&self, kind: &str, bytes: &[u8]) -> Result<ObjectId> {
        self.inner.put(kind, bytes)
    }
    fn get(&self, id: &ObjectId, kind: &str) -> Result<Vec<u8>> {
        self.reads.set(self.reads.get() + 1);
        self.inner.get(id, kind)
    }
}
fn commit(s: &Counting, parents: Vec<ObjectId>, message: &str) -> ObjectId {
    save(
        s,
        "commit/v1",
        &Commit {
            version: 1,
            parents,
            root: save(s, "snapshot/v1", &Snapshot::new()).unwrap(),
            author: "test".into(),
            message: message.into(),
            timestamp: "fixed".into(),
        },
    )
    .unwrap()
}

#[test]
fn merge_base_reads_shared_history_once_and_rejects_multiple_best_bases() {
    let s = Counting::default();
    let mut head = commit(&s, vec![], "root");
    for i in 0..1000 {
        head = commit(&s, vec![head], &i.to_string());
    }
    let a = commit(&s, vec![head.clone()], "left");
    let b = commit(&s, vec![head.clone()], "right");
    s.reads.set(0);
    assert_eq!(graph::merge_base(&s, &a, &b).unwrap(), head);
    assert!(s.reads.get() <= 1003);
    s.reads.set(0);
    assert_eq!(graph::merge_base(&s, &a, &a).unwrap(), a);
    assert_eq!(s.reads.get(), 1);
    let x = commit(&s, vec![a.clone(), b.clone()], "x");
    let y = commit(&s, vec![b, a], "y");
    assert!(matches!(
        graph::merge_base(&s, &x, &y),
        Err(Error::Unsupported(_))
    ));
    let unrelated = commit(&s, vec![], "unrelated");
    assert!(matches!(
        graph::merge_base(&s, &x, &unrelated),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn root_rotation_diff_skips_shared_subtrees_in_both_directions() {
    let s = Counting::default();
    let value = s.put("value", b"one").unwrap();
    let keys: Vec<_> = (0..4000).map(|i| format!("{i:06}")).collect();
    let min = keys
        .iter()
        .map(|k| *blake3::hash(k.as_bytes()).as_bytes())
        .min()
        .unwrap();
    let mut builder = tree::BulkBuilder::default();
    for k in keys {
        builder.push(&s, k, value.clone()).unwrap();
    }
    let before = builder.finish(&s).unwrap();
    for prefix in ["new", "002000_"] {
        let key = (0..100_000)
            .map(|i| format!("{prefix}{i}"))
            .find(|k| *blake3::hash(k.as_bytes()).as_bytes() < min)
            .unwrap();
        let after = tree::set(&s, before.as_ref(), &key, Some(&value)).unwrap();
        for (a, b) in [(&before, &after), (&after, &before)] {
            s.reads.set(0);
            let delta = tree::diff(&s, a.as_ref(), b.as_ref()).unwrap();
            assert_eq!(delta.len(), 1);
            assert_eq!(delta[0].key, key);
            assert!(s.reads.get() < 150, "read {} nodes", s.reads.get());
        }
    }
}

#[test]
fn streamed_diffs_match_reference_maps_across_arbitrary_tree_shapes() {
    let s = Counting::default();
    let mut roots = [None, None];
    let mut maps = [BTreeMap::new(), BTreeMap::new()];
    for i in 0..1200 {
        let side = i % 2;
        let key = format!("{:04}", (i * 137) % 317);
        let value = if i % 7 == 0 {
            None
        } else {
            Some(s.put("value", i.to_string().as_bytes()).unwrap())
        };
        roots[side] = tree::set(&s, roots[side].as_ref(), &key, value.as_ref()).unwrap();
        if let Some(v) = value {
            maps[side].insert(key, v);
        } else {
            maps[side].remove(&key);
        }
    }
    let expected: BTreeMap<_, _> = maps[0]
        .keys()
        .chain(maps[1].keys())
        .filter(|k| maps[0].get(*k) != maps[1].get(*k))
        .map(|k| {
            (
                k.clone(),
                (maps[0].get(k).cloned(), maps[1].get(k).cloned()),
            )
        })
        .collect();
    let mut actual = BTreeMap::new();
    tree::visit_diff(&s, roots[0].as_ref(), roots[1].as_ref(), &mut |d| {
        actual.insert(d.key, (d.before, d.after));
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, expected);
    let error = tree::visit_diff(&s, roots[0].as_ref(), roots[1].as_ref(), &mut |_| {
        Err(Error::Invalid("stop".into()))
    });
    assert!(matches!(error, Err(Error::Invalid(_))));
}

#[test]
fn backend_errors_keep_the_original_source_and_category() {
    use std::error::Error as _;
    let error = Error::storage_source("reading object", std::io::Error::other("disk failure"));
    assert_eq!(error.code(), "storage_error");
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .is_some()
    );
}
