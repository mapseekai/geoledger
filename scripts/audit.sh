#!/usr/bin/env bash
# Dependency advisory gate (needs network). CI runs it in the supply-chain job;
# Every scanner is required. Missing tools, lookup errors and findings fail closed.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo-audit npm govulncheck pip-audit; do
  command -v "$tool" >/dev/null || { printf 'required scanner missing: %s\n' "$tool" >&2; exit 1; }
done
echo '== Rust (RustSec); accepted advisories and reasons: .cargo/audit.toml'
cargo audit --deny warnings
echo '== npm: runtime dependencies (moderate and above), all dependencies (high and above)'
for package in web sdk/ts; do
  npm audit --prefix "$package" --omit=dev --audit-level=moderate
  npm audit --prefix "$package" --audit-level=high
done
echo '== Go (toolchain pinned by sdk/go/go.mod)'
(cd sdk/go && go version && govulncheck ./...)
echo '== Python'
pip-audit --strict ./sdk/python
echo 'All dependency audits passed.'
