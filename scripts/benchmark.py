#!/usr/bin/env python3
"""Run the Rust real-file benchmark on a disposable service and sample process RSS.

Build first: cargo build --release -p geoledger-server -p geoledger-client --bin geoledger-server --example large_data
GL_SPATIALITE_EXTENSION=/trusted/mod_spatialite python3 scripts/benchmark.py --geojson file.geojson --output artifacts/performance/run
PostGIS: --backend postgis (creates and removes its own Docker container).
Optional HTTP read pressure: --k6 /path/to/k6 (rates 5,20,50 requests/sec).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import platform
import socket
import subprocess
import time
import urllib.request
import uuid


def fingerprint(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def native_leaks(pid, output):
    """Keep detector signals and inspection errors separate from functional results."""
    output = Path(output)
    log_path = output / "native-leaks.log"
    with log_path.open("w") as log:
        try:
            code = subprocess.run(["leaks", str(pid)], stdout=log, stderr=subprocess.STDOUT, timeout=60).returncode
        except subprocess.TimeoutExpired:
            log.write("\nNative memory inspection exceeded 60 seconds.\n")
            code = 124
    result = {"test": "native-leaks", "exit_code": code,
              "status": {0: "no_signal", 1: "signal"}.get(code, "error"),
              "restricted": "not debuggable" in log_path.read_text()}
    if code == 1:
        # Preserve evidence before the disposable server and allocation history exit.
        with (output / "native-leaks-graph.log").open("w") as log:
            try:
                result["graph_exit_code"] = subprocess.run(
                    ["leaks", "-outputGraph", str(output / "native-leaks.memgraph"), str(pid)],
                    stdout=log, stderr=subprocess.STDOUT, timeout=60).returncode
            except subprocess.TimeoutExpired:
                result["graph_exit_code"] = 124
    return result


def attach_source(base, token, fixture, container, output, name):
    """Create an existing business-table fixture, then enroll and publish through the API."""
    def call(action, body):
        request = urllib.request.Request(base + "/api/v1/" + action, data=json.dumps(body).encode(), headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=610) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"{action}: HTTP {error.code}: {error.read().decode()}") from error
    family = {"point": "MultiPoint", "line": "MultiLineString", "polygon": "MultiPolygon"}[fixture["geometry_type"]]
    kind = family + ("Z" if fixture["dimension"] == 3 else "")
    table = "source_" + uuid.uuid4().hex[:12]
    # Values below are produced by our disposable service, never interpolated user input.
    project = str(uuid.UUID(fixture["project"]))
    dataset = str(uuid.UUID(fixture["dataset"]))
    sql = f"CREATE SCHEMA IF NOT EXISTS business; CREATE TABLE business.{table}(id text PRIMARY KEY,attributes jsonb,geom geometry({kind},4326)); INSERT INTO business.{table} SELECT feature_id,properties::jsonb,ST_Multi(ST_SetSRID(ST_GeomFromGeoJSON(geometry_json),4326)) FROM gl_history WHERE project='{project}' AND dataset='{dataset}' AND valid_to IS NULL;"
    subprocess.run(["docker", "exec", container, "psql", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", "geoledger_test", "-c", sql], check=True, capture_output=True)
    started = time.monotonic()
    bound = call("create_dataset", {"project": project, "name": table, "postgis_table": {"schema": "business", "table": table, "id_column": "id", "geometry_column": "geom"}})
    result = {"features": fixture["features"], "attach_seconds": time.monotonic() - started}
    feature = call("features", {"project": project, "dataset": bound["dataset"], "feature_id": "000000000001"})
    feature = {key: feature[key] for key in ["type", "id", "properties", "geometry"]}
    feature["properties"]["attributes"]["benchmark"] = True
    workspace = call("create_workspace", {"project": project})
    saved = call("save", {"project": project, "workspace": workspace["workspace"], "expected_workspace_version": 0, "edits": [{"dataset": bound["dataset"], "feature_id": feature["id"], "feature": feature}]})
    started = time.monotonic()
    receipt = call("publish", {"project": project, "workspace": workspace["workspace"], "expected_workspace_version": saved["version"], "request_id": str(uuid.uuid4()), "message": "business table benchmark"})
    result.update(single_row_publish_seconds=time.monotonic() - started, changes=receipt["changes"])
    sql = f"SELECT attributes->>'benchmark' FROM business.{table} WHERE id='000000000001'"
    actual = subprocess.run(["docker", "exec", container, "psql", "-At", "-U", "postgres", "-d", "geoledger_test", "-c", sql], check=True, capture_output=True, text=True).stdout.strip()
    assert actual == "true", "publication did not update original table"
    result["original_table_updated"] = True
    (output / f"{name}-binding.json").write_text(json.dumps(result, indent=2))
    print(json.dumps({"binding": name, **result}), flush=True)


def ports(count):
    with_sockets = [socket.socket() for _ in range(count)]
    try:
        for sock in with_sockets:
            sock.bind(("127.0.0.1", 0))
        return [sock.getsockname()[1] for sock in with_sockets]
    finally:
        for sock in with_sockets:
            sock.close()


def stop(process):
    if process and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def monitor(command, env, server, output, name, timeout, rss_limit):
    started = time.monotonic()
    peak = {"server_rss_mib": 0, "client_rss_mib": 0}
    reason = None
    with (output / f"{name}.log").open("w") as log, (output / f"{name}-memory.jsonl").open("w") as memory:
        child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while child.poll() is None:
                sample = {"seconds": round(time.monotonic() - started, 3)}
                for label, process in [("server", server), ("client", child)]:
                    raw = subprocess.run(["ps", "-o", "rss=", "-p", str(process.pid)], capture_output=True, text=True).stdout.strip()
                    if raw:
                        key = f"{label}_rss_mib"
                        sample[key] = int(raw) / 1024
                        peak[key] = max(peak[key], sample[key])
                        if sample[key] > rss_limit:
                            reason = f"{label} exceeded {rss_limit} MiB RSS safety ceiling"
                memory.write(json.dumps(sample) + "\n")
                memory.flush()
                if time.monotonic() - started > timeout:
                    reason = f"test exceeded {timeout} seconds"
                if server.poll() is not None:
                    reason = "server exited during test"
                if reason:
                    stop(child)
                    break
                time.sleep(0.5)
            result = {"exit_code": child.wait(), "seconds": time.monotonic() - started, **peak, "stopped_reason": reason}
            (output / f"{name}-process.json").write_text(json.dumps(result, indent=2))
            print(json.dumps({"test": name, **result}), flush=True)
            return result
        finally:
            stop(child)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--geojson", type=Path, action="append", required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--backend", choices=["sqlite", "postgis"], default="sqlite")
    p.add_argument("--k6", type=Path)
    p.add_argument("--timeout", type=int, default=1200)
    p.add_argument("--rss-limit-mib", type=int, default=4096)
    a = p.parse_args()
    output = a.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    # Strip ambient credentials and configuration; each run owns its data and listeners.
    env = {k: v for k, v in os.environ.items() if not k.startswith("GL_")}
    if os.environ.get("GL_SPATIALITE_EXTENSION"):
        env["GL_SPATIALITE_EXTENSION"] = os.environ["GL_SPATIALITE_EXTENSION"]
    http, grpc, dbport = ports(3)
    container = f"geoledger-perf-{uuid.uuid4().hex[:12]}"
    server = None
    database_monitor = None
    database_log = None
    docker_started = False
    results = []
    try:
        if a.backend == "postgis":
            subprocess.run(["docker", "run", "--rm", "-d", "--name", container, "-e", "POSTGRES_HOST_AUTH_METHOD=trust", "-e", "POSTGRES_DB=geoledger_test", "-p", f"127.0.0.1:{dbport}:5432", "imresamu/postgis:17-3.6-alpine3.22"], check=True, capture_output=True)
            docker_started = True
            for _ in range(120):
                if subprocess.run(["docker", "exec", container, "pg_isready", "-h", "127.0.0.1", "-U", "postgres", "-d", "geoledger_test"], capture_output=True).returncode == 0:
                    break
                time.sleep(0.5)
            else:
                raise RuntimeError("PostGIS startup timeout")
            database_log = (output / "postgres-resources.jsonl").open("w")
            database_monitor = subprocess.Popen(["docker", "stats", "--format", "{{json .}}", container], stdout=database_log, stderr=subprocess.STDOUT, start_new_session=True)
            env["GL_DATABASE_URL"] = f"postgresql://postgres@127.0.0.1:{dbport}/geoledger_test?sslmode=disable"
        base = f"http://127.0.0.1:{http}"
        command = ["target/release/geoledger-server", "--storage", a.backend, "--data-dir", str(output / "data"), "--http", f"127.0.0.1:{http}", "--grpc", f"127.0.0.1:{grpc}", "--admin-subjects", "admin", "--request-timeout-secs", "600", "--db-statement-timeout-secs", "600"]
        (output / "environment.json").write_text(json.dumps({"backend": a.backend, "server_command": command, "platform": platform.platform(), "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(), "server_sha256": fingerprint("target/release/geoledger-server"), "client_sha256": fingerprint("target/release/examples/large_data"), "rss_sampling_seconds": 0.5, "rpc_deadline_seconds": 600, "client_deadline_seconds": a.timeout, "files": [{"path": str(f.resolve()), "bytes": f.stat().st_size, "sha256": fingerprint(f)} for f in a.geojson]}, indent=2))
        with (output / "server.log").open("w") as log:
            server = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            for _ in range(120):
                try:
                    with urllib.request.urlopen(base + "/ready", timeout=1) as r:
                        if r.status == 200:
                            break
                except (OSError, urllib.error.URLError):
                    pass
                if server.poll() is not None:
                    raise RuntimeError("server startup failed; see server.log")
                time.sleep(0.5)
            else:
                raise RuntimeError("server readiness timeout")
            env.update(GL_ENDPOINT=f"http://127.0.0.1:{grpc}", GL_TOKEN_FILE=str(output / "data/admin-credentials.json"), GL_BENCH_DISPOSABLE="1")
            binding_reports = []
            for path in a.geojson:
                report = output / f"{path.stem}.json"
                result = monitor(["target/release/examples/large_data", str(path.resolve()), str(report)], env, server, output, path.stem, a.timeout, a.rss_limit_mib)
                results.append({"file": str(path), **result})
                if result["exit_code"] == 0 and a.backend == "postgis":
                    binding_reports.append(report)
                if result["exit_code"] == 0 and a.k6:
                    token = json.loads(Path(env["GL_TOKEN_FILE"]).read_text())[0]["token"]
                    load_env = dict(env, GL_BASE_URL=base, GL_TOKEN=token, GL_BENCH_REPORT=str(report))
                    for rate in [5, 20, 50]:
                        load_env["GL_LOAD_RATE"] = str(rate)
                        name = f"{path.stem}-http-{rate}rps"
                        results.append({"test": name, **monitor([str(a.k6.resolve()), "run", "--quiet", "--summary-export", str(output / f"{name}.json"), "k6/scripts/large-data-reads.js"], load_env, server, output, name, 120, a.rss_limit_mib)})
                if server.poll() is not None:
                    break
            for fixture in binding_reports:
                bind_env = dict(env, GL_BASE_URL=base, GL_BENCH_REPORT=str(fixture), GL_BENCH_CONTAINER=container)
                command = ["python3", "-c", "import json,os; from pathlib import Path; from scripts.benchmark import attach_source; p=Path(os.environ['GL_BENCH_REPORT']); token=json.loads(Path(os.environ['GL_TOKEN_FILE']).read_text())[0]['token']; attach_source(os.environ['GL_BASE_URL'],token,json.loads(p.read_text()),os.environ['GL_BENCH_CONTAINER'],p.parent,p.stem)"]
                results.append({"test": fixture.stem + "-binding", **monitor(command, bind_env, server, output, fixture.stem + "-binding", a.timeout, a.rss_limit_mib)})
            token = json.loads(Path(env["GL_TOKEN_FILE"]).read_text())[0]["token"]
            request = urllib.request.Request(base + "/metrics", headers={"Authorization": "Bearer " + token})
            with urllib.request.urlopen(request, timeout=5) as response:
                (output / "server-metrics.txt").write_bytes(response.read())
            if platform.system() == "Darwin":
                results.append(native_leaks(server.pid, output))
    finally:
        stop(server)
        stop(database_monitor)
        if database_log:
            database_log.close()
        if docker_started:
            with (output / "postgres.log").open("w") as log:
                subprocess.run(["docker", "logs", container], stdout=log, stderr=subprocess.STDOUT, check=False)
            subprocess.run(["docker", "stop", container], check=True, capture_output=True)
        (output / "process-results.json").write_text(json.dumps(results, indent=2))
    return int(any(r["exit_code"] for r in results))


if __name__ == "__main__":
    raise SystemExit(main())
