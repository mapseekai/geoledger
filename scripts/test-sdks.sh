#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${GL_ENDPOINT:?Use a disposable test service}"
: "${GL_TOKEN_FILE:?Credential file required}"
PYTHONPATH="$PWD/sdk/python${PYTHONPATH:+:$PYTHONPATH}" "${PYTHON:-python3}" -m unittest discover -s sdk/python/tests -p 'test_*.py'
PYTHONPATH="$PWD/sdk/python${PYTHONPATH:+:$PYTHONPATH}" "${PYTHON:-python3}" sdk/python/tests/smoke.py
cargo run --locked -p geoledger-client --example smoke
npm run build --prefix sdk/ts
node --test sdk/ts/test/client.test.cjs
node sdk/ts/test/smoke.cjs
go -C sdk/go test ./...
go -C sdk/go run ./cmd/smoke
