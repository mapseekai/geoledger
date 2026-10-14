"""Validate Compose administrator configuration without starting containers."""
from __future__ import annotations

import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
COMPOSE_FILES = ("compose.yaml", "deploy/compose.production.yaml")


class AdministratorEnvironmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which("docker"):
            raise unittest.SkipTest("Docker Compose is required for configuration validation")
        result = subprocess.run(["docker", "compose", "version"], capture_output=True)
        if result.returncode:
            raise unittest.SkipTest("Docker Compose plugin is required")

    def configuration(self, filename: str, administrator: dict[str, str]):
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GL_")}
        environment.update({key: secrets.token_urlsafe(32) for key in (
            "GL_WEB_SESSION_SECRET", "GL_DB_PASSWORD", "GL_DB_APP_PASSWORD")})
        environment.update(administrator)
        result = subprocess.run(
            ["docker", "compose", "--env-file", os.devnull, "-f", str(ROOT / filename),
             "config", "--format", "json"],
            cwd=ROOT, env=environment, capture_output=True, text=True, timeout=30,
        )
        # Rendered config contains secrets; never include stdout/stderr in failures.
        self.assertEqual(result.returncode, 0, f"Compose rendering failed for {filename}")
        return json.loads(result.stdout)["services"]["geoledger"]["environment"]

    def test_unset_password_preserves_random_bootstrap(self):
        for filename in COMPOSE_FILES:
            with self.subTest(compose=filename):
                environment = self.configuration(filename, {})
                self.assertEqual(environment["GL_ADMIN_USERNAME"], "admin")
                self.assertIsNone(environment.get("GL_ADMIN_PASSWORD"))

    def test_custom_administrator_reaches_the_server(self):
        password = secrets.token_urlsafe(32)
        for filename in COMPOSE_FILES:
            with self.subTest(compose=filename):
                environment = self.configuration(filename, {
                    "GL_ADMIN_USERNAME": "deployment-admin", "GL_ADMIN_PASSWORD": password,
                })
                self.assertEqual(environment["GL_ADMIN_USERNAME"], "deployment-admin")
                self.assertTrue(environment.get("GL_ADMIN_PASSWORD") == password,
                                "Initial password was not forwarded")


if __name__ == "__main__":
    unittest.main()
