#![allow(clippy::unwrap_used)]
use geoledger_core::{Error, ObjectStore};
use geoledger_storage::Repository;
#[test]
fn lock_is_cross_instance_and_released_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    assert!(matches!(Repository::open(dir.path()), Err(Error::Busy)));
    drop(repo);
    Repository::open(dir.path()).unwrap();
}
#[test]
fn objects_are_durable_typed_and_transactional() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    repo.begin().unwrap();
    let committed = repo.put("record/v3", b"hello").unwrap();
    repo.commit().unwrap();
    repo.begin().unwrap();
    let lost = repo.put("record/v3", b"lost").unwrap();
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.get(&committed, "record/v3").unwrap(), b"hello");
    assert!(repo.get(&committed, "schema/v3").is_err());
    assert!(repo.get(&lost, "record/v3").is_err());
    assert_eq!(repo.verify_objects().unwrap(), 1);
}

#[test]
fn reused_codecs_still_detect_corrupt_payloads_and_handle_empty_objects() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let empty = repo.put("test/v1", b"").unwrap();
    assert_eq!(repo.get(&empty, "test/v1").unwrap(), b"");
    let id = repo.put("test/v1", b"original bytes").unwrap();
    assert_eq!(repo.get(&id, "test/v1").unwrap(), b"original bytes");
    let conn = rusqlite::Connection::open(repo.directory.join("repository.sqlite")).unwrap();
    conn.execute(
        "UPDATE objects SET payload=?1 WHERE id=?2",
        rusqlite::params![
            zstd::bulk::compress(b"altered bytes", 3).unwrap(),
            id.as_str()
        ],
    )
    .unwrap();
    assert!(repo.get(&id, "test/v1").is_err());
    assert!(repo.verify_objects().is_err());
    conn.execute(
        "UPDATE objects SET payload=x'0000' WHERE id=?1",
        [id.as_str()],
    )
    .unwrap();
    assert!(repo.get(&id, "test/v1").is_err());
}

#[test]
fn staged_bulk_objects_are_readable_deduplicated_and_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    assert!(repo.bulk_write(|_| Ok(())).is_err());
    repo.begin().unwrap();
    let old = repo.put("record/v3", b"existing").unwrap();
    let added = repo
        .bulk_write(|store| {
            // Reject nesting without leaving an extra savepoint or losing the outer batch.
            assert!(repo.bulk_write(|_| Ok(())).is_err());
            assert_eq!(store.get(&old, "record/v3")?, b"existing");
            assert_eq!(store.put("record/v3", b"existing")?, old);
            let added = store.put("record/v3", b"added")?;
            assert_eq!(store.get(&added, "record/v3")?, b"added");
            assert!(store.get(&added, "schema/v3").is_err());
            Ok(added)
        })
        .unwrap();
    let lost = geoledger_core::object::digest("record/v3", b"lost");
    let failed: geoledger_core::Result<()> = repo.bulk_write(|store| {
        store.put("record/v3", b"lost")?;
        Err(Error::Invalid("simulated scan error".into()))
    });
    assert!(failed.is_err());
    assert!(repo.get(&lost, "record/v3").is_err());
    // A failed batch can be followed by a successful one in the same transaction.
    repo.bulk_write(|store| store.put("record/v3", b"added"))
        .unwrap();
    assert_eq!(repo.verify_objects().unwrap(), 2);
    repo.commit().unwrap();
    repo.begin().unwrap();
    repo.bulk_write(|store| store.put("record/v3", b"lost"))
        .unwrap();
    repo.rollback().unwrap();
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.get(&added, "record/v3").unwrap(), b"added");
    assert!(repo.get(&lost, "record/v3").is_err());
    assert_eq!(repo.verify_objects().unwrap(), 2);
}

#[test]
fn new_repositories_use_geoledger_directory() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    assert_eq!(repo.directory, dir.path().join(".geoledger"));
    assert!(repo.directory.join("repository.sqlite").is_file());
}

#[test]
fn opening_a_missing_repository_has_no_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        Repository::open(dir.path()),
        Err(Error::NotFound(_))
    ));
    assert!(!dir.path().join(".geoledger").exists());
}

#[test]
fn current_repository_identity_and_version_are_required() {
    for (application, version) in [(0x474c4433, 1), (0x474c4433, 2), (0x474c4433, 5), (0, 3)] {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let path = repo.directory.join("repository.sqlite");
        drop(repo);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let current: (i64, i64) = (
            conn.query_row("PRAGMA application_id", [], |row| row.get(0))
                .unwrap(),
            conn.query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap(),
        );
        assert_eq!(current, (0x474c4433, 4));
        conn.execute_batch(&format!(
            "PRAGMA application_id={application}; PRAGMA user_version={version};"
        ))
        .unwrap();
        drop(conn);
        assert!(matches!(
            Repository::open(dir.path()),
            Err(Error::Unsupported(_))
        ));
    }
}

