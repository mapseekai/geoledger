//! Opt-in PostgreSQL TLS policy checks against real servers.
//!
//! GL_TEST_TLS_DATABASE_URL: a TLS-only server (pg_hba `hostssl`) with `sslrootcert=<CA>`,
//! database geoledger_test. GL_TEST_PLAINTEXT_DATABASE_URL: a server with `ssl=off`.
use geoledger_engine::{Application, Storage};
use std::time::Duration;

fn app(dsn: &str) -> Application {
    Application::new(Storage::Postgis(dsn.to_owned())).with_timeout(Duration::from_secs(10))
}
fn with_param(dsn: &str, param: &str) -> String {
    let sep = if dsn.starts_with("postgres") {
        if dsn.contains('?') { "&" } else { "?" }
    } else {
        " "
    };
    format!("{dsn}{sep}{param}")
}

#[test]
#[ignore = "requires a TLS-only PostgreSQL server and its CA"]
fn verify_full_verify_ca_and_require_connect_over_tls() -> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_TLS_DATABASE_URL")?;
    // Default (no sslmode) is verify-full; the server only accepts encrypted sessions.
    app(&dsn).migrate()?;
    app(&with_param(&dsn, "sslmode=verify-full")).check_schema()?;
    app(&with_param(&dsn, "sslmode=verify-ca")).check_schema()?;
    app(&with_param(&dsn, "sslmode=require")).check_schema()?;
    // A wrong trust anchor is rejected for verifying modes.
    let dir = tempfile::tempdir()?;
    let other = dir.path().join("other-ca.pem");
    std::fs::write(&other, include_str!("fixtures/unrelated-ca.pem"))?;
    let wrong = dsn
        .split(['?', ' ', '&'])
        .find(|p| p.starts_with("sslrootcert="))
        .map(|p| dsn.replace(p, &format!("sslrootcert={}", other.display())))
        .ok_or("GL_TEST_TLS_DATABASE_URL must set sslrootcert")?;
    assert!(app(&wrong).check_schema().is_err(), "untrusted CA accepted");
    // prefer/allow are refused before connecting.
    assert!(
        app(&with_param(&dsn, "sslmode=prefer"))
            .check_schema()
            .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires a PostgreSQL server with ssl=off"]
fn no_silent_downgrade_to_plaintext() -> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_PLAINTEXT_DATABASE_URL")?;
    assert!(
        app(&dsn).check_schema().is_err(),
        "default verify-full downgraded"
    );
    assert!(
        app(&with_param(&dsn, "sslmode=require"))
            .check_schema()
            .is_err()
    );
    // Explicit plaintext stays possible for loopback development servers.
    app(&with_param(&dsn, "sslmode=disable")).migrate()?;
    Ok(())
}
