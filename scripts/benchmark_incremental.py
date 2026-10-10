#!/usr/bin/env python3
"""Disposable incremental CRUD benchmark; SQL only initializes untimed test fixtures.

The measured Rust client uses RPC exclusively. Each run creates fresh server storage
and, for PostGIS, its own container and business table. Never use production data dirs.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import sys
import time
import traceback
import urllib.error
import urllib.request
import uuid

from benchmark import fingerprint, monitor, native_leaks, ports, stop


def compact(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def collection(geometry):
    """Match the service's SpatiaLite workaround for multipart XYZ geometries."""
    parts = []
    def visit(value):
        kind = value["type"]
        if kind == "GeometryCollection":
            for item in value["geometries"]:
                visit(item)
        elif kind.startswith("Multi"):
            for coordinates in value["coordinates"]:
                visit({"type": kind[5:], "coordinates": coordinates})
        elif value.get("coordinates"):
            parts.append({"type": kind, "coordinates": value["coordinates"]})
    visit(geometry)
    return compact({"type": "GeometryCollection", "geometries": parts})


def rows(path):
    with path.open() as source:
        for index, line in enumerate(source, 1):
            value = json.loads(line)
            if value.get("type") != "Feature" or value.get("id") != f"{index:012}" or not isinstance(value.get("properties", {}).get("attributes"), dict):
                raise ValueError(f"invalid fixture row {index}")
            yield value


def sqlite_seed(path, fixture, project, dataset, revision, dimension, extension):
    with sqlite3.connect(path) as db:
        db.enable_load_extension(True)
        db.load_extension(extension)
        db.enable_load_extension(False)
        db.execute("PRAGMA foreign_keys=ON")
        column = "geom_z" if dimension == 3 else "geom"
        sql = f"INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,{column}) VALUES(?,?,?,?,?,?,coalesce(SetSRID(GeomFromGeoJSON(?),4326),SetSRID(GeomFromGeoJSON(?),4326)))"
        count = 0
        for feature in rows(fixture):
            count += 1
            if count == 1:
                continue  # The API published the first row to establish a real baseline revision.
            geometry = compact(feature["geometry"])
            db.execute(sql, (project, dataset, feature["id"], revision, compact(feature["properties"]), geometry, geometry, collection(feature["geometry"])))
        bad = db.execute(f"SELECT count(*) FROM gl_history WHERE project=? AND dataset=? AND {column} IS NULL", (project, dataset)).fetchone()[0]
        if bad:
            raise ValueError(f"fixture contains {bad} unrepresented native geometries")
        db.execute("ANALYZE")
    return count



