#!/usr/bin/env python3
"""Disposable incremental CRUD benchmark; SQL only initializes untimed test fixtures.

The measured Rust client uses RPC exclusively. Each run creates fresh server storage
and, for PostGIS, its own container and business table. Never use production data dirs.
"""
import argparse
import json
import math
import re
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


# SQL fixture initialization targets the current service storage contract only.
STORAGE_FORMAT = 19
INTERNAL_SRID = 0


def check_storage_format(version):
    if version != STORAGE_FORMAT:
        raise ValueError(f"fixture seeding requires storage format {STORAGE_FORMAT}; server created {version}; rebuild the current server")


def fixture_crs(metadata, requested=None):
    declared = metadata.get("crs")
    for value in (declared, requested):
        if value is not None and (not isinstance(value, str) or not value.strip() or len(value.encode()) > 16384 or "\0" in value):
            raise ValueError("fixture CRS must be a nonempty CRS identifier or definition")
    if declared is not None and requested is not None and declared.strip() != requested.strip():
        raise ValueError("--crs differs from fixture metadata CRS; verify the source coordinates")
    value = requested if requested is not None else declared
    if value is None:
        raise ValueError("fixture CRS is missing; regenerate metadata with an explicit source CRS or pass --crs; coordinates are not reprojected")
    return value.strip()


def postgis_srid(crs):
    match = re.fullmatch(r"(?:EPSG|POSTGIS):([0-9]{1,9})", crs)
    if not match:
        raise ValueError("PostGIS fixture CRS must identify a registered EPSG:<srid> or POSTGIS:<srid>")
    return int(match[1])


def geometry_positions(value):
    """Read finite XY/XYZ positions without modifying source coordinates."""
    if not isinstance(value, list):
        raise ValueError("coordinates must be arrays")
    if value and isinstance(value[0], (int, float)):
        if len(value) not in (2, 3) or any(isinstance(n, bool) or not isinstance(n, (int, float)) or not math.isfinite(n) for n in value):
            raise ValueError("finite XY or XYZ coordinates required")
        yield value
    else:
        for item in value:
            yield from geometry_positions(item)


def shift_geometry(geometry):
    """Translate finite native XY uniformly; retain Z, type, and ring closure."""
    positions = list(geometry_positions(geometry["coordinates"]))
    if not positions:
        raise ValueError("nonempty geometry required for translation")
    deltas = []
    for axis in range(2):
        low = min(p[axis] for p in positions)
        high = max(p[axis] for p in positions)
        step = max(1e-7, max(abs(low), abs(high)) * sys.float_info.epsilon * 4)
        delta = -step if high > 0 else step
        if any(not math.isfinite(v + delta) or v + delta == v for v in (low, high)):
            raise ValueError("geometry cannot be translated with finite distinct coordinates")
        deltas.append(delta)
    for position in positions:
        for axis, delta in enumerate(deltas):
            position[axis] += delta


def postgis_seed_sql(project, dataset, revision, source_srid):
    # The source table is the only current copy; untouched rows inherit registration revision.
    project, dataset = str(uuid.UUID(project)), str(uuid.UUID(dataset))
    return f"""BEGIN;
DO $fixture$ BEGIN
 PERFORM project,dataset,revision,definition FROM gl_schema_history LIMIT 0;
 PERFORM project,workspace,dataset,definition FROM gl_workspace_schemas LIMIT 0;
 IF EXISTS(SELECT 1 FROM gl_schema_history WHERE project='{project}' AND dataset='{dataset}')
 OR EXISTS(SELECT 1 FROM gl_workspace_schemas WHERE project='{project}' AND dataset='{dataset}') THEN
  RAISE EXCEPTION 'fixture seeding rejects existing schema state';
 END IF;
END $fixture$;
SELECT set_config('geoledger.publication_table','business.source'::regclass::oid::text,true);
CREATE TEMP TABLE fixture_payload(payload text);
COPY fixture_payload FROM '/tmp/fixture.jsonl' WITH(FORMAT csv,DELIMITER E'\\x01',QUOTE E'\\x02');
INSERT INTO business.source SELECT payload::jsonb->>'id',payload::jsonb->'properties'->'attributes',ST_SetSRID(ST_GeomFromGeoJSON(payload::jsonb->'geometry'),{source_srid}) FROM fixture_payload;
DELETE FROM gl_source_changes WHERE schema_name='business' AND table_name='source';
ANALYZE business.source; COMMIT;
SELECT count(*) FROM business.source;
"""


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


