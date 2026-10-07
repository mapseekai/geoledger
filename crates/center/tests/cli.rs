#![allow(clippy::unwrap_used)]
#![cfg(feature = "http")]
use serde_json::Value;
use std::process::Command;

#[test]
fn version_help_and_token_generation_are_database_independent() {
    let bin = env!("CARGO_BIN_EXE_gl-center");
    for argument in ["--version", "--help"] {
        let result = Command::new(bin)
            .env_remove("GL_CENTER_DATABASE_URL")
            .arg(argument)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("gl-center"));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let result = Command::new(bin)
        .args(["tokens", "--out"])
        .arg(&path)
        .args(["alice", "bob"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let data = std::fs::read(&path).unwrap();
    let entries: Vec<Value> = serde_json::from_slice(&data).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["subject"], "alice");
    for entry in &entries {
        let token = entry["token"].as_str().unwrap();
        assert_eq!(token.len(), 64);
        assert!(!String::from_utf8_lossy(&result.stdout).contains(token));
    }
    assert_ne!(entries[0]["token"], entries[1]["token"]);
    assert!(geoledger_center::Tokens::from_json(&data).is_ok());
    let retry = Command::new(bin)
        .args(["tokens", "--out"])
        .arg(&path)
        .arg("new-user")
        .output()
        .unwrap();
    assert!(!retry.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), data);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn invalid_subjects_do_not_create_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    for subjects in [vec!["alice", "alice"], vec![""]] {
        let result = Command::new(env!("CARGO_BIN_EXE_gl-center"))
            .args(["tokens", "--out"])
            .arg(&path)
            .args(subjects)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!path.exists());
    }
}

#[test]
fn build_metadata_is_embedded_and_independent_of_database_configuration() {
    let result = Command::new(env!("CARGO_BIN_EXE_gl-center"))
        .env_remove("GL_CENTER_DATABASE_URL")
        .env_remove("GL_CENTER_TOKEN_FILE")
        .arg("--build-info")
        .output()
        .unwrap();
    assert!(result.status.success());
    let info: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(info["product"], "geoledger-center");
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert!(info["rustc"].as_str().unwrap().starts_with("rustc "));
    assert!(matches!(
        info["source_status"].as_str(),
        Some("clean" | "dirty" | "unknown")
    ));
    assert_eq!(
        info["c_runtime"],
        if cfg!(target_feature = "crt-static") {
            "static"
        } else {
            "dynamic"
        }
    );
    #[cfg(windows)]
    assert!(info["target"].as_str().unwrap().contains("windows"));
}
