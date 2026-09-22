#![allow(clippy::unwrap_used)]
use spatial_version_core::{Error, ObjectStore};
use spatial_version_storage::Repository;
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
    let committed = repo.put("record/v1", b"hello").unwrap();
    repo.commit().unwrap();
    repo.begin().unwrap();
    let lost = repo.put("record/v1", b"lost").unwrap();
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.get(&committed, "record/v1").unwrap(), b"hello");
    assert!(repo.get(&committed, "schema/v1").is_err());
    assert!(repo.get(&lost, "record/v1").is_err());
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
    let old = repo.put("record/v1", b"existing").unwrap();
    let added = repo
        .bulk_write(|store| {
            // Reject nesting without leaving an extra savepoint or losing the outer batch.
            assert!(repo.bulk_write(|_| Ok(())).is_err());
            assert_eq!(store.get(&old, "record/v1")?, b"existing");
            assert_eq!(store.put("record/v1", b"existing")?, old);
            let added = store.put("record/v1", b"added")?;
            assert_eq!(store.get(&added, "record/v1")?, b"added");
            assert!(store.get(&added, "schema/v2").is_err());
            Ok(added)
        })
        .unwrap();
    let lost = spatial_version_core::object::digest("record/v1", b"lost");
    let failed: spatial_version_core::Result<()> = repo.bulk_write(|store| {
        store.put("record/v1", b"lost")?;
        Err(Error::Invalid("simulated scan error".into()))
    });
    assert!(failed.is_err());
    assert!(repo.get(&lost, "record/v1").is_err());
    // A failed batch can be followed by a successful one in the same transaction.
    repo.bulk_write(|store| store.put("record/v1", b"added"))
        .unwrap();
    assert_eq!(repo.verify_objects().unwrap(), 2);
    repo.commit().unwrap();
    repo.begin().unwrap();
    repo.bulk_write(|store| store.put("record/v1", b"lost"))
        .unwrap();
    repo.rollback().unwrap();
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.get(&added, "record/v1").unwrap(), b"added");
    assert!(repo.get(&lost, "record/v1").is_err());
    assert_eq!(repo.verify_objects().unwrap(), 2);
}
