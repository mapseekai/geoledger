#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
unset GL_TEST_DATABASE_URL GL_DATABASE_URL GL_STORAGE GL_LOAD_P95_READ_MS GL_LOAD_P95_WRITE_MS GL_LOAD_BATCH GL_MAX_CONCURRENCY
export GL_SPATIALITE_EXTENSION=${GL_SPATIALITE_EXTENSION:-/usr/lib/x86_64-linux-gnu/mod_spatialite.so}
export GL_SERVER_BIN=$PWD/target/release/geoledger-server
export K6=${K6:-k6}
report=${1:?provide a new absolute report directory}
[[ $report == /* && ! -e $report ]] || { echo 'a new absolute report directory is required' >&2; exit 2; }
mkdir -m 700 -p "$report"
python3 - "$report" <<'PYTHON'
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
with (root / 'fixture.jsonl').open('w') as output:
    for i in range(1, 10001):
        output.write(json.dumps({'type': 'Feature', 'id': f'{i:012}', 'properties': {'attributes': {'value': i}}, 'geometry': {'type': 'Point', 'coordinates': [120, 30]}}) + '\n')
# Synthetic public fixture must be readable by the container's postgres user.
(root / 'fixture.jsonl').chmod(0o644)
(root / 'fixture.meta.json').write_text(json.dumps({'features': 10000, 'dimension': 2, 'geometry_type': 'point', 'crs': 'EPSG:4326'}))
PYTHON
container=geoledger-release-load-$(python3 -c 'import uuid; print(uuid.uuid4().hex[:12])')
trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT
docker run --rm -d --name "$container" -e POSTGRES_HOST_AUTH_METHOD=trust -e POSTGRES_DB=geoledger_test -p 127.0.0.1::5432 postgis/postgis@sha256:01a6a70e41e6c4467c8f55f6063555ed72db2d6662cd0d571040d42eadaeb6f6 > "$report/http-container-id"
for attempt in $(seq 1 120); do
  if docker exec "$container" pg_isready -h 127.0.0.1 -U postgres -d geoledger_test >/dev/null 2>&1; then break; fi
  sleep .5
done
port=$(docker port "$container" 5432/tcp | cut -d: -f2)
docker exec "$container" psql -U postgres -d geoledger_test -v ON_ERROR_STOP=1 -c 'CREATE EXTENSION IF NOT EXISTS postgis' > "$report/http-database.log"
set +e
GL_DATABASE_URL="postgresql://postgres@127.0.0.1:$port/geoledger_test?sslmode=disable" GL_DATABASE_ALLOW_PLAINTEXT=true GL_LOAD_VUS=20 GL_LOAD_DURATION=5m GL_LOAD_SUMMARY="$report/load-postgis.json" ./scripts/load-test.sh > "$report/load-postgis.log" 2>&1
load_status=$?
set -e
echo "$load_status" > "$report/load-postgis.exit"
docker logs "$container" > "$report/http-postgres.log" 2>&1
docker stop "$container" >/dev/null
for backend in sqlite postgis; do
  set +e
  python3 scripts/benchmark_incremental.py --server target/release/geoledger-server --client target/release/examples/incremental --fixture "$report/fixture.jsonl" --metadata "$report/fixture.meta.json" --output "$report/incremental-$backend" --backend "$backend" --rounds 10 --load-seconds 300 --load-workers 20 > "$report/incremental-$backend.log" 2>&1
  result=$?
  set -e
  echo "$result" > "$report/incremental-$backend.exit"
done

python3 - "$report" <<'PYTHON'
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
passed = (root / 'load-postgis.exit').read_text().strip() == '0'
for backend in ('sqlite', 'postgis'):
    result = json.loads((root / f'incremental-{backend}/result.json').read_text())
    passed = passed and result['success'] and (root / f'incremental-{backend}.exit').read_text().strip() == '0'
if not passed:
    raise SystemExit('release performance acceptance failed; inspect saved results')
print('PASS PostGIS HTTP load and both incremental soak suites')
PYTHON