def verify_source_crud(call, psql, project, dataset, first, batch_sizes=(1, 100, 500)):
    """Check every source row after each untimed CRUD publication, including batches."""
    for size in batch_sizes:
        prefix = "source-check-" + uuid.uuid4().hex + "-"
        features = []
        for index in range(size):
            feature = json.loads(compact(first))
            feature["id"] = prefix + str(index)
            feature["properties"]["attributes"]["source_check_row"] = index
            features.append(feature)

        def publish(values):
            workspace = call("create_workspace", {"project": project})["workspace"]
            edits = [{"dataset": dataset, "feature_id": feature["id"], "feature": value}
                     for feature, value in zip(features, values, strict=True)]
            version = 0
            for offset in range(0, size, 100):
                saved = call("save", {"project": project, "workspace": workspace,
                                     "expected_workspace_version": version,
                                     "edits": edits[offset:offset + 100]})
                version = saved["version"]
            receipt = call("publish", {"project": project, "workspace": workspace,
                                       "expected_workspace_version": version,
                                       "request_id": str(uuid.uuid4()), "message": "untimed source verification"})
            if receipt["changes"] != size:
                raise AssertionError(f"source verification expected {size} published changes")

        def verify(expected):
            # Only locally generated UUID-prefixed identifiers enter this SQL.
            raw = psql(f"SELECT json_build_object('type','Feature','id',id,'properties',json_build_object('attributes',attributes),'geometry',ST_AsGeoJSON(geom,17,0)::json)::text FROM business.source WHERE id LIKE '{prefix}%' ORDER BY id")
            actual = [json.loads(line) for line in raw.splitlines() if line]
            if actual != sorted(expected, key=lambda feature: feature["id"]):
                raise AssertionError(f"published source rows differ from expected {len(expected)} features in batch {size}")

        publish(features)
        verify(features)
        for feature in features:
            feature["properties"]["attributes"]["incremental_source_verification"] = prefix

            def translate(coordinates):
                if coordinates and isinstance(coordinates[0], (int, float)):
                    for axis, bound in enumerate([180.0, 90.0]):
                        coordinates[axis] += -1e-7 if coordinates[axis] > bound - 1e-7 else 1e-7
                else:
                    for item in coordinates:
                        translate(item)
            translate(feature["geometry"]["coordinates"])
        publish(features)
        verify(features)
        publish([None] * size)
        verify([])
    return {"source_crud_verified": True, "source_verification_timed": False,
            "source_verification_batch_sizes": list(batch_sizes),
            "source_verification_operations": ["add", "update_properties_and_geometry", "delete"]}


