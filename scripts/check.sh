#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-docs.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
if [[ -n "${GL_TEST_DATABASE_URL:-}" ]]; then
  cargo test -p geoledger --test postgis -- --ignored --test-threads=1
  cargo test -p geoledger-server --test thrift -- --ignored --test-threads=1
  cargo test -p geoledger-postgis --lib -- --ignored --test-threads=1
else
  printf '%s\n' 'PostGIS integration tests not run: GL_TEST_DATABASE_URL is unset.'
fi
