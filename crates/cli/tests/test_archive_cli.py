#!/usr/bin/env python3
"""Real gl/archive regression against a disposable local server.

Build first: cargo build --locked -p geoledger-server -p geoledger-cli
Run: python3 crates/cli/tests/test_archive_cli.py
GL_SERVER_BIN / GL_CLI_BIN override the debug binaries, as in test-gateway.sh.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.request

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts"))
from auth_test_support import provision


def executable(name: str, override: str) -> Path:
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    suffix = ".exe" if os.name == "nt" else ""
    path = Path(os.environ.get(override, target / "debug" / (name + suffix))).resolve()
    if not path.is_file():
        raise FileNotFoundError(f"build {name} first, or set {override}")
    return path


class ArchiveCliTests(unittest.TestCase):
    def setUp(self):
        server_binary = executable("geoledger-server", "GL_SERVER_BIN")
        self.cli_binary = executable("gl", "GL_CLI_BIN")
        fixtures = ROOT / "target" / "test-fixtures" / "cli"
        fixtures.mkdir(parents=True, exist_ok=True)
        directory = tempfile.TemporaryDirectory(prefix="archive-", dir=fixtures)
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)
        self.data_dir = self.directory / "data"
        self.key_file = self.directory / "api-key"
        # Keep the operator's trusted native-extension location, but isolate all
        # service, credential, TLS, and backend settings from the surrounding run.
        self.environment = {
            key: value for key, value in os.environ.items()
            if not key.startswith("GL_") or key == "GL_SPATIALITE_EXTENSION"
        }
        with socket.socket() as http, socket.socket() as grpc:
            http.bind(("127.0.0.1", 0))
            grpc.bind(("127.0.0.1", 0))
            http_address = f"127.0.0.1:{http.getsockname()[1]}"
            grpc_address = f"127.0.0.1:{grpc.getsockname()[1]}"
        self.base = "http://" + http_address
        self.endpoint = "http://" + grpc_address
        with (self.directory / "server.log").open("wb") as log:
            self.server = subprocess.Popen(
                [str(server_binary), "--storage", "sqlite", "--data-dir", str(self.data_dir),
                 "--http", http_address, "--grpc", grpc_address],
                cwd=ROOT, env=self.environment, stdin=subprocess.DEVNULL,
                stdout=log, stderr=subprocess.STDOUT,
            )
        self.addCleanup(self.stop_server)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.server.poll() is not None:
                self.fail("disposable GeoLedger server exited before readiness")
            try:
                with urllib.request.urlopen(self.base + "/ready", timeout=0.5):
                    break
            except OSError:
                time.sleep(0.05)
        else:
            self.fail("disposable GeoLedger server did not become ready")
        self.identity = provision(self.base, self.data_dir, self.key_file)

    def stop_server(self):
        if self.server.poll() is None:
            self.server.terminate()
            try:
                self.server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.server.kill()
                self.server.wait(timeout=5)

    def api(self, operation: str, body: dict):
        request = urllib.request.Request(
            self.base + "/api/v1/" + operation,
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json",
                     "Authorization": "Bearer " + self.identity["api_key"]},
        )
        with urllib.request.urlopen(request, timeout=5) as response:
            return json.load(response)

    def call(self, operation: str, body: dict, *, from_file: bool = False,
             flags: tuple[str, ...] = (), environment: dict | None = None):
        arguments = [str(self.cli_binary), *flags, "--endpoint", self.endpoint,
                     "--api-key-file", str(self.key_file), "call", operation]
        payload = json.dumps(body)
        if from_file:
            path = self.directory / "request.json"
            path.write_text(payload)
            arguments += ["--file", str(path)]
            payload = None
        return subprocess.run(
            arguments, input=payload, capture_output=True, text=True,
            cwd=ROOT, env=self.environment if environment is None else environment, timeout=10,
        )

    def test_invalid_archive_intent_preserves_archived_project_and_false_reactivates(self):
        project = self.api("create_project", {"name": "CLI archive regression"})["project"]
        archived = self.call("archive_project", {"project": project, "archived": True})
        self.assertEqual(archived.returncode, 0, archived.stderr)
        self.assertEqual(json.loads(archived.stdout)["state"], "archived")
        before = self.api("audit", {"project": project, "limit": 100})
        for body in [
            {"project": project},
            {"project": project, "archive": True},
            {"project": project, "archived": "false"},
            {"project": project, "archived": False, "archive": True},
        ]:
            with self.subTest(body=body):
                rejected = self.call("archive_project", body)
                self.assertEqual(rejected.returncode, 1, rejected.stderr)
                self.assertEqual(rejected.stdout, "")
                error = json.loads(rejected.stderr)
                self.assertEqual(error["code"], "invalid_argument")
                self.assertIs(error["uncertain"], False)
                self.assertIsNone(error["request_id"])
                self.assertEqual(self.api("get_project", {"project": project})["state"], "archived")
                self.assertEqual(self.api("audit", {"project": project, "limit": 100}), before)

        reactivated = self.call("archive_project", {"project": project, "archived": False}, from_file=True)
        self.assertEqual(reactivated.returncode, 0, reactivated.stderr)
        self.assertEqual(json.loads(reactivated.stdout)["state"], "active")
        self.assertEqual(self.api("get_project", {"project": project})["state"], "active")
        after = self.api("audit", {"project": project, "limit": 100})
        self.assertEqual(len(after["events"]), len(before["events"]) + 1)
        self.assertEqual(after["events"][-1]["action"], "unarchive_project")


    def test_call_rejects_current_shape_errors_before_mutating_workspace(self):
        project = self.api("create_project", {"name": "CLI shape regression"})["project"]
        workspace = self.api("create_workspace", {"project": project})["workspace"]
        before_workspace = self.api("get_workspace", {"project": project, "workspace": workspace})
        before_audit = self.api("audit", {"project": project, "limit": 100})
        for operation, body in [
            ("info", {"project": project}),
            ("create_project", {}),
            ("create_project", {"name": "unwanted", "obsolete": True}),
            ("save", {"project": project, "workspace": workspace, "edits": []}),
            ("save", {"project": project, "workspace": workspace,
                      "expected_workspace_version": "0", "edits": []}),
            ("resolve", {"project": project, "workspace": workspace,
                         "expected_workspace_version": 0, "expected_head": 0}),
        ]:
            with self.subTest(operation=operation, body=body):
                rejected = self.call(operation, body)
                self.assertEqual(rejected.returncode, 1, rejected.stderr)
                self.assertEqual(rejected.stdout, "")
                error = json.loads(rejected.stderr)
                self.assertEqual(error["code"], "invalid_argument")
                self.assertIs(error["uncertain"], False)
                self.assertIsNone(error["request_id"])
        self.assertEqual(self.api("get_workspace", {"project": project, "workspace": workspace}),
                         before_workspace)
        self.assertEqual(self.api("audit", {"project": project, "limit": 100}), before_audit)
        current_defaults = self.call("list_projects", {"limit": None})
        self.assertEqual(current_defaults.returncode, 0, current_defaults.stderr)
        self.assertIn(project, [item["project"] for item in json.loads(current_defaults.stdout)["projects"]])

    def test_client_message_limit_flags_and_environment_are_explicit(self):
        for option in ["--max-encoding-bytes", "--max-decoding-bytes"]:
            with self.subTest(option=option):
                rejected = self.call("info", {}, flags=(option, "0"))
                self.assertEqual(rejected.returncode, 1, rejected.stderr)
                error = json.loads(rejected.stderr)
                self.assertEqual(error["code"], "invalid_argument")
                self.assertIs(error["uncertain"], False)
                self.assertIsNone(error["request_id"])
        explicit = self.call("info", {}, flags=("--max-encoding-bytes", "1024",
                                                "--max-decoding-bytes", "1024"))
        self.assertEqual(explicit.returncode, 0, explicit.stderr)
        self.assertIn("version", json.loads(explicit.stdout))
        environment = dict(self.environment, GL_CLIENT_MAX_ENCODING_BYTES="1024",
                           GL_CLIENT_MAX_DECODING_BYTES="1024")
        configured = self.call("info", {}, environment=environment)
        self.assertEqual(configured.returncode, 0, configured.stderr)
        self.assertIn("version", json.loads(configured.stdout))
        rejected = self.call("create_project", {"name": "encoding limit"},
                             flags=("--max-encoding-bytes", "1"))
        self.assertEqual(rejected.returncode, 1, rejected.stderr)
        error = json.loads(rejected.stderr)
        self.assertIn(error["code"], ["resource_exhausted", "unavailable"])
        self.assertIs(error["uncertain"], True)


if __name__ == "__main__":
    unittest.main()