#[test]
fn repository_state_and_journal_require_current_format() {
    use geoledger_core::{PendingOperation, RepositoryState, STATE_VERSION};
    use std::collections::BTreeMap;
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let head = repo.put("test/v1", b"head").unwrap();
    let mut state = RepositoryState {
        version: STATE_VERSION,
        repository_id: "format-test".into(),
        branch: "main".into(),
        branches: BTreeMap::from([("main".into(), head.clone())]),
        bindings: BTreeMap::new(),
        merging: None,
    };
    repo.save_state(&state).unwrap();
    state.version = STATE_VERSION - 1;
    assert!(repo.save_state(&state).is_err());
    assert!(
        repo.prepare(&PendingOperation {
            id: "test".into(),
            before_head: head,
            after: state
        })
        .is_err()
    );
    assert_eq!(repo.state().unwrap().version, STATE_VERSION);
    assert!(repo.pending().unwrap().is_none());
}

fn merge_state(repo: &Repository) -> geoledger_core::RepositoryState {
    use geoledger_core::*;
    let head = repo.put("test/v1", b"history-must-survive").unwrap();
    RepositoryState {
        version: STATE_VERSION,
        repository_id: "migration".into(),
        branch: "main".into(),
        branches: std::collections::BTreeMap::from([("main".into(), head.clone())]),
        bindings: Default::default(),
        merging: Some(MergeState {
            base: head.clone(),
            ours: head.clone(),
            theirs: head.clone(),
            parents: vec![head],
            snapshot: Default::default(),
            conflicts: vec![],
            author: "test".into(),
            message: "merge".into(),
        }),
    }
}
fn conflict(key: &str) -> geoledger_core::Conflict {
    use geoledger_core::*;
    let r = |value: &str| {
        Some(Record {
            key: key.into(),
            fields: std::collections::BTreeMap::from([(
                "value".into(),
                Cell::Text(value.repeat(100_000)),
            )]),
        })
    };
    Conflict {
        dataset: "data".into(),
        key: key.into(),
        base: r("b"),
        ours: r("o"),
        theirs: r("t"),
        fields: vec!["value".into()],
    }
}

#[test]
fn v3_conflicts_upgrade_transactionally_without_changing_history() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let mut state = merge_state(&repo);
    state.version = 3;
    let head = state.head().unwrap().clone();
    state.merging.as_mut().unwrap().conflicts = vec![conflict("b"), conflict("a")];
    let path = repo.directory.join("repository.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO metadata VALUES('state',?1)",
        [serde_json::to_string(&state).unwrap()],
    )
    .unwrap();
    conn.execute_batch("DROP TABLE conflicts; DROP TABLE conflict_stats; PRAGMA user_version=3;")
        .unwrap();
    drop(conn);
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.get(&head, "test/v1").unwrap(), b"history-must-survive");
    assert_eq!(repo.state().unwrap().head().unwrap(), &head);
    assert!(repo.state().unwrap().merging.unwrap().conflicts.is_empty());
    assert_eq!(repo.conflict_count().unwrap(), 2);
    assert_eq!(repo.conflicts_page(1, 0).unwrap()[0].key, "a");
    assert_eq!(repo.conflicts_page(1, 1).unwrap()[0].key, "b");
    assert_eq!(
        serde_json::to_value(repo.conflict("data", "a").unwrap()).unwrap(),
        serde_json::to_value(conflict("a")).unwrap()
    );
    repo.begin().unwrap();
    repo.remove_conflict("data", "a").unwrap();
    assert_eq!(repo.conflict_count().unwrap(), 1);
    repo.rollback().unwrap();
    assert_eq!(repo.conflict_count().unwrap(), 2);
    repo.begin().unwrap();
    repo.remove_conflict("data", "a").unwrap();
    repo.commit().unwrap();
    assert_eq!(repo.conflict_count().unwrap(), 1);
    let conn = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    let metadata: String = conn
        .query_row("SELECT value FROM metadata WHERE key='state'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(metadata.len() < 2000);
    conn.execute(
        "UPDATE metadata SET value='null' WHERE key='conflict_owner'",
        [],
    )
    .unwrap();
    assert!(repo.state().is_err());
}

