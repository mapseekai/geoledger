# Development boundaries

This repository root is the single Cargo workspace. All implementation crates
live under crates/. Run workspace commands from this directory.

Run cargo fmt --all -- --check, cargo clippy --workspace --all-targets -- -D warnings,
and cargo test --workspace. Opt-in PostGIS tests require GL_TEST_DATABASE_URL and
refuse databases not named geoledger_test. Never run those fixtures on business data.

Keep core independent of databases/transports. Every transport must call Application.
Do not disable working-copy triggers to implement checkout. Preserve the journal/marker
protocol and fail closed on unsupported schema, conflicting heads or dirty working copies.
Encoding changes require a new object/schema/repository format version and migration plan.
Do not claim production readiness or unmeasured large-data performance.
