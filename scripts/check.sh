#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-docs.py
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
# Exercise the engine without features enabled only by the Rust SDK.
cargo test --locked -p geoledger-engine --lib
cargo test --locked --workspace
if [[ -n "${GL_TEST_DATABASE_URL:-}" ]]; then
  cargo test --locked -p geoledger-engine --lib postgis_audit_cursor -- --ignored --test-threads=1
  GL_CONFORMANCE_BACKEND=postgis cargo test --locked -p geoledger-engine --test conformance -- --test-threads=1
  cargo test --locked -p geoledger-engine --test storage -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --test upgrade -- --ignored
else
  printf '%s\n' 'PostGIS integration tests not run: GL_TEST_DATABASE_URL is unset.'
fi
