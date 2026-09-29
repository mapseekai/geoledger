#![allow(clippy::unwrap_used)]
use geoledger_core::{merge::merge_record, object::MemoryStore, tree, *};
use std::collections::BTreeMap;
fn row(a: &str, b: &str) -> Record {
    Record {
        key: "1".into(),
        fields: BTreeMap::from([
            ("a".into(), Cell::Text(a.into())),
            ("b".into(), Cell::Text(b.into())),
        ]),
    }
}
#[test]
fn independent_fields_merge() {
    assert_eq!(
        merge_record(
            Some(&row("0", "0")),
            Some(&row("1", "0")),
            Some(&row("0", "2"))
        )
        .unwrap(),
        Some(row("1", "2"))
    );
}
#[test]
fn conflicting_fields_are_named() {
    assert_eq!(
        merge_record(
            Some(&row("0", "0")),
            Some(&row("1", "0")),
            Some(&row("2", "0"))
        )
        .unwrap_err(),
        vec!["a"]
    );
}
#[test]
fn identical_concurrent_edits_merge() {
    assert_eq!(
        merge_record(
            Some(&row("0", "0")),
            Some(&row("1", "0")),
            Some(&row("1", "0"))
        )
        .unwrap(),
        Some(row("1", "0"))
    );
}
#[test]
fn delete_modify_conflicts() {
    assert!(merge_record(Some(&row("0", "0")), None, Some(&row("1", "0"))).is_err());
}
#[test]
fn insert_same_key_conflicts_unless_identical() {
    assert!(merge_record(None, Some(&row("0", "0")), Some(&row("1", "0"))).is_err());
    assert!(merge_record(None, Some(&row("0", "0")), Some(&row("0", "0"))).is_ok());
}
#[test]
fn geometry_is_atomic_and_preserved() {
    let mut base = row("0", "0");
    base.fields
        .insert("geom".into(), Cell::Geometry("00aaa".into()));
    let mut ours = base.clone();
    ours.fields
        .insert("geom".into(), Cell::Geometry("00bbb".into()));
    let mut theirs = base.clone();
    theirs
        .fields
        .insert("geom".into(), Cell::Geometry("00ccc".into()));
    assert_eq!(
        merge_record(Some(&base), Some(&ours), Some(&theirs)).unwrap_err(),
        vec!["geom"]
    );
}
#[test]
fn deterministic_tree_matches_bulk_import() {
    let store = MemoryStore::default();
    let mut a = None;
    let mut b = None;
    let value = save(&store, "record/v3", &row("0", "0")).unwrap();
    let keys: Vec<_> = (0..300).map(|i| format!("{i:04}")).collect();
    for key in &keys {
        a = tree::set(&store, a.as_ref(), key, Some(&value)).unwrap();
    }
    for key in keys.iter().rev() {
        b = tree::set(&store, b.as_ref(), key, Some(&value)).unwrap();
    }
    let mut bulk = tree::BulkBuilder::default();
    for key in &keys {
        bulk.push(&store, key.clone(), value.clone()).unwrap();
    }
    assert_eq!(a, b);
    assert_eq!(a, bulk.finish(&store).unwrap());
    for key in &keys {
        assert_eq!(
            tree::get(&store, a.as_ref(), key).unwrap(),
            Some(value.clone())
        );
    }
}
#[test]
fn changing_one_record_reuses_subtrees() {
    let store = MemoryStore::default();
    let value = save(&store, "record/v3", &row("0", "0")).unwrap();
    let mut b = tree::BulkBuilder::default();
    for i in 0..10_000 {
        b.push(&store, format!("{i:05}"), value.clone()).unwrap();
    }
    let root = b.finish(&store).unwrap();
    let count = store.object_count();
    let v2 = save(&store, "record/v3", &row("1", "0")).unwrap();
    let edited = tree::set(&store, root.as_ref(), "05000", Some(&v2)).unwrap();
    assert!(store.object_count() - count < 100);
    let delta = tree::diff(&store, root.as_ref(), edited.as_ref()).unwrap();
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0].key, "05000");
    let restored = tree::set(&store, edited.as_ref(), "05000", Some(&value)).unwrap();
    assert_eq!(root, restored);
}
#[test]
fn deletes_and_inserts_match_reference_map() {
    let store = MemoryStore::default();
    let value = save(&store, "record/v3", &row("0", "0")).unwrap();
    let mut root = None;
    let mut expected = BTreeMap::new();
    for i in 0..1000 {
        let key = format!("{}", (i * 137) % 313);
        let v = if i % 3 == 0 { None } else { Some(&value) };
        root = tree::set(&store, root.as_ref(), &key, v).unwrap();
        if let Some(v) = v {
            expected.insert(key, v.clone());
        } else {
            expected.remove(&key);
        }
    }
    let mut actual = BTreeMap::new();
    tree::visit(&store, root.as_ref(), &mut |k, v| {
        actual.insert(k.to_string(), v.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(expected, actual);
}
#[test]
fn object_hash_is_domain_separated() {
    let store = MemoryStore::default();
    assert_ne!(
        store.put("record/v3", b"{}").unwrap(),
        store.put("schema/v3", b"{}").unwrap()
    );
}
#[test]
fn malformed_ids_and_branch_names_fail() {
    assert!(ObjectId::parse("../../etc/passwd").is_err());
    for b in ["HEAD", "../main", "a//b", "a.lock", ""] {
        assert!(validate_branch(b).is_err());
    }
    validate_branch("feature/new-roads").unwrap();
}

#[test]
fn regression_bulk_and_incremental_key_limits_match() {
    let store = MemoryStore::default();
    let value = save(&store, "record/v3", &row("0", "0")).unwrap();
    let valid = "a".repeat(8192);
    let mut bulk = tree::BulkBuilder::default();
    bulk.push(&store, valid.clone(), value.clone()).unwrap();
    let root = tree::set(&store, None, &valid, Some(&value)).unwrap();
    for invalid in ["b".repeat(8193), "中".repeat(2731)] {
        assert!(tree::set(&store, root.as_ref(), &invalid, Some(&value)).is_err());
        let count = store.object_count();
        assert!(bulk.push(&store, invalid, value.clone()).is_err());
        assert_eq!(store.object_count(), count);
    }
    // A rejected key must not change the builder's ordering or pending nodes.
    bulk.push(&store, "c".into(), value.clone()).unwrap();
    let expected = tree::set(&store, root.as_ref(), "c", Some(&value)).unwrap();
    assert_eq!(bulk.finish(&store).unwrap(), expected);
}

#[test]
fn current_schema_roundtrips_and_validates_format_and_identities() {
    let store = MemoryStore::default();
    let current = schema::with_identities(Schema {
        version: FORMAT_VERSION,
        kind: DatasetKind::Table,
        primary_key: "id".into(),
        fields: vec![Field {
            name: "id".into(),
            logical_type: "text".into(),
            codec: "postgres-text/v1".into(),
            nullable: false,
            geometry: false,
            metadata: BTreeMap::new(),
        }],
        metadata: BTreeMap::new(),
    });
    assert_eq!(schema::COLUMN_ID, "geoledger.column-id");
    let id = schema::store(&store, &current).unwrap();
    assert_eq!(schema::read(&store, &id).unwrap(), current);
    for version in [1, 2, 4] {
        let mut invalid = current.clone();
        invalid.version = version;
        assert!(schema::store(&store, &invalid).is_err());
        let id = save(&store, "schema/v3", &invalid).unwrap();
        assert!(schema::read(&store, &id).is_err());
    }
    let mut missing = current.clone();
    missing.fields[0].metadata.clear();
    assert!(schema::store(&store, &missing).is_err());
    let mut duplicate = current.clone();
    duplicate.fields.push(current.fields[0].clone());
    assert!(schema::store(&store, &duplicate).is_err());
}

#[test]
fn object_digest_uses_current_geoledger_domain() {
    let kind = "record/v3";
    let payload = b"geoledger format regression";
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"geoledger\0object-v3\0");
    hasher.update(&(kind.len() as u64).to_be_bytes());
    hasher.update(kind.as_bytes());
    hasher.update(payload);
    assert_eq!(
        object::digest(kind, payload).as_str(),
        hasher.finalize().to_hex().as_str()
    );
}

#[test]
fn persisted_json_errors_keep_source_and_storage_code() {
    use std::error::Error as _;
    let store = MemoryStore::default();
    let id = store.put("record/v3", b"{").unwrap();
    let error = load::<Record>(&store, "record/v3", &id).unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(error.source().unwrap().is::<serde_json::Error>());
    let id = store.put("schema/v3", b"{").unwrap();
    let error = schema::read(&store, &id).unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(error.source().unwrap().is::<serde_json::Error>());
}

#[test]
fn merge_keeps_missing_distinct_from_null_and_conflicts_sorted() {
    let base = row("0", "0");
    let mut ours = base.clone();
    ours.fields.remove("a");
    let mut theirs = base.clone();
    theirs.fields.insert("b".into(), Cell::Null);
    let merged = merge_record(Some(&base), Some(&ours), Some(&theirs))
        .unwrap()
        .unwrap();
    assert!(!merged.fields.contains_key("a"));
    assert_eq!(merged.fields["b"], Cell::Null);
    theirs.fields.insert("a".into(), Cell::Null);
    ours.fields.insert("b".into(), Cell::Text("other".into()));
    assert_eq!(
        merge_record(Some(&base), Some(&ours), Some(&theirs)).unwrap_err(),
        vec!["a", "b"]
    );
}

#[test]
fn streaming_merge_matches_convenience_and_stops_on_sink_error() {
    let store = MemoryStore::default();
    let schema = store.put("schema/v3", b"unused equal schema").unwrap();
    let snapshot = |value: &str| {
        let mut d = Dataset {
            schema: schema.clone(),
            root: None,
            records: 0,
        };
        for key in ["a", "b", "c"] {
            let mut r = row(value, "geometry payload");
            r.key = key.into();
            merge::update(&store, &mut d, key, Some(&r)).unwrap();
        }
        Snapshot::from([("data".into(), d)])
    };
    let (b, o, t) = (snapshot("base"), snapshot("ours"), snapshot("theirs"));
    let expected = merge::three_way(&store, &b, &o, &t).unwrap();
    let mut received = Vec::new();
    let actual = merge::three_way_streaming(
        &store,
        &b,
        &o,
        &t,
        &mut |_, _| Ok(BTreeMap::new()),
        &mut |c| {
            received.push(c);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(actual, expected.0);
    assert_eq!(
        serde_json::to_value(received).unwrap(),
        serde_json::to_value(expected.1).unwrap()
    );
    let mut calls = 0;
    assert!(
        merge::three_way_streaming(
            &store,
            &b,
            &o,
            &t,
            &mut |_, _| Ok(BTreeMap::new()),
            &mut |_| {
                calls += 1;
                Err(Error::Storage("sink failed".into()))
            }
        )
        .is_err()
    );
    assert_eq!(calls, 1);
}

#[test]
fn fsck_rejects_corrupt_references_order_and_keys_even_with_cached_subtrees() {
    let store = MemoryStore::default();
    let schema = schema::with_identities(Schema {
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
    });
    let schema_id = schema::store(&store, &schema).unwrap();
    let value = |key: &str| {
        save(
            &store,
            "record/v3",
            &Record {
                key: key.into(),
                fields: BTreeMap::from([("id".into(), Cell::Text(key.into()))]),
            },
        )
        .unwrap()
    };
    let node = |key: &str, value: &ObjectId, left: Option<&ObjectId>, right: Option<&ObjectId>| {
        save(
            &store,
            "tree-node/v3",
            &serde_json::json!({"key":key,"value":value,"left":left,"right":right}),
        )
        .unwrap()
    };
    let leaf = node("z", &value("z"), None, None);
    let mut validator = tree::Validator::default();
    let dataset = |root: ObjectId, records| Dataset {
        schema: schema_id.clone(),
        root: Some(root),
        records,
    };
    validator
        .dataset(&store, &dataset(leaf.clone(), 1))
        .unwrap();
    let wrong_order = node("a", &value("a"), Some(&leaf), None);
    assert!(validator.dataset(&store, &dataset(wrong_order, 2)).is_err());
    let wrong_key = node("wrong", &value("z"), None, None);
    assert!(validator.dataset(&store, &dataset(wrong_key, 1)).is_err());
    let missing = ObjectId::parse(&"0".repeat(64)).unwrap();
    let bad_reference = node("a", &missing, None, None);
    assert!(
        validator
            .dataset(&store, &dataset(bad_reference, 1))
            .is_err()
    );
    // An adversarial store bypasses content hashes to exercise the cycle guard.
    struct Cyclic {
        schema: MemoryStore,
        id: ObjectId,
        bytes: Vec<u8>,
    }
    impl ObjectStore for Cyclic {
        fn put(&self, _: &str, _: &[u8]) -> Result<ObjectId> {
            unreachable!()
        }
        fn get(&self, id: &ObjectId, kind: &str) -> Result<Vec<u8>> {
            if id == &self.id {
                Ok(self.bytes.clone())
            } else {
                self.schema.get(id, kind)
            }
        }
    }
    let bytes = serde_json::to_vec(
        &serde_json::json!({"key":"z","value":value("z"),"left":missing,"right":null}),
    )
    .unwrap();
    let cycle_dataset = dataset(missing.clone(), 1);
    let cyclic = Cyclic {
        schema: store,
        id: missing,
        bytes,
    };
    assert!(
        tree::Validator::default()
            .dataset(&cyclic, &cycle_dataset)
            .is_err()
    );
}
