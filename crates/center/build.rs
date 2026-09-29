#![forbid(unsafe_code)]
use std::{env, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn emit(key: &str, value: &str) {
    println!("cargo:rustc-env=GL_CENTER_BUILD_{key}={value}");
}
fn main() {
    for path in [
        "build.rs",
        "src",
        "web",
        "Cargo.toml",
        "../core/src",
        "../core/Cargo.toml",
        "../../Cargo.toml",
        "../../Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let status = match git(&["status", "--porcelain", "--untracked-files=normal"]) {
        Some(s) if s.is_empty() => "clean",
        Some(_) => "dirty",
        None => "unknown",
    };
    for name in [
        Some("HEAD".into()),
        Some("index".into()),
        git(&["symbolic-ref", "-q", "HEAD"]),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", &name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".into());
    let features = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    let runtime = if features.split(',').any(|f| f == "crt-static") {
        "static"
    } else {
        "dynamic"
    };
    let compiler = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let rustc = Command::new(compiler)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    emit("COMMIT", &commit);
    emit("SOURCE_STATUS", status);
    emit("TARGET", &target);
    emit("PROFILE", &profile);
    emit("C_RUNTIME", runtime);
    emit("RUSTC", &rustc);
}
