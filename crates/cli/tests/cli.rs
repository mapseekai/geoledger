#![allow(clippy::unwrap_used)]
use std::process::{Command, Output};

fn gl() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gl"));
    for key in [
        "GL_DATABASE_URL",
        "GL_AUTHOR",
        "GL_STATEMENT_TIMEOUT_SECS",
        "GL_API_TOKEN",
    ] {
        command.env_remove(key);
    }
    command
}

fn success(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn json(command: &mut Command) -> serde_json::Value {
    serde_json::from_slice(&success(command).stdout).unwrap()
}

#[test]
fn executable_and_help_use_gl_and_geoledger_names() {
    let output = success(gl().arg("--version"));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("gl {}", env!("CARGO_PKG_VERSION"))
    );
    let output = success(gl().arg("--help"));
    let help = String::from_utf8(output.stdout).unwrap();
    for text in [
        "GeoLedger",
        "Usage: gl",
        "GL_AUTHOR",
        "GL_DATABASE_URL",
        "GL_STATEMENT_TIMEOUT_SECS",
    ] {
        assert!(help.contains(text), "missing {text}: {help}");
    }
}

#[test]
fn gl_offline_workflow_uses_new_directory_and_author_environment() {
    let dir = tempfile::tempdir().unwrap();
    let init = json(
        gl().arg("--repo")
            .arg(dir.path())
            .env("GL_AUTHOR", "rename-test")
            .arg("init"),
    );
    assert_eq!(init["branch"], "main");
    assert!(dir.path().join(".geoledger/repository.sqlite").is_file());
    let log = json(gl().arg("--repo").arg(dir.path()).arg("log"));
    assert_eq!(log["commits"][0]["commit"]["author"], "rename-test");
    success(gl().arg("--repo").arg(dir.path()).args(["branch", "draft"]));
    let check = json(gl().arg("--repo").arg(dir.path()).arg("fsck"));
    assert_eq!(check["ok"], true);
}

#[test]
fn gl_timeout_environment_is_validated() {
    let output = gl()
        .env("GL_STATEMENT_TIMEOUT_SECS", "0")
        .arg("status")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
}

#[test]
fn gl_api_token_environment_is_used_before_binding_listeners() {
    let output = gl()
        .env("GL_API_TOKEN", "short")
        .args(["serve", "--http", "0.0.0.0:0", "--grpc", "127.0.0.1:0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("GL_API_TOKEN must contain at least 24 bytes")
    );
}

#[test]
fn gl_defaults_to_mapseekai_and_explicit_author_takes_precedence() {
    for (environment, explicit, expected) in [
        (None, None, "mapseekai"),
        (Some("environment-author"), None, "environment-author"),
        (
            Some("environment-author"),
            Some("flag-author"),
            "flag-author",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut command = gl();
        command.arg("--repo").arg(dir.path());
        if let Some(value) = environment {
            command.env("GL_AUTHOR", value);
        }
        if let Some(value) = explicit {
            command.args(["--author", value]);
        }
        command.arg("init");
        assert_eq!(
            json(&mut command)["format_version"],
            geoledger::core::FORMAT_VERSION
        );
        let history = json(gl().arg("--repo").arg(dir.path()).arg("log"));
        assert_eq!(history["commits"][0]["commit"]["author"], expected);
    }
    let help = String::from_utf8(success(gl().arg("--help")).stdout).unwrap();
    assert!(!help.contains("upgrade"));
    assert!(help.contains("mapseekai"));
}

#[test]
fn help_matches_selected_transport_features() {
    let output = success(gl().arg("--help"));
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("serve"));
    assert_eq!(help.contains("serve-thrift"), cfg!(feature = "thrift"));
}

#[test]
fn repository_paths_accept_unicode_and_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("地图版本 test");
    let init = json(gl().arg("--repo").arg(&path).arg("init"));
    let status = json(gl().arg("--repo").arg(&path).arg("status"));
    assert_eq!(status["head"], init["head"]);
    assert_eq!(status["clean"], true);
    assert_eq!(json(gl().arg("--repo").arg(&path).arg("fsck"))["ok"], true);
}
