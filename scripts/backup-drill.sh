#!/usr/bin/env bash
# Backup and restore drill on disposable servers:
# seed data -> online backup + export while serving -> restore into a new data
# directory -> start -> business checks -> verify export -> import -> verify.
# Set GL_DRILL_DATABASE_URL to an empty PostgreSQL database to also import there.
set -euo pipefail
cd "$(dirname "$0")/.."
server=${GL_SERVER_BIN:-target/debug/geoledger-server}
[[ -x $server ]] || cargo build --locked -p geoledger-server
work=$(mktemp -d)
pid=
cleanup() {
  [[ -n $pid ]] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
  [[ -n ${GL_DRILL_KEEP:-} ]] && echo "kept $work" || rm -rf "$work"
}
trap cleanup EXIT
read -r http grpc < <(python3 -c 'import socket
s=[socket.socket() for _ in range(2)]
[x.bind(("127.0.0.1",0)) for x in s]
print(*[x.getsockname()[1] for x in s])')
base="http://127.0.0.1:$http"
run() { env -i HOME="$HOME" PATH="$PATH" RUST_LOG=warn ${GL_DATABASE_URL:+"GL_DATABASE_URL=$GL_DATABASE_URL"} "$server" "$@"; }
start() {
  env -i HOME="$HOME" PATH="$PATH" RUST_LOG=warn "$server" --data-dir "$1" \
    --http "127.0.0.1:$http" --grpc "127.0.0.1:$grpc" "${@:2}" >"$work/server.log" 2>&1 &
  pid=$!
  for _ in $(seq 100); do
    curl -fsS "$base/ready" >/dev/null 2>&1 && return
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  cat "$work/server.log" >&2
  exit 1
}
stop() {
  kill "$pid"
  wait "$pid" || true
  pid=
}
call() {
  curl -fsS "$base/api/v1/$1" -H "Authorization: Bearer $token" -H 'Content-Type: application/json' --data "$2"
}
field() { python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"; }

echo "== seed (SQLite, $base)"
start "$work/live"
token=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print((d[0] if isinstance(d,list) else d)["token"])' "$work/live/admin-credentials.json")
project=$(call create_project '{"name":"drill"}' | field project)
dataset=$(call create_dataset "{\"project\":\"$project\",\"name\":\"roads\"}" | field dataset)
for i in 1 2 3; do
  ws=$(call create_workspace "{\"project\":\"$project\"}" | field workspace)
  call save "{\"project\":\"$project\",\"workspace\":\"$ws\",\"expected_workspace_version\":0,\"edits\":[{\"dataset\":\"$dataset\",\"feature_id\":\"road-$i\",\"feature\":{\"type\":\"Feature\",\"id\":\"road-$i\",\"properties\":{\"exact\":18446744073709551615},\"geometry\":{\"type\":\"Point\",\"coordinates\":[120,30]}}}]}" >/dev/null
  call publish "{\"project\":\"$project\",\"workspace\":\"$ws\",\"expected_workspace_version\":1,\"request_id\":\"$(python3 -c 'import uuid; print(uuid.uuid4())')\",\"message\":\"drill $i\"}" >/dev/null
done
head=$(call get_project "{\"project\":\"$project\"}" | field head)
audit=$(call audit "{\"project\":\"$project\",\"limit\":1000}" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["events"]))')

echo "== online backup and export while serving"
run --data-dir "$work/live" backup --output "$work/backup.sqlite3" >/dev/null
run --data-dir "$work/live" export --output "$work/export.jsonl" >/dev/null
if run --data-dir "$work/live" export --output "$work/export.jsonl" >/dev/null 2>&1; then
  echo "export overwrote an existing file" >&2
  exit 1
fi
# Writes after the backup are outside it (RPO); verify must notice the drift.
call set_member "{\"project\":\"$project\",\"subject\":\"late-member\",\"role\":\"viewer\"}" >/dev/null
stop
status=0
run --data-dir "$work/live" verify --input "$work/export.jsonl" >/dev/null 2>&1 || status=$?
[[ $status == 4 ]] || { echo "verify did not report drift (exit $status)" >&2; exit 1; }

echo "== restore into a new data directory"
run --data-dir "$work/restored" restore --input "$work/backup.sqlite3" >/dev/null
if run --data-dir "$work/restored" restore --input "$work/backup.sqlite3" >/dev/null 2>&1; then
  echo "restore overwrote an existing database" >&2
  exit 1
fi
start "$work/restored" --token-file "$work/live/tokens.json"
[[ $(call get_project "{\"project\":\"$project\"}" | field head) == "$head" ]]
[[ $(call audit "{\"project\":\"$project\",\"limit\":1000}" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["events"]))') == "$audit" ]]
call features "{\"project\":\"$project\",\"dataset\":\"$dataset\",\"feature_id\":\"road-2\"}" | grep -q 18446744073709551615
stop
run --data-dir "$work/restored" verify --input "$work/export.jsonl" >/dev/null

echo "== import the export into a new database"
run --data-dir "$work/imported" import --input "$work/export.jsonl" >/dev/null
run --data-dir "$work/imported" verify --input "$work/export.jsonl" >/dev/null
if [[ -n ${GL_DRILL_DATABASE_URL:-} ]]; then
  echo "== import into PostgreSQL"
  GL_DATABASE_URL=$GL_DRILL_DATABASE_URL run --storage postgis --data-dir "$work/pg" import --input "$work/export.jsonl" >/dev/null
  GL_DATABASE_URL=$GL_DRILL_DATABASE_URL run --storage postgis --data-dir "$work/pg" verify --input "$work/export.jsonl" >/dev/null
fi
echo "Backup drill passed: head r$head, $audit audit events preserved through backup/restore and export/import."
