#!/usr/bin/env bash
# Dependency advisory gate (needs network). CI runs it in the supply-chain job;
# GL_AUDIT_REQUIRE_ALL=1 turns a missing optional scanner into a failure.
set -euo pipefail
cd "$(dirname "$0")/.."
missing() {
  if [[ -n ${GL_AUDIT_REQUIRE_ALL:-} ]]; then
    printf '%s is required (GL_AUDIT_REQUIRE_ALL is set)\n' "$1" >&2
    exit 1
  fi
  printf 'skipped: %s is not installed\n' "$1" >&2
}
echo '== Rust (RustSec); accepted advisories and reasons: .cargo/audit.toml'
cargo audit --deny warnings
echo '== npm: runtime dependencies (moderate and above), all dependencies (high and above)'
for package in web sdk/ts; do
  npm audit --prefix "$package" --omit=dev --audit-level=moderate
  npm audit --prefix "$package" --audit-level=high
done
echo '== Go'
if command -v govulncheck >/dev/null; then
  (cd sdk/go && govulncheck ./...)
else
  missing govulncheck
fi
echo '== Python'
if command -v pip-audit >/dev/null; then
  pip-audit --strict ./sdk/python
else
  missing pip-audit
fi
echo 'Dependency audit passed.'
