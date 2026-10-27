#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-test-layout.py
python3 -m unittest discover -s scripts -p test_test_layout.py
ast-grep test -c sgconfig.yml
python3 scripts/check-docs.py
python3 scripts/check-env.py
python3 scripts/test_container.py
python3 scripts/test_deployment_config.py
python3 scripts/check-release-policy.py
python3 -m unittest discover -s scripts -p test_supply_chain.py
python3 -m unittest discover -s scripts -p test_source_release.py
python3 -m unittest discover -s scripts -p test_sdk_supplement.py
python3 -m unittest discover -s scripts -p 'test_benchmark*.py'
bash -n scripts/backup-drill.sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
# Verify the default downstream API separately from test-support feature unification.
cargo test --locked -p geoledger-engine --test public_api
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
  cargo test --locked -p geoledger-engine --test authentication -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --test postgis_table -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --test schema -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --test storage -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --test portable -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --test multi_instance -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --lib postgis_capabilities_share_commit_error_and_drop_rollback -- --ignored --test-threads=1
  cargo test --locked -p geoledger-engine --lib postgis_dataset_tables -- --ignored --test-threads=1
else
  printf '%s\n' 'PostGIS integration tests not run: GL_TEST_DATABASE_URL is unset.'
fi
