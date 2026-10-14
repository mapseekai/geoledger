//! Compile a downstream default dependency separately from dev-feature unification.
#[test]
fn default_build_has_no_subject_execution_entry() -> Result<(), Box<dyn std::error::Error>> {
    let engine = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = engine.join("../../target/test-fixtures/engine-api");
    std::fs::create_dir_all(&root)?;
    let dir = tempfile::tempdir_in(&root)?;
    std::fs::create_dir(dir.path().join("src"))?;
    std::fs::write(
        dir.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"default-api-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\ngeoledger-engine = {{ path = {:?} }}\n",
            engine
        ),
    )?;
    let source = dir.path().join("src/main.rs");
    let check = || {
        std::process::Command::new(env!("CARGO"))
            .args(["check", "--offline", "--manifest-path"])
            .arg(dir.path().join("Cargo.toml"))
            .arg("--target-dir")
            .arg(engine.join("../../target"))
            .output()
    };
    std::fs::write(
        &source,
        "fn main() { let _ = geoledger_engine::Application::new(geoledger_engine::Storage::Sqlite(\"unused.db\".into())); }",
    )?;
    let positive = check()?;
    assert!(
        positive.status.success(),
        "control compile failed: {}",
        String::from_utf8_lossy(&positive.stderr)
    );
    std::fs::write(
        &source,
        "fn main() { let app = geoledger_engine::Application::new(geoledger_engine::Storage::Sqlite(\"unused.db\".into())); let _ = app.execute(\"forged\", \"list_projects\", Default::default()); }",
    )?;
    let negative = check()?;
    let diagnostic = String::from_utf8_lossy(&negative.stderr);
    assert!(!negative.status.success());
    assert!(
        diagnostic.contains("E0599") && diagnostic.contains("no method named `execute`"),
        "{diagnostic}"
    );
    Ok(())
}
