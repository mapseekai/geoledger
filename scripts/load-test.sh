#!/usr/bin/env bash
# Run the k6 HTTP load profile against a throwaway server and fail on threshold
# breaches, internal errors or a published head that does not match.
#   GL_LOAD_VUS=20 GL_LOAD_DURATION=60s ./scripts/load-test.sh
#   GL_DATABASE_URL=postgresql://... ./scripts/load-test.sh   # PostGIS backend
# GL_SERVER_BIN selects a prebuilt (ideally release) server; K6 selects k6.
set -euo pipefail
cd "$(dirname "$0")/.."
k6=${K6:-k6}
command -v "$k6" >/dev/null || { echo "k6 not found: https://grafana.com/docs/k6/latest/set-up/install-k6/" >&2; exit 2; }
if [[ -n ${GL_SERVER_BIN:-} ]]; then
  server=$GL_SERVER_BIN
else
  cargo build --locked --quiet --release -p geoledger-server
  server=${CARGO_TARGET_DIR:-target}/release/geoledger-server
fi
mkdir -p target
work=$(mktemp -d "$PWD/target/auth-refactor-drill.XXXXXX")
pid=
cleanup() {
  [[ -n $pid ]] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
  rm -rf "$work"
}
trap cleanup EXIT
read -r http grpc < <(python3 -c 'import socket
s=[socket.socket() for _ in range(2)]
[x.bind(("127.0.0.1",0)) for x in s]
print(*[x.getsockname()[1] for x in s])')
base="http://127.0.0.1:$http"
storage=(--storage sqlite)
[[ -n ${GL_DATABASE_URL:-} ]] && storage=(--storage postgis)
env -i HOME="$HOME" PATH="$PATH" RUST_LOG=warn ${GL_DATABASE_URL:+"GL_DATABASE_URL=$GL_DATABASE_URL"} \
  ${GL_SPATIALITE_EXTENSION:+"GL_SPATIALITE_EXTENSION=$GL_SPATIALITE_EXTENSION"} \
  ${GL_DATABASE_ALLOW_PLAINTEXT:+"GL_DATABASE_ALLOW_PLAINTEXT=$GL_DATABASE_ALLOW_PLAINTEXT"} \
  "$server" "${storage[@]}" --data-dir "$work/data" --http "127.0.0.1:$http" --grpc "127.0.0.1:$grpc" \
  --max-concurrency "${GL_MAX_CONCURRENCY:-20}" >"$work/server.log" 2>&1 &
pid=$!
for _ in $(seq 100); do
  curl -fsS "$base/ready" >/dev/null 2>&1 && break
  kill -0 "$pid" 2>/dev/null || { cat "$work/server.log" >&2; exit 1; }
  sleep 0.1
done
python3 scripts/auth_test_support.py --base "$base" --data-dir "$work/data" --output "$work/api-key"
api_key=$(cat "$work/api-key")
(umask 077; printf 'Authorization: Bearer %s\n' "$api_key" > "$work/http-headers")
summary=${GL_LOAD_SUMMARY:-$work/summary.json}
status=0
GL_BASE_URL=$base GL_API_KEY=$api_key "$k6" run --quiet --summary-export "$summary" scripts/load/http.js || status=$?
metrics=$(curl -fsS "$base/metrics" -H "@$work/http-headers")
internal=$(awk '$1=="geoledger_internal_errors_total"{print $2}' <<<"$metrics")
echo "internal errors: ${internal:-missing}"
[[ ${internal:-1} == 0 ]] || { echo "server reported internal errors" >&2; tail -50 "$work/server.log" >&2; status=1; }
# Every publication k6 counted must be on the head, and nothing more.
expected=$(python3 -c 'import json,sys; print(int(json.load(open(sys.argv[1]))["metrics"].get("geoledger_published",{}).get("count",0)))' "$summary")
project=$(curl -fsS "$base/api/v1/list_projects" -H "@$work/http-headers" -H 'Content-Type: application/json' --data '{}' |
  python3 -c 'import json,sys; v=json.load(sys.stdin); v=v.get("projects",v) if isinstance(v,dict) else v; print(max(v,key=lambda p:p["name"])["project"])')
head=$(curl -fsS "$base/api/v1/get_project" -H "@$work/http-headers" -H 'Content-Type: application/json' --data "{\"project\":\"$project\"}" |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["head"])')
echo "published: $expected, head: $head"
[[ $head == "$expected" ]] || { echo "published head does not match successful publications" >&2; status=1; }
exit $status
