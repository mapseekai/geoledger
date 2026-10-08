#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Generated network bindings are implementation details, never package-root exports.
protoc -I proto --go_out=sdk/go --go_opt=module=github.com/mapseekai/geoledger/sdk/go --go-grpc_out=sdk/go --go-grpc_opt=module=github.com/mapseekai/geoledger/sdk/go proto/geoledger/v1/geoledger.proto
sdk_generated_dir=$(mktemp -d)
trap 'rm -rf "$sdk_generated_dir"' EXIT
"${PYTHON:-python3}" -m grpc_tools.protoc -I proto --python_out="$sdk_generated_dir" --pyi_out="$sdk_generated_dir" --grpc_python_out="$sdk_generated_dir" proto/geoledger/v1/geoledger.proto
python3 - "$sdk_generated_dir" <<'PY'
from pathlib import Path
import sys
source = Path(sys.argv[1]) / 'geoledger/v1'
target = Path('sdk/python/geoledger/_internal/v1')
target.mkdir(parents=True, exist_ok=True)
for path in source.iterdir():
    text = path.read_text().replace('from geoledger.v1 ', 'from geoledger._internal.v1 ').replace("'geoledger.v1.geoledger_pb2'", "'geoledger._internal.v1.geoledger_pb2'")
    (target / path.name).write_text(text)
PY
protoc -I proto --plugin=protoc-gen-ts_proto=sdk/ts/node_modules/.bin/protoc-gen-ts_proto --ts_proto_out=sdk/ts/src/_internal --ts_proto_opt=outputServices=grpc-js,env=node,forceLong=string,useOptionals=messages,esModuleInterop=true proto/geoledger/v1/geoledger.proto
