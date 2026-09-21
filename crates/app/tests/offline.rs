#![allow(clippy::unwrap_used)]
use spatial_version::{Application, Command};
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
