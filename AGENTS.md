# Development contract

The repository root is the single Cargo workspace. Implementation crates live
under crates/. Run workspace commands from this directory.

Run python3 scripts/check-docs.py, cargo fmt --all -- --check,
cargo clippy --workspace --all-targets -- -D warnings, and cargo test --workspace.
Opt-in PostGIS tests use GL_TEST_DATABASE_URL and a database named geoledger_test.
Keep fixtures within dedicated disposable test environments.

Core stays independent of databases and transports. Every transport calls Application.
Keep working-copy triggers enabled during checkout and restoration. Preserve table locks,
journal/marker ordering, schema validation, clean-copy checks and conflicting-head protection.

Use the current GeoLedger format for objects, schema, repository state and database tracking.
Encoding changes receive explicit format versions. During development, initialize fresh
repositories and import source tables into dedicated current-format working copies.
All entry points share the default author mapseekai and preserve explicit author overrides.

Document implemented behavior, operating prerequisites and measured validation scope.
Keep planned work in the roadmap. Use repository-relative links to tracked files;
identify generated paths and service addresses through their setup commands.
