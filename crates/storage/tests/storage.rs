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
