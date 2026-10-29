#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-test-layout.py
# These checks are read-only and require no running service. CI uses Linux with
# cgo support; unsupported race-detector environments must report a failure.
go_format_diff=$(gofmt -l sdk/go)
if [[ -n "$go_format_diff" ]]; then
  printf 'Go formatting differences:\n%s\n' "$go_format_diff" >&2
  exit 1
fi
go -C sdk/go vet ./...
go -C sdk/go test -race ./...
: "${GL_ENDPOINT:?Use a disposable test service}"
: "${GL_API_KEY_FILE:?Credential file required}"
export GL_TEST_MEMBER_ID=${GL_TEST_MEMBER_ID:-$(cat "$GL_API_KEY_FILE.member-id")}
PYTHONPATH="$PWD/sdk/python${PYTHONPATH:+:$PYTHONPATH}" "${PYTHON:-python3}" -m unittest discover -s sdk/python/tests -p 'test_*.py'
PYTHONPATH="$PWD/sdk/python${PYTHONPATH:+:$PYTHONPATH}" "${PYTHON:-python3}" sdk/python/tests/smoke.py
cargo run --locked -p geoledger-client --example smoke
npm run build --prefix sdk/ts
npm --prefix sdk/ts test
node sdk/ts/test/smoke.cjs
go -C sdk/go run ./cmd/smoke