def self_test_source_crud():
    first = {"type": "Feature", "id": "original", "properties": {"attributes": {"n": 18446744073709551615}},
             "geometry": {"type": "Point", "coordinates": [1, 2, 3]}}
    original = compact(first)
    faults = [None, "no_writes", "no_update", "no_delete", "no_geometry_update", "rounded_integer",
              "first_row_only_add", "first_row_only_update", "first_row_only_delete"]
    for fault in faults:
        pending, source = [], {}
        publications = 0
        def call(action, body):
            nonlocal pending, publications
            if action == "create_workspace":
                pending = []
                return {"workspace": "w"}
            if action == "save":
                assert 1 <= len(body["edits"]) <= 100
                pending.extend(json.loads(compact(body["edits"])))
                return {"version": body["expected_workspace_version"] + 1}
            assert action == "publish"
            publications += 1
            operation = (publications - 1) % 3
            skip = fault == "no_writes" or (fault == "no_update" and operation == 1) or (fault == "no_delete" and operation == 2)
            edits = pending
            if fault == ["first_row_only_add", "first_row_only_update", "first_row_only_delete"][operation]:
                edits = pending[:1]
            if not skip:
                for edit in edits:
                    key, value = edit["feature_id"], edit["feature"]
                    if value is None:
                        source.pop(key, None)
                    else:
                        if fault == "no_geometry_update" and operation == 1:
                            value["geometry"] = source[key]["geometry"]
                        source[key] = value
            return {"changes": len(pending)}
        def psql(sql):
            assert "ST_AsGeoJSON(geom,17,0)" in sql
            values = json.loads(compact(sorted(source.values(), key=lambda feature: feature["id"])))
            if fault == "rounded_integer":
                for value in values:
                    value["properties"]["attributes"]["n"] = float(value["properties"]["attributes"]["n"])
            return "\n".join(compact(value) for value in values)
        try:
            report = verify_source_crud(call, psql, "p", "d", first)
        except AssertionError:
            if fault is None:
                raise
        else:
            assert fault is None, f"missed source corruption: {fault}"
            assert report["source_crud_verified"] and not report["source_verification_timed"]
            assert report["source_verification_batch_sizes"] == [1, 100, 500]
            assert publications == 9 and not source
        assert compact(first) == original


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["server", "client", "fixture", "metadata", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--backend", choices=["sqlite", "postgis"], default="sqlite")
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--timeout", type=int, default=1200)
    parser.add_argument("--rss-limit-mib", type=int, default=4096)
    parser.add_argument("--k6", type=Path)
    args = parser.parse_args()
    if not 3 <= args.rounds <= 100 or args.timeout <= 0 or args.rss_limit_mib <= 0:
        parser.error("rounds must be 3..100; timeout and RSS ceiling must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    metadata = json.loads(args.metadata.read_text())
    dimension = metadata["dimension"]
    family = metadata["geometry_type"]
    if dimension not in [2, 3] or family not in ["point", "line", "polygon"] or metadata["features"] < 1:
        raise ValueError("invalid fixture metadata")
    first = next(rows(args.fixture))
    kind = first["geometry"]["type"]
    if kind not in ["Point", "MultiPoint", "LineString", "MultiLineString", "Polygon", "MultiPolygon"]:
        raise ValueError("typed point/line/polygon fixture required")
    env = {key: value for key, value in os.environ.items() if not key.startswith("GL_")}
    extension = os.environ.get("GL_SPATIALITE_EXTENSION")
    if args.backend == "sqlite" and not extension:
        raise ValueError("GL_SPATIALITE_EXTENSION must identify a trusted native extension")
    if extension:
        env["GL_SPATIALITE_EXTENSION"] = extension
    http, grpc, dbport = ports(3)
    base = f"http://127.0.0.1:{http}"
    container = "geoledger-incremental-" + uuid.uuid4().hex[:12]
    server = stats = None
    docker_started = False
    log = stats_log = None
    result = {"success": False, "backend": args.backend}
    command = [str(args.server.resolve()), "--storage", args.backend, "--data-dir", str(output / "data"), "--http", f"127.0.0.1:{http}", "--grpc", f"127.0.0.1:{grpc}", "--admin-subjects", "admin", "--request-timeout-secs", "600", "--db-statement-timeout-secs", "600"]
    (output / "environment.json").write_text(json.dumps({"backend": args.backend, "platform": platform.platform(), "server_command": command, "server_sha256": fingerprint(args.server), "client_sha256": fingerprint(args.client), "fixture": str(args.fixture.resolve()), "fixture_sha256": fingerprint(args.fixture), "metadata": metadata, "rounds": args.rounds, "timeout_seconds": args.timeout, "fixture_method": "untimed SQL in fresh disposable storage; measured client RPC only"}, indent=2))

    def psql(sql):
        completed = subprocess.run(["docker", "exec", "-i", container, "psql", "-At", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", "geoledger_test"], input=sql, capture_output=True, text=True)
        with (output / "fixture-sql.log").open("a") as file:
            file.write(completed.stdout + completed.stderr)
        if completed.returncode:
            raise RuntimeError("fixture SQL failed; see fixture-sql.log")
        return completed.stdout.strip()

    def start_server():
        process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            for _ in range(120):
                try:
                    with urllib.request.urlopen(base + "/ready", timeout=1) as response:
                        if response.status == 200:
                            return process
                except (OSError, urllib.error.URLError):
                    pass
                if process.poll() is not None:
                    raise RuntimeError("server startup failed; see server.log")
                time.sleep(0.5)
            raise RuntimeError("server readiness timeout")
        except BaseException:
            stop(process)
            raise

    try:
        if args.backend == "postgis":
            subprocess.run(["docker", "run", "--rm", "-d", "--name", container, "-e", "POSTGRES_HOST_AUTH_METHOD=trust", "-e", "POSTGRES_DB=geoledger_test", "-p", f"127.0.0.1:{dbport}:5432", "imresamu/postgis:17-3.6-alpine3.22"], check=True, capture_output=True)
            docker_started = True
            for _ in range(120):
                if subprocess.run(["docker", "exec", container, "pg_isready", "-h", "127.0.0.1", "-U", "postgres", "-d", "geoledger_test"], capture_output=True).returncode == 0:
                    break
                time.sleep(0.5)
            else:
                raise RuntimeError("PostGIS startup timeout")
            env["GL_DATABASE_URL"] = f"postgresql://postgres@127.0.0.1:{dbport}/geoledger_test?sslmode=disable"
            stats_log = (output / "postgres-resources.jsonl").open("w")
            stats = subprocess.Popen(["docker", "stats", "--format", "{{json .}}", container], stdout=stats_log, stderr=subprocess.STDOUT, start_new_session=True)
        log = (output / "server.log").open("w")
        server = start_server()
        token_path = output / "data/admin-credentials.json"
        token = json.loads(token_path.read_text())[0]["token"]
        def call(action, body):
            request = urllib.request.Request(base + "/api/v1/" + action, data=compact(body).encode(), headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
            try:
                with urllib.request.urlopen(request, timeout=610) as response:
                    return json.load(response)
            except urllib.error.HTTPError as error:
                raise RuntimeError(f"{action}: HTTP {error.code}: {error.read().decode()}") from error
        project = str(uuid.UUID(call("create_project", {"name": "incremental benchmark"})["project"]))
        started = time.monotonic()
        if args.backend == "sqlite":
            dataset = str(uuid.UUID(call("create_dataset", {"project": project, "name": "background", "geometry_type": family, "coordinate_dimension": dimension})["dataset"]))
            workspace = call("create_workspace", {"project": project})["workspace"]
            saved = call("save", {"project": project, "workspace": workspace, "expected_workspace_version": 0, "edits": [{"dataset": dataset, "feature_id": first["id"], "feature": first}]})
            revision = call("publish", {"project": project, "workspace": workspace, "expected_workspace_version": saved["version"], "request_id": str(uuid.uuid4()), "message": "fixture baseline"})["revision"]
            stop(server)
            count = sqlite_seed(output / "data/geoledger.sqlite3", args.fixture, project, dataset, revision, dimension, extension)
            server = start_server()
        else:
            psql(f"CREATE SCHEMA business; CREATE TABLE business.source(id text PRIMARY KEY, attributes jsonb, geom geometry({kind}{'Z' if dimension == 3 else ''},4326));")
            dataset = str(uuid.UUID(call("create_dataset", {"project": project, "name": "background", "postgis_table": {"schema": "business", "table": "source", "id_column": "id", "geometry_column": "geom"}})["dataset"]))
            revision = call("create_workspace", {"project": project})["base_revision"]
            # Server-owned schema stays intact; bypass only this disposable fixture's write guard.
            subprocess.run(["docker", "cp", str(args.fixture.resolve()), container + ":/tmp/fixture.jsonl"], check=True, capture_output=True)
            sql = f"""BEGIN;
SELECT set_config('geoledger.publication_table','business.source'::regclass::oid::text,true);
CREATE TEMP TABLE fixture_payload(payload text);
COPY fixture_payload FROM '/tmp/fixture.jsonl' WITH(FORMAT csv,DELIMITER E'\\x01',QUOTE E'\\x02');
INSERT INTO business.source SELECT payload::jsonb->>'id',payload::jsonb->'properties'->'attributes',ST_SetSRID(ST_GeomFromGeoJSON(payload::jsonb->'geometry'),4326) FROM fixture_payload;
INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,geom) SELECT '{project}','{dataset}',payload::jsonb->>'id',{int(revision)},(payload::jsonb->'properties')::text,(payload::jsonb->'geometry')::text,ST_SetSRID(ST_GeomFromGeoJSON(payload::jsonb->'geometry'),4326) FROM fixture_payload;
DO $$ BEGIN IF to_regclass('public.gl_source_changes') IS NOT NULL THEN EXECUTE 'DELETE FROM gl_source_changes'; END IF; END $$;
ANALYZE business.source; ANALYZE gl_history; COMMIT;
SELECT count(*) FROM business.source;
"""
            count = int(psql(sql).splitlines()[-1])
        if count != metadata["features"]:
            raise ValueError(f"fixture count differs: {count} != {metadata['features']}")
        manifest = {"project": project, "dataset": dataset, "count": count, "seed_ids": [f"{index:012}" for index in range(1, min(count, 10) + 1)], "update_property": "attributes", "update_geometry": True}
        manifest_path = output / "manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2))
        result.update(fixture_seconds=time.monotonic() - started, fixture_features=count, baseline_revision=revision)
        print(json.dumps({"phase": "fixture_ready", **result}), flush=True)
        env.update(GL_ENDPOINT=f"http://127.0.0.1:{grpc}", GL_TOKEN_FILE=str(token_path), GL_BENCH_DISPOSABLE="1", GL_BENCH_ROUNDS=str(args.rounds))
        result["client"] = monitor([str(args.client.resolve()), str(manifest_path), str(output / "incremental.json")], env, server, output, "incremental", args.timeout, args.rss_limit_mib)
        if args.k6 and result["client"]["exit_code"] == 0:
            read_fixture = output / "read-fixture.json"
            read_fixture.write_text(json.dumps({"project": project, "dataset": dataset, "features": count,
                                                "revision": call("get_project", {"project": project})["head"]}))
            read_env = dict(env, GL_BASE_URL=base, GL_TOKEN=token, GL_BENCH_REPORT=str(read_fixture),
                            GL_LOAD_RATE="50", GL_LOAD_DURATION="30s")
            with (output / "reads.log").open("w") as read_log:
                read_run = subprocess.run([str(args.k6.resolve()), "run", "--quiet", "--summary-export",
                                           str(output / "reads.json"), "k6/scripts/large-data-reads.js"],
                                          env=read_env, stdout=read_log, stderr=subprocess.STDOUT, timeout=90)
            result["read_load_exit_code"] = read_run.returncode
            if read_run.returncode:
                raise AssertionError("read pressure thresholds failed; see reads.json")
        if platform.system() == "Darwin":
            result["native_memory"] = native_leaks(server.pid, output)
            result["native_leaks_exit_code"] = result["native_memory"]["exit_code"]
        if args.backend == "sqlite":
            stop(server)
            with sqlite3.connect(f"file:{output / 'data/geoledger.sqlite3'}?mode=ro", uri=True) as db:
                live = db.execute("SELECT count(*) FROM gl_history WHERE project=? AND dataset=? AND valid_to IS NULL AND properties IS NOT NULL", (project, dataset)).fetchone()[0]
            result["history_live"] = live
        else:
            if result["client"]["exit_code"] == 0:
                result.update(verify_source_crud(call, psql, project, dataset, first))
            counts = psql(f"SELECT count(*) FROM business.source; SELECT count(*) FROM gl_history WHERE project='{project}' AND dataset='{dataset}' AND valid_to IS NULL AND properties IS NOT NULL;").splitlines()
            result["source_live"], result["history_live"] = map(int, counts)
        if result["client"]["exit_code"] == 0:
            if result["history_live"] != count or result.get("source_live", count) != count:
                raise AssertionError("CRUD cycle did not restore background feature count")
            result["success"] = True
    except BaseException as error:
        result["error"] = str(error)
        (output / "failure.log").write_text(traceback.format_exc())
        raise
    finally:
        stop(server)
        stop(stats)
        if log:
            log.close()
        if stats_log:
            stats_log.close()
        if docker_started:
            with (output / "postgres.log").open("w") as docker_log:
                subprocess.run(["docker", "logs", container], stdout=docker_log, stderr=subprocess.STDOUT, check=False)
            cleanup = subprocess.run(["docker", "stop", container], capture_output=True, text=True)
            (output / "cleanup.log").write_text(cleanup.stdout + cleanup.stderr)
            result["container_cleanup_exit_code"] = cleanup.returncode
        (output / "result.json").write_text(json.dumps(result, indent=2))
    return int(not result["success"])


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test_source_crud()
        print("source CRUD verification self-test passed")
    else:
        raise SystemExit(main())