def rows(path, dimension=None):
    with path.open() as source:
        for index, line in enumerate(source, 1):
            value = json.loads(line)
            if value.get("type") != "Feature" or value.get("id") != f"{index:012}" or not isinstance(value.get("properties", {}).get("attributes"), dict):
                raise ValueError(f"invalid fixture row {index}")
            if dimension is not None:
                seen = False
                for position in geometry_positions(value["geometry"]["coordinates"]):
                    seen = True
                    if len(position) != dimension:
                        raise ValueError(f"fixture row {index} coordinate dimension differs: expected {dimension}, got {len(position)}")
                if not seen:
                    raise ValueError(f"fixture row {index} requires a nonempty geometry")
            yield value


def sqlite_seed(path, fixture, project, dataset, revision, dimension, extension):
    with sqlite3.connect(path) as db:
        db.enable_load_extension(True)
        db.load_extension(extension)
        db.enable_load_extension(False)
        db.execute("PRAGMA foreign_keys=ON")
        if dimension not in (2, 3):
            raise ValueError("fixture dimension must be 2 or 3")
        check_storage_format(db.execute("SELECT version FROM gl_format").fetchone()[0])
        for schema_table, columns in [("gl_schema_history", "project,dataset,revision,definition"),
                                      ("gl_workspace_schemas", "project,workspace,dataset,definition")]:
            try:
                db.execute(f"SELECT {columns} FROM {schema_table} LIMIT 0")
                present = db.execute(f"SELECT EXISTS(SELECT 1 FROM {schema_table} WHERE project=? AND dataset=?)", (project, dataset)).fetchone()[0]
            except sqlite3.OperationalError as error:
                raise ValueError(f"fixture schema contract requires {schema_table}") from error
            if present:
                raise ValueError("fixture seeding rejects existing schema state")
        column = "geom_z" if dimension == 3 else "geom"
        table = db.execute("SELECT storage_table FROM gl_datasets WHERE project=? AND id=?", (project, dataset)).fetchone()[0]
        if not isinstance(table, str) or not re.fullmatch(r"gl_data_[0-9a-f]{32}", table):
            raise ValueError("fixture requires a managed dataset table")
        contract = db.execute("SELECT coord_dimension,srid FROM geometry_columns WHERE f_table_name=? AND f_geometry_column=?", (table, column)).fetchone()
        if contract != (dimension, INTERNAL_SRID):
            raise ValueError(f"{table}.{column} geometry contract differs: expected dimension/SRID {(dimension, INTERNAL_SRID)}, got {contract}")
        db.create_function("gl_dataset_write_allowed", 0, lambda: 1)
        sql = f"INSERT INTO {table}(feature_id,valid_from,properties,geometry_json,{column}) VALUES(?,?,?,?,coalesce(SetSRID(GeomFromGeoJSON(?),{INTERNAL_SRID}),SetSRID(GeomFromGeoJSON(?),{INTERNAL_SRID})))"
        count = 0
        for feature in rows(fixture, dimension):
            count += 1
            if count == 1:
                continue  # The API published the first row to establish a real baseline revision.
            geometry = compact(feature["geometry"])
            db.execute(sql, (feature["id"], revision, compact(feature["properties"]), geometry, geometry, collection(feature["geometry"])))
        bad = db.execute(f"SELECT count(*) FROM {table} WHERE {column} IS NULL").fetchone()[0]
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

            shift_geometry(feature["geometry"])
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


