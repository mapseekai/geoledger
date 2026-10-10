#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-docs.py
python3 scripts/check-env.py
python3 scripts/check-release-policy.py
python3 -m unittest discover -s scripts -p test_supply_chain.py
bash -n scripts/backup-drill.sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
# Exercise the engine without features enabled only by the Rust SDK.
cargo test --locked -p geoledger-spatialite --lib
cargo test --locked -p geoledger-engine --lib
cargo test --locked --workspace
./scripts/backup-drill.sh
if [[ -n "${GL_TEST_DATABASE_URL:-}" ]]; then
  cargo test --locked -p geoledger-engine --test review_postgis -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --lib postgis_hot_spatial_queries -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --lib postgis_audit_cursor -- --ignored --test-threads=1
  GL_CONFORMANCE_BACKEND=postgis cargo test --locked -p geoledger-engine --test conformance -- --test-threads=1
  cargo test --locked -p geoledger-engine --test postgis_table -- --ignored
  cargo test --locked -p geoledger-engine --test storage -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --test portable -- --ignored
  cargo test --locked -p geoledger-engine --test multi_instance -- --ignored
else
  printf '%s\n' 'PostGIS integration tests not run: GL_TEST_DATABASE_URL is unset.'
fi
