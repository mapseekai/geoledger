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
    for (application, version) in [(0x474c4433, 1), (0x474c4433, 2), (0x474c4433, 4), (0, 3)] {
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
        assert_eq!(
            current,
            (0x474c4433, i64::from(geoledger_core::FORMAT_VERSION))
        );
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
    use geoledger_core::{FORMAT_VERSION, PendingOperation, RepositoryState};
    use std::collections::BTreeMap;
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let head = repo.put("test/v1", b"head").unwrap();
    let mut state = RepositoryState {
        version: FORMAT_VERSION,
        repository_id: "format-test".into(),
        branch: "main".into(),
        branches: BTreeMap::from([("main".into(), head.clone())]),
        bindings: BTreeMap::new(),
        merging: None,
    };
    repo.save_state(&state).unwrap();
    state.version = FORMAT_VERSION - 1;
    assert!(repo.save_state(&state).is_err());
    assert!(
        repo.prepare(&PendingOperation {
            id: "test".into(),
            before_head: head,
            after: state
        })
        .is_err()
    );
    assert_eq!(repo.state().unwrap().version, FORMAT_VERSION);
    assert!(repo.pending().unwrap().is_none());
}
