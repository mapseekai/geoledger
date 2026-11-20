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
        "fn main() { let app = geoledger_engine::Application::new(geoledger_engine::Storage::Sqlite(\"unused.db\".into())); let _ = app.initialize(); }",
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
    std::fs::write(
        &source,
        "fn main() { let app = geoledger_engine::Application::new(geoledger_engine::Storage::Sqlite(\"unused.db\".into())); let _ = app.migrate(); }",
    )?;
    let removed = check()?;
    let diagnostic = String::from_utf8_lossy(&removed.stderr);
    assert!(!removed.status.success());
    assert!(
        diagnostic.contains("E0599") && diagnostic.contains("no method named `migrate`"),
        "{diagnostic}"
    );
    Ok(())
}

#[test]
fn downstream_backend_must_implement_schema_and_key_only_feature_probes()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = engine.join("../../target/test-fixtures/engine-api");
    std::fs::create_dir_all(&root)?;
    let dir = tempfile::tempdir_in(&root)?;
    std::fs::create_dir(dir.path().join("src"))?;
    std::fs::write(
        dir.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"backend-api-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\ngeoledger-engine = {{ path = {:?} }}\n",
            engine
        ),
    )?;
    std::fs::write(
        dir.path().join("src/main.rs"),
        "mod backend; fn main() { let backend = std::sync::Arc::new(backend::SchemaLessBackend::default()); let _ = backend.modes(); let _ = geoledger_engine::Application::with_backend(backend); }",
    )?;
    let source = dir.path().join("src/backend.rs");
    let fixture = include_str!("support/schema_less_backend.rs");
    let check = || {
        std::process::Command::new(env!("CARGO"))
            .args(["check", "--offline", "--manifest-path"])
            .arg(dir.path().join("Cargo.toml"))
            .arg("--target-dir")
            .arg(engine.join("../../target"))
            .output()
    };
    std::fs::write(&source, fixture)?;
    let positive = check()?;
    assert!(
        positive.status.success(),
        "external backend control compile failed: {}",
        String::from_utf8_lossy(&positive.stderr)
    );

    let required = [
        "feature_exists",
        "schema_projection",
        "schema_at",
        "workspace_schemas",
        "replace_workspace_schemas",
        "published_schema_changes",
        "schema_changed_since",
        "clear_workspace_schemas",
        "stage_schema_merge",
        "restore_workspace_schemas",
    ];
    let mut incomplete = fixture.to_owned();
    for method in required {
        // Remove each real implementation, preserving all other external trait impls.
        let start = incomplete
            .find(&format!("    fn {method}("))
            .ok_or("missing fixture method")?;
        let body = start + incomplete[start..].find('{').ok_or("missing method body")?;
        let mut depth = 0;
        let mut end = None;
        for (offset, character) in incomplete[body..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(body + offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        incomplete.replace_range(start..end.ok_or("unclosed fixture method")?, "");
    }
    std::fs::write(&source, incomplete)?;
    let negative = check()?;
    let diagnostic = String::from_utf8_lossy(&negative.stderr);
    assert!(!negative.status.success());
    assert!(diagnostic.contains("E0046"), "{diagnostic}");
    for method in required {
        assert!(
            diagnostic.contains(method),
            "missing required {method}: {diagnostic}"
        );
    }
    Ok(())
}
