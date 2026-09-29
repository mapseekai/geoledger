#![allow(clippy::unwrap_used)]
use geoledger::{Application, Command};
#[test]
fn init_log_branch_fsck_and_reflog_work_without_postgis() {
    let dir = tempfile::tempdir().unwrap();
    let app = Application::new(dir.path());
    let initial = app
        .execute(Command::Init {
            author: "test".into(),
        })
        .unwrap();
    assert_eq!(initial["branch"], "main");
    app.execute(Command::Branch {
        name: "draft".into(),
        from: "HEAD".into(),
    })
    .unwrap();
    let branches = app.execute(Command::Branches).unwrap();
    assert_eq!(branches["branches"]["draft"], initial["head"]);
    assert_eq!(
        app.execute(Command::Status { limit: 10 }).unwrap()["clean"],
        true
    );
    assert_eq!(
        app.execute(Command::Log {
            reference: "HEAD".into(),
            limit: 10
        })
        .unwrap()["commits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(app.execute(Command::Fsck).unwrap()["ok"], true);
    assert_eq!(
        app.execute(Command::Reflog { limit: 10 }).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn repeated_init_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let app = Application::new(dir.path());
    let head = app
        .execute(Command::Init {
            author: "test".into(),
        })
        .unwrap()["head"]
        .clone();
    assert!(
        app.execute(Command::Init {
            author: "other".into()
        })
        .is_err()
    );
    assert_eq!(
        app.execute(Command::Show {
            reference: "HEAD".into(),
            dataset: None,
            key: None
        })
        .unwrap()["id"],
        head
    );
}
#[test]
fn commands_reject_unknown_fields() {
    assert!(
        serde_json::from_str::<Command>(
            r#"{"op":"reset","target":"HEAD","hard":true,"force":true}"#
        )
        .is_err()
    );
}

#[test]
fn every_author_bearing_command_defaults_to_mapseekai() {
    for input in [
        r#"{"op":"init"}"#,
        r#"{"op":"import","dataset":"roads","table":"roads"}"#,
        r#"{"op":"commit","message":"save"}"#,
        r#"{"op":"merge","source":"draft"}"#,
        r#"{"op":"revert","target":"HEAD"}"#,
        r#"{"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"note","data_type":"text"}}"#,
    ] {
        let command: Command = serde_json::from_str(input).unwrap();
        assert_eq!(
            serde_json::to_value(command).unwrap()["author"],
            "mapseekai"
        );
        let mut explicit: serde_json::Value = serde_json::from_str(input).unwrap();
        explicit["author"] = "custom-author".into();
        let command: Command = serde_json::from_value(explicit).unwrap();
        assert_eq!(
            serde_json::to_value(command).unwrap()["author"],
            "custom-author"
        );
    }
}

#[test]
fn fresh_repository_uses_current_format_and_default_author() {
    let dir = tempfile::tempdir().unwrap();
    let app = Application::new(dir.path());
    let result = app
        .execute(serde_json::from_str(r#"{"op":"init"}"#).unwrap())
        .unwrap();
    assert_eq!(result["format_version"], geoledger::core::FORMAT_VERSION);
    let history = app
        .execute(serde_json::from_str(r#"{"op":"log"}"#).unwrap())
        .unwrap();
    assert_eq!(history["commits"][0]["commit"]["author"], "mapseekai");
    assert_eq!(
        history["commits"][0]["commit"]["version"],
        geoledger::core::FORMAT_VERSION
    );
}

mod import_regression {
    use super::*;
    use geoledger::core::{adapter::*, *};
    use std::{
        collections::BTreeMap,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    struct Provider {
        fail: Arc<AtomicBool>,
    }
    struct Transaction {
        fail: bool,
        head: ObjectId,
    }
    fn schema() -> Schema {
        geoledger::core::schema::with_identities(Schema {
            version: FORMAT_VERSION,
            kind: DatasetKind::Table,
            primary_key: "id".into(),
            fields: ["id", "value"]
                .into_iter()
                .map(|name| Field {
                    name: name.into(),
                    logical_type: "text".into(),
                    codec: "text".into(),
                    nullable: false,
                    geometry: false,
                    metadata: BTreeMap::new(),
                })
                .collect(),
            metadata: BTreeMap::new(),
        })
    }
    fn row(key: &str) -> Record {
        Record {
            key: key.into(),
            fields: BTreeMap::from([
                ("id".into(), Cell::Text(key.into())),
                ("value".into(), Cell::Text("initial".into())),
            ]),
        }
    }
    impl WorkingCopyProvider for Provider {
        fn name(&self) -> &'static str {
            "test"
        }
        fn begin(
            &self,
            _: &str,
            head: &ObjectId,
            _: &BTreeMap<String, Binding>,
            _: Option<(&str, &str)>,
        ) -> Result<Box<dyn WorkingCopyTransaction>> {
            Ok(Box::new(Transaction {
                fail: self.fail.load(Ordering::SeqCst),
                head: head.clone(),
            }))
        }
    }
    impl WorkingCopyTransaction for Transaction {
        fn marker(&mut self) -> Result<DatabaseMarker> {
            Ok(DatabaseMarker {
                head: self.head.to_string(),
                operation: None,
            })
        }
        fn inspect(&mut self, _: &str, _: &str) -> Result<Schema> {
            Ok(schema())
        }
        fn register(&mut self, _: &str, _: &Binding) -> Result<()> {
            Ok(())
        }
        fn verify(&mut self, _: &str, _: &Binding) -> Result<()> {
            Ok(())
        }
        fn scan(&mut self, _: &Binding, visit: &mut dyn FnMut(Record) -> Result<()>) -> Result<()> {
            visit(row("a"))?;
            if self.fail {
                return Err(Error::Database("injected scan failure".into()));
            }
            visit(row("b"))
        }
        fn dirty_keys(&mut self, _: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn read(&mut self, _: &Binding, _: &str) -> Result<Option<Record>> {
            unreachable!()
        }
        fn normalize(&mut self, _: &Binding, record: &Record) -> Result<Record> {
            Ok(record.clone())
        }
        fn write(&mut self, _: &Binding, _: &str, _: Option<&Record>) -> Result<()> {
            unreachable!()
        }
        fn clear_dirty(&mut self) -> Result<()> {
            Ok(())
        }
        fn mark(&mut self, _: &str, head: &ObjectId) -> Result<()> {
            self.head = head.clone();
            Ok(())
        }
        fn commit(&mut self) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn import_bulk_capture_preserves_hashes_and_failed_scan_rolls_back() {
        use geoledger::core::{graph, object::MemoryStore, tree};
        use geoledger_storage::Repository;
        let dir = tempfile::tempdir().unwrap();
        let fail = Arc::new(AtomicBool::new(true));
        let app =
            Application::new(dir.path()).with_provider(Arc::new(Provider { fail: fail.clone() }));
        app.execute(Command::Init {
            author: "test".into(),
        })
        .unwrap();
        let (head, count) = {
            let repo = Repository::open(dir.path()).unwrap();
            (
                repo.state().unwrap().head().unwrap().clone(),
                repo.verify_objects().unwrap(),
            )
        };
        let import = || {
            serde_json::from_value(serde_json::json!({"op":"import","dataset":"data","table":"source","author":"test"})).unwrap()
        };
        assert!(app.execute(import()).is_err());
        {
            let repo = Repository::open(dir.path()).unwrap();
            assert_eq!(repo.state().unwrap().head().unwrap(), &head);
            assert_eq!(repo.verify_objects().unwrap(), count);
            assert!(repo.state().unwrap().bindings.is_empty());
            assert!(repo.pending().unwrap().is_none());
        }
        fail.store(false, Ordering::SeqCst);
        assert_eq!(app.execute(import()).unwrap()["records"], 2);
        let repo = Repository::open(dir.path()).unwrap();
        let state = repo.state().unwrap();
        let commit = graph::commit(&repo, state.head().unwrap()).unwrap();
        let snapshot: Snapshot = load(&repo, "snapshot/v3", &commit.root).unwrap();
        let memory = MemoryStore::default();
        let mut builder = tree::BulkBuilder::default();
        for key in ["a", "b"] {
            builder
                .push(
                    &memory,
                    key.into(),
                    save(&memory, "record/v3", &row(key)).unwrap(),
                )
                .unwrap();
        }
        assert_eq!(snapshot["data"].root, builder.finish(&memory).unwrap());
        assert_eq!(
            snapshot["data"].schema,
            geoledger::core::schema::store(&memory, &schema()).unwrap()
        );
    }
    #[test]
    fn indexed_resolution_preserves_merge_and_revert_candidates_and_abort() {
        use geoledger::core::{graph, merge};
        use geoledger_storage::Repository;
        for parent_count in [1, 2] {
            let dir = tempfile::tempdir().unwrap();
            let app = Application::new(dir.path()).with_provider(Arc::new(Provider {
                fail: Arc::new(AtomicBool::new(false)),
            }));
            app.execute(Command::Init {
                author: "test".into(),
            })
            .unwrap();
            app.execute(
                serde_json::from_value(
                    serde_json::json!({"op":"import","dataset":"data","table":"source"}),
                )
                .unwrap(),
            )
            .unwrap();
            let head;
            let candidate;
            let changed = |key: &str, value: &str| {
                let mut r = row(key);
                r.fields.insert("value".into(), Cell::Text(value.into()));
                r
            };
            {
                let repo = Repository::open(dir.path()).unwrap();
                let mut state = repo.state().unwrap();
                head = state.head().unwrap().clone();
                let commit = graph::commit(&repo, &head).unwrap();
                let mut snapshot: Snapshot = load(&repo, "snapshot/v3", &commit.root).unwrap();
                repo.begin().unwrap();
                merge::update(
                    &repo,
                    snapshot.get_mut("data").unwrap(),
                    "b",
                    Some(&changed("b", "candidate")),
                )
                .unwrap();
                candidate = snapshot.clone();
                state.merging = Some(MergeState {
                    base: head.clone(),
                    ours: head.clone(),
                    theirs: head.clone(),
                    parents: vec![head.clone(); parent_count],
                    snapshot,
                    author: "test".into(),
                    message: "candidate".into(),
                });
                repo.insert_conflict(&Conflict {
                    dataset: "data".into(),
                    key: "a".into(),
                    base: Some(row("a")),
                    ours: Some(changed("a", "ours")),
                    theirs: Some(changed("a", "theirs")),
                    fields: vec!["value".into()],
                })
                .unwrap();
                repo.save_state(&state).unwrap();
                repo.commit().unwrap();
            }
            assert_eq!(
                app.execute(Command::Conflicts { limit: 1 }).unwrap()["total"],
                1
            );
            let resolve = |record: Record| Command::Resolve {
                dataset: "data".into(),
                key: "a".into(),
                choice: geoledger::Resolution::Custom,
                record: Some(record),
            };
            assert!(app.execute(resolve(changed("wrong", "custom"))).is_err());
            {
                let repo = Repository::open(dir.path()).unwrap();
                assert_eq!(repo.conflict_count().unwrap(), 1);
                assert_eq!(repo.state().unwrap().merging.unwrap().snapshot, candidate);
            }
            assert_eq!(
                app.execute(resolve(changed("a", "custom"))).unwrap()["remaining_conflicts"],
                0
            );
            {
                let repo = Repository::open(dir.path()).unwrap();
                let state = repo.state().unwrap();
                assert_eq!(state.head().unwrap(), &head);
                let pending = state.merging.unwrap();
                assert_eq!(pending.parents.len(), parent_count);
                assert_eq!(
                    merge::record(&repo, pending.snapshot.get("data"), "a").unwrap(),
                    Some(changed("a", "custom"))
                );
                assert_eq!(
                    merge::record(&repo, pending.snapshot.get("data"), "b").unwrap(),
                    Some(changed("b", "candidate"))
                );
            }
            app.execute(Command::MergeAbort).unwrap();
            let repo = Repository::open(dir.path()).unwrap();
            assert_eq!(repo.state().unwrap().head().unwrap(), &head);
            assert!(repo.state().unwrap().merging.is_none());
            assert_eq!(repo.conflict_count().unwrap(), 0);
        }
    }
}

#[test]
fn stalled_database_releases_repository_lock_without_changing_head() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::Arc,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let app = Application::new(dir.path());
    let initial = app
        .execute(Command::Init {
            author: "test".into(),
        })
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let peer = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut socket, _) = listener.accept()?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut size = [0; 4];
        socket.read_exact(&mut size)?;
        let size = u32::from_be_bytes(size) as usize;
        assert!((8..=4096).contains(&size));
        socket.read_exact(&mut vec![0; size - 4])?;
        socket.write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I")?;
        let mut bytes = [0; 4096];
        loop {
            match socket.read(&mut bytes) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    });
    let provider = geoledger_postgis::PostgisProvider::new(format!(
        "host=127.0.0.1 port={} user=stub dbname=stub sslmode=disable",
        address.port()
    ))
    .with_statement_timeout(Duration::from_millis(10))
    .unwrap();
    let started = Instant::now();
    let error = app
        .clone()
        .with_provider(Arc::new(provider))
        .execute(Command::Import {
            dataset: "rows".into(),
            schema: "public".into(),
            table: "rows".into(),
            author: "test".into(),
            message: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), "database_error");
    assert!(started.elapsed() < Duration::from_secs(4));
    peer.join().unwrap().unwrap();
    let status = app.execute(Command::Status { limit: 1 }).unwrap();
    assert_eq!(status["head"], initial["head"]);
    assert_eq!(status["clean"], true);
    assert!(status["datasets"].as_array().unwrap().is_empty());
}
