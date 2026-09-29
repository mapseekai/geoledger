# Development contract

The repository root is the single Cargo workspace. Implementation crates live
under crates/. Run workspace commands from this directory.

Run ./scripts/check.sh with Python 3 and Node.js 22 available. It checks docs,
browser protocol regressions, rustfmt, locked workspace Clippy and Rust tests.
Opt-in PostGIS tests use GL_TEST_DATABASE_URL and a database named geoledger_test.
Keep fixtures within dedicated disposable test environments.

Core stays independent of databases and transports. Every transport calls Application.
Keep working-copy triggers enabled during checkout and restoration. Preserve table locks,
journal/marker ordering, schema validation, clean-copy checks and conflicting-head protection.

Keep immutable object/schema/PostGIS tracking FORMAT_VERSION distinct from mutable local
STATE_VERSION and SQLite storage layout. Storage migrations preserve object IDs and run
transactionally with migration/rollback tests. Encoding changes receive explicit versions. During development, initialize fresh
repositories and import source tables into dedicated current-format working copies.

Document implemented behavior, operating prerequisites and measured validation scope.
Keep README as the overview, with guides for getting started, daily operations, API,
and development. Keep each topic in one place. Use repository-relative links to tracked
files; identify generated paths and service addresses through their setup commands.
