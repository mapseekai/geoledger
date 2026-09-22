#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
if [[ -n "${SV_TEST_DATABASE_URL:-}" ]]; then
  cargo test -p spatial-version --test postgis -- --ignored --test-threads=1
  cargo test -p spatial-version-server --test thrift -- --ignored --test-threads=1
  cargo test -p spatial-version-postgis --lib -- --ignored --test-threads=1
else
  printf '%s\n' 'PostGIS integration tests not run: SV_TEST_DATABASE_URL is unset.'
fi
