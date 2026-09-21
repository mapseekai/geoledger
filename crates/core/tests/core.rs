#![allow(clippy::unwrap_used)]
use spatial_version_core::{merge::merge_record, object::MemoryStore, tree, *};
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
    let value = save(&store, "record/v1", &row("0", "0")).unwrap();
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
    let value = save(&store, "record/v1", &row("0", "0")).unwrap();
    let mut b = tree::BulkBuilder::default();
    for i in 0..10_000 {
        b.push(&store, format!("{i:05}"), value.clone()).unwrap();
    }
    let root = b.finish(&store).unwrap();
    let count = store.object_count();
    let v2 = save(&store, "record/v1", &row("1", "0")).unwrap();
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
    let value = save(&store, "record/v1", &row("0", "0")).unwrap();
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
        store.put("record/v1", b"{}").unwrap(),
        store.put("schema/v1", b"{}").unwrap()
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
