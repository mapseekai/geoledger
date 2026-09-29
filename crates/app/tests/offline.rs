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
    assert!(serde_json::from_str::<Command>(r#"{"op":"upgrade"}"#).is_err());
}
