#![allow(clippy::unwrap_used)]
use std::process::{Command, Output};

fn gl() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gl"));
    for key in [
        "GL_DATABASE_URL",
        "GL_AUTHOR",
        "GL_STATEMENT_TIMEOUT_SECS",
        "GL_API_TOKEN",
        "SV_DATABASE_URL",
        "SV_AUTHOR",
        "SV_STATEMENT_TIMEOUT_SECS",
        "SV_API_TOKEN",
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
    assert!(!help.contains("spatial-version"));
    assert!(!help.contains("SV_"));
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
    assert!(!dir.path().join(".spatial-version").exists());
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