#[test]
fn failed_upgrade_rolls_back_schema_objects_and_version() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let mut state = merge_state(&repo);
    state.version = 3;
    // Duplicate keys fail after the first record objects were inserted.
    state.merging.as_mut().unwrap().conflicts = vec![conflict("a"), conflict("a")];
    let path = repo.directory.join("repository.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO metadata VALUES('state',?1)",
        [serde_json::to_string(&state).unwrap()],
    )
    .unwrap();
    conn.execute_batch("DROP TABLE conflicts; DROP TABLE conflict_stats; PRAGMA user_version=3;")
        .unwrap();
    drop(repo);
    assert!(Repository::open(dir.path()).is_err());
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM objects", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='conflicts'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let original: String = conn
        .query_row("SELECT value FROM metadata WHERE key='state'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(original, serde_json::to_string(&state).unwrap());
}

#[test]
fn corrupt_metadata_is_storage_error_with_json_source() {
    use std::error::Error as _;
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let conn = rusqlite::Connection::open(repo.directory.join("repository.sqlite")).unwrap();
    conn.execute("INSERT INTO metadata VALUES('state','{')", [])
        .unwrap();
    let error = repo.state().unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(error.source().unwrap().is::<serde_json::Error>());
}

#[test]
fn pending_v3_journal_blocks_upgrade_without_mutation() {
    use geoledger_core::PendingOperation;
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let mut state = merge_state(&repo);
    state.merging = None;
    repo.save_state(&state).unwrap();
    repo.prepare(&PendingOperation {
        id: "unfinished".into(),
        before_head: state.head().unwrap().clone(),
        after: state,
    })
    .unwrap();
    let conn = rusqlite::Connection::open(repo.directory.join("repository.sqlite")).unwrap();
    conn.execute_batch("DROP TABLE conflicts; DROP TABLE conflict_stats; PRAGMA user_version=3;")
        .unwrap();
    drop(repo);
    assert!(matches!(
        Repository::open(dir.path()),
        Err(Error::Recovery(_))
    ));
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM metadata WHERE key='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='conflicts'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn conflict_page_does_not_decode_unrequested_entries_and_checks_identity() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let state = merge_state(&repo);
    repo.begin().unwrap();
    repo.insert_conflict(&conflict("a")).unwrap();
    repo.insert_conflict(&conflict("b")).unwrap();
    repo.save_state(&state).unwrap();
    repo.commit().unwrap();
    let conn = rusqlite::Connection::open(repo.directory.join("repository.sqlite")).unwrap();
    conn.execute("UPDATE conflicts SET refs='{' WHERE key='b'", [])
        .unwrap();
    assert_eq!(repo.conflicts_page(1, 0).unwrap()[0].key, "a");
    assert_eq!(repo.conflict_count().unwrap(), 2);
    assert_eq!(
        repo.conflicts_page(1, 1).unwrap_err().code(),
        "storage_error"
    );
    assert!(repo.verify_objects().is_err());
    conn.execute("UPDATE conflict_stats SET count=0", [])
        .unwrap();
    assert!(repo.verify_objects().is_err());
}

#[test]
fn normal_v3_history_upgrade_preserves_commit_ids_and_reflog() {
    use geoledger_core::{Commit, FORMAT_VERSION, STATE_VERSION, Snapshot, graph, save};
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let root = save(&repo, "snapshot/v3", &Snapshot::new()).unwrap();
    let first = save(
        &repo,
        "commit/v3",
        &Commit {
            version: FORMAT_VERSION,
            parents: vec![],
            root: root.clone(),
            author: "test".into(),
            message: "first".into(),
            timestamp: "fixed".into(),
        },
    )
    .unwrap();
    let second = save(
        &repo,
        "commit/v3",
        &Commit {
            version: FORMAT_VERSION,
            parents: vec![first.clone()],
            root,
            author: "test".into(),
            message: "second".into(),
            timestamp: "fixed".into(),
        },
    )
    .unwrap();
    let mut state = merge_state(&repo);
    state.merging = None;
    state.branches.insert("main".into(), second.clone());
    repo.save_state(&state).unwrap();
    let reflog = repo.reflog(100).unwrap();
    let objects = repo.verify_objects().unwrap();
    state.version = 3;
    let conn = rusqlite::Connection::open(repo.directory.join("repository.sqlite")).unwrap();
    conn.execute(
        "UPDATE metadata SET value=?1 WHERE key='state'",
        [serde_json::to_string(&state).unwrap()],
    )
    .unwrap();
    conn.execute_batch("DROP TABLE conflicts; DROP TABLE conflict_stats; PRAGMA user_version=3;")
        .unwrap();
    drop(conn);
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.state().unwrap().version, STATE_VERSION);
    assert_eq!(repo.state().unwrap().head().unwrap(), &second);
    assert_eq!(graph::commit(&repo, &second).unwrap().parents, vec![first]);
    assert_eq!(graph::ancestors(&repo, &second).unwrap().len(), 2);
    assert_eq!(repo.reflog(100).unwrap(), reflog);
    assert_eq!(repo.verify_objects().unwrap(), objects);
}
