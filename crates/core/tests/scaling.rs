#![allow(clippy::unwrap_used)]
use geoledger_core::{graph, object::MemoryStore, tree, *};
use std::{
    cell::{Cell as Counter, RefCell},
    collections::BTreeMap,
};

#[derive(Default)]
struct Counting {
    inner: MemoryStore,
    reads: Counter<usize>,
    visits: RefCell<BTreeMap<(ObjectId, String), usize>>,
}
impl ObjectStore for Counting {
    fn put(&self, kind: &str, bytes: &[u8]) -> Result<ObjectId> {
        self.inner.put(kind, bytes)
    }
    fn get(&self, id: &ObjectId, kind: &str) -> Result<Vec<u8>> {
        self.reads.set(self.reads.get() + 1);
        *self
            .visits
            .borrow_mut()
            .entry((id.clone(), kind.into()))
            .or_default() += 1;
        self.inner.get(id, kind)
    }
}
fn commit(s: &Counting, parents: Vec<ObjectId>, message: &str) -> ObjectId {
    save(
        s,
        "commit/v3",
        &Commit {
            version: geoledger_core::FORMAT_VERSION,
            parents,
            root: save(s, "snapshot/v3", &Snapshot::new()).unwrap(),
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

#[test]
fn fsck_caches_shared_structure_and_records_per_schema() {
    let s = Counting::default();
    let schema = Schema {
        version: FORMAT_VERSION,
        kind: DatasetKind::Table,
        primary_key: "id".into(),
        fields: vec![Field {
            name: "id".into(),
            logical_type: "text".into(),
            codec: "text".into(),
            nullable: false,
            geometry: false,
            metadata: BTreeMap::new(),
        }],
        metadata: BTreeMap::new(),
    };
    let schema = schema::with_identities(schema);
    let schema_id = schema::store(&s, &schema).unwrap();
    let mut builder = tree::BulkBuilder::default();
    for i in 0..1000 {
        let key = format!("{i:06}");
        let row = Record {
            key: key.clone(),
            fields: BTreeMap::from([("id".into(), Cell::Text(key.clone()))]),
        };
        builder
            .push(&s, key, save(&s, "record/v3", &row).unwrap())
            .unwrap();
    }
    let mut dataset = Dataset {
        schema: schema_id,
        root: builder.finish(&s).unwrap(),
        records: 1000,
    };
    let mut versions = vec![dataset.clone()];
    for i in 1000..1100 {
        let key = format!("{i:06}");
        merge::update(
            &s,
            &mut dataset,
            &key,
            Some(&Record {
                key: key.clone(),
                fields: BTreeMap::from([("id".into(), Cell::Text(key.clone()))]),
            }),
        )
        .unwrap();
        versions.push(dataset.clone());
    }
    s.reads.set(0);
    s.visits.borrow_mut().clear();
    let mut validator = tree::Validator::default();
    for d in &versions {
        validator.dataset(&s, d).unwrap();
    }
    assert!(
        s.reads.get() < 6000,
        "{} reads for shared history",
        s.reads.get()
    );
    for ((_, kind), visits) in s.visits.borrow().iter() {
        if kind == "record/v3" || kind == "tree-node/v3" {
            assert_eq!(
                *visits, 1,
                "each reachable structure and record is validated once"
            );
        }
    }
    let reads = s.reads.get();
    validator.dataset(&s, &dataset).unwrap();
    assert_eq!(s.reads.get(), reads + 1); // Only schema reload, no rows/subtrees.
    let mut invalid_schema = schema.clone();
    invalid_schema.fields[0].geometry = true;
    dataset.schema = schema::store(&s, &invalid_schema).unwrap();
    assert!(validator.dataset(&s, &dataset).is_err());
    dataset = versions[0].clone();
    dataset.records += 1;
    assert!(validator.dataset(&s, &dataset).is_err());
}

#[test]
fn merge_delivers_first_conflict_without_collecting_all_changed_keys() {
    let store = Counting::default();
    let schema = object::digest("schema", b"shared");
    let mut sides = Vec::new();
    for value in ["left", "right"] {
        let mut builder = tree::BulkBuilder::default();
        for i in 0..2000 {
            let key = format!("{i:06}");
            let record = Record {
                key: key.clone(),
                fields: BTreeMap::from([("v".into(), Cell::Text(value.into()))]),
            };
            builder
                .push(&store, key, save(&store, "record/v3", &record).unwrap())
                .unwrap();
        }
        sides.push(Snapshot::from([(
            "rows".into(),
            Dataset {
                schema: schema.clone(),
                root: builder.finish(&store).unwrap(),
                records: 2000,
            },
        )]));
    }
    store.reads.set(0);
    let result = merge::three_way_streaming(
        &store,
        &Snapshot::new(),
        &sides[0],
        &sides[1],
        &mut |_, _| Ok(BTreeMap::new()),
        &mut |c| {
            assert_eq!(c.key, "000000");
            Err(Error::Invalid("stop at first conflict".into()))
        },
    );
    assert!(result.is_err());
    assert!(
        store.reads.get() < 150,
        "read {} objects before the first conflict",
        store.reads.get()
    );
    let mut count = 0;
    let result = merge::three_way_streaming(
        &store,
        &Snapshot::new(),
        &sides[0],
        &sides[1],
        &mut |_, _| Ok(BTreeMap::new()),
        &mut |c| {
            assert_eq!(c.key, format!("{count:06}"));
            count += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(count, 2000);
    assert_eq!(result, sides[0]);
}