def check_results(result, count):
    if result["client"]["exit_code"] != 0:
        return
    if result["current_live"] != count or result.get("source_live", count) != count:
        raise AssertionError("CRUD cycle did not restore background feature count")
    result["integrity_verified"] = True
    if result.get("soak", {}).get("exit_code", 0):
        raise AssertionError("incremental soak failed; integrity checked; see soak.json")
    if result.get("read_load_exit_code", 0):
        raise AssertionError("read pressure thresholds failed; integrity checked; see reads.json")
    result["success"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["server", "client", "fixture", "metadata", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--crs", help="source CRS; required when fixture metadata has no crs")
    parser.add_argument("--backend", choices=["sqlite", "postgis"], default="sqlite")
    parser.add_argument("--postgis-image", default="postgis/postgis@sha256:01a6a70e41e6c4467c8f55f6063555ed72db2d6662cd0d571040d42eadaeb6f6")
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--timeout", type=int, default=1200)
    parser.add_argument("--rss-limit-mib", type=int, default=4096)
    parser.add_argument("--k6", type=Path)
    parser.add_argument("--load-seconds", type=int, default=0)
    parser.add_argument("--load-workers", type=int, default=20)
    args = parser.parse_args()
    if not 0 <= args.load_seconds <= 86400 or not 1 <= args.load_workers <= 100:
        parser.error("invalid load duration or concurrency")
    if not 3 <= args.rounds <= 100 or args.timeout <= 0 or args.rss_limit_mib <= 0:
        parser.error("rounds must be 3..100; timeout and RSS ceiling must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    metadata = json.loads(args.metadata.read_text())
    crs = fixture_crs(metadata, args.crs)
    metadata["crs"] = crs
    source_srid = postgis_srid(crs) if args.backend == "postgis" else None
    dimension = metadata["dimension"]
    family = metadata["geometry_type"]
    if dimension not in [2, 3] or family not in ["point", "line", "polygon"] or metadata["features"] < 1:
        raise ValueError("invalid fixture metadata")
    first = next(rows(args.fixture, dimension))
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
    result = {"success": False, "backend": args.backend, "phase": "startup", "crs": crs, "internal_srid": INTERNAL_SRID, "source_srid": source_srid}
    command = [str(args.server.resolve()), "--storage", args.backend, "--data-dir", str(output / "data"), "--http", f"127.0.0.1:{http}", "--grpc", f"127.0.0.1:{grpc}", "--request-timeout-secs", "600", "--db-statement-timeout-secs", "600"]
    (output / "environment.json").write_text(json.dumps({"backend": args.backend, "platform": platform.platform(), "server_command": command, "server_sha256": fingerprint(args.server), "client_sha256": fingerprint(args.client), "fixture": str(args.fixture.resolve()), "fixture_sha256": fingerprint(args.fixture), "metadata": metadata, "rounds": args.rounds, "load_seconds": args.load_seconds, "load_workers": args.load_workers, "postgis_image": args.postgis_image if args.backend == "postgis" else None, "timeout_seconds": args.timeout, "expected_storage_format": STORAGE_FORMAT, "internal_srid": INTERNAL_SRID, "source_srid": source_srid, "fixture_method": "untimed SQL in fresh disposable storage; measured client RPC only"}, indent=2))

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
            subprocess.run(["docker", "run", "--rm", "-d", "--name", container, "-e", "POSTGRES_HOST_AUTH_METHOD=trust", "-e", "POSTGRES_DB=geoledger_test", "-p", f"127.0.0.1:{dbport}:5432", args.postgis_image], check=True, capture_output=True)
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
        from auth_test_support import provision
        token_path = output / "api-key"
        token = provision(base, output / "data", token_path)["api_key"]
        def call(action, body):
            request = urllib.request.Request(base + "/api/v1/" + action, data=compact(body).encode(), headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
            try:
                with urllib.request.urlopen(request, timeout=610) as response:
                    return json.load(response)
            except urllib.error.HTTPError as error:
                raise RuntimeError(f"{action}: HTTP {error.code}: {error.read().decode()}") from error
        project = str(uuid.UUID(call("create_project", {"name": "incremental benchmark"})["project"]))
        started = time.monotonic()
        result["phase"] = "fixture"
        if args.backend == "sqlite":
            created = call("create_dataset", {"project": project, "name": "background", "geometry_type": family, "coordinate_dimension": dimension, "crs": crs})
            if created.get("crs") != crs:
                raise ValueError(f"dataset CRS differs: expected {crs}, got {created.get('crs')}")
            dataset = str(uuid.UUID(created["dataset"]))
            workspace = call("create_workspace", {"project": project})["workspace"]
            saved = call("save", {"project": project, "workspace": workspace, "expected_workspace_version": 0, "edits": [{"dataset": dataset, "feature_id": first["id"], "feature": first}]})
            revision = call("publish", {"project": project, "workspace": workspace, "expected_workspace_version": saved["version"], "request_id": str(uuid.uuid4()), "message": "fixture baseline"})["revision"]
            stop(server)
            count = sqlite_seed(output / "data/geoledger.sqlite3", args.fixture, project, dataset, revision, dimension, extension)
            server = start_server()
        else:
            check_storage_format(int(psql("SELECT version FROM gl_format")))
            psql(f"CREATE SCHEMA business; CREATE TABLE business.source(id text PRIMARY KEY, attributes jsonb, geom geometry({kind}{'Z' if dimension == 3 else ''},{source_srid}));")
            created = call("create_dataset", {"project": project, "name": "background", "postgis_table": {"schema": "business", "table": "source", "id_column": "id", "geometry_column": "geom"}})
            if (created.get("geometry_type"), created.get("coordinate_dimension"), created.get("crs")) != (family, dimension, crs):
                raise ValueError("bound source geometry type, dimension or CRS differs from fixture metadata")
            dataset = str(uuid.UUID(created["dataset"]))
            revision = call("create_workspace", {"project": project})["base_revision"]
            # Validate dimensions before COPY; native parsers may promote XY to XYZ.
            for _ in rows(args.fixture, dimension):
                pass
            # Server-owned schema stays intact; bypass only this disposable fixture's write guard.
            subprocess.run(["docker", "cp", str(args.fixture.resolve()), container + ":/tmp/fixture.jsonl"], check=True, capture_output=True)
            sql = postgis_seed_sql(project, dataset, revision, source_srid)
            count = int(psql(sql).splitlines()[-1])
        if count != metadata["features"]:
            raise ValueError(f"fixture count differs: {count} != {metadata['features']}")
        manifest = {"project": project, "dataset": dataset, "count": count, "seed_ids": [f"{index:012}" for index in range(1, min(count, 10) + 1)], "update_property": "attributes", "update_geometry": True}
        manifest_path = output / "manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2))
        result.update(fixture_seconds=time.monotonic() - started, fixture_features=count, baseline_revision=revision)
        print(json.dumps({**result, "phase": "fixture_ready"}), flush=True)
        env.update(GL_ENDPOINT=f"http://127.0.0.1:{grpc}", GL_API_KEY_FILE=str(token_path), GL_BENCH_DISPOSABLE="1", GL_BENCH_ROUNDS=str(args.rounds))
        result["phase"] = "incremental"
        result["client"] = monitor([str(args.client.resolve()), str(manifest_path), str(output / "incremental.json")], env, server, output, "incremental", args.timeout, args.rss_limit_mib)
        if args.load_seconds and result["client"]["exit_code"] == 0:
            load_env = dict(env, GL_BASE_URL=base)
            result["soak"] = monitor([sys.executable, "scripts/load-incremental.py", "--manifest", str(manifest_path),
                                      "--feature", str(args.fixture.resolve()), "--output", str(output / "soak.json"),
                                      "--seconds", str(args.load_seconds), "--workers", str(args.load_workers)],
                                     load_env, server, output, "soak", args.load_seconds + 180, args.rss_limit_mib)
        if args.k6 and result["client"]["exit_code"] == 0:
            read_fixture = output / "read-fixture.json"
            read_fixture.write_text(json.dumps({"project": project, "dataset": dataset, "features": count,
                                                "revision": call("get_project", {"project": project})["head"]}))
            read_env = dict(env, GL_BASE_URL=base, GL_API_KEY=token, GL_BENCH_REPORT=str(read_fixture),
                            GL_LOAD_RATE="50", GL_LOAD_DURATION="30s")
            with (output / "reads.log").open("w") as read_log:
                read_run = subprocess.run([str(args.k6.resolve()), "run", "--quiet", "--summary-export",
                                           str(output / "reads.json"), "k6/scripts/large-data-reads.js"],
                                          env=read_env, stdout=read_log, stderr=subprocess.STDOUT, timeout=90)
            result["read_load_exit_code"] = read_run.returncode
        if platform.system() == "Darwin":
            result["native_memory"] = native_leaks(server.pid, output)
            result["native_leaks_exit_code"] = result["native_memory"]["exit_code"]
        if args.backend == "sqlite":
            stop(server)
            with sqlite3.connect(f"file:{output / 'data/geoledger.sqlite3'}?mode=ro", uri=True) as db:
                db.enable_load_extension(True)
                db.load_extension(extension)
                db.enable_load_extension(False)
                table = db.execute("SELECT storage_table FROM gl_datasets WHERE project=? AND id=?", (project, dataset)).fetchone()[0]
                result["current_srids"] = [row[0] for row in db.execute(f"SELECT DISTINCT ST_SRID(coalesce(geom,geom_z)) FROM {table}")]
                result["current_live"] = db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
                result["history_rows"] = db.execute("SELECT count(*) FROM gl_history WHERE project=? AND dataset=?", (project, dataset)).fetchone()[0]
        else:
            if result["client"]["exit_code"] == 0:
                result.update(verify_source_crud(call, psql, project, dataset, first))
            counts = psql(f"SELECT count(*) FROM business.source; SELECT count(*) FROM gl_history WHERE project='{project}' AND dataset='{dataset}';").splitlines()
            result["source_live"], result["history_rows"] = map(int, counts)
            result["current_live"] = result["source_live"]
            result["source_srids"] = [int(value) for value in psql("SELECT DISTINCT ST_SRID(geom) FROM business.source").splitlines()]
            if result["source_srids"] != [source_srid]:
                raise AssertionError("business source SRID differs from fixture CRS")
        if args.backend == "sqlite" and result["current_srids"] != [INTERNAL_SRID]:
            raise AssertionError("managed table SRID differs from the internal geometry contract")
        result["crs_verified"] = True
        check_results(result, count)
        if result["success"]:
            result["phase"] = "complete"
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
