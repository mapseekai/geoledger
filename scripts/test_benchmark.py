"""Supervisor checks using short-lived dummy processes, not the user's services."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from benchmark import monitor, native_leaks, ports, stop


class SupervisorTests(unittest.TestCase):
    def test_memory_signal_keeps_evidence_when_graph_capture_fails(self):
        def inspect(command, **kwargs):
            if "-outputGraph" in command:
                raise subprocess.TimeoutExpired(command, 60)
            kwargs["stdout"].write("Process is not debuggable.\n1 leak for 98304 total leaked bytes.\n")
            return subprocess.CompletedProcess(command, 1)

        with tempfile.TemporaryDirectory() as folder, patch("benchmark.subprocess.run", side_effect=inspect):
            result = native_leaks(123, folder)
            self.assertEqual(result["status"], "signal")
            self.assertEqual(result["exit_code"], 1)
            self.assertEqual(result["graph_exit_code"], 124)
            self.assertTrue(result["restricted"])
            self.assertIn("98304", (Path(folder) / "native-leaks.log").read_text())

    def test_memory_timeout_is_not_a_clean_check(self):
        with tempfile.TemporaryDirectory() as folder, patch("benchmark.subprocess.run", side_effect=subprocess.TimeoutExpired("leaks", 60)):
            result = native_leaks(123, folder)
            self.assertEqual(result["status"], "error")
            self.assertEqual(result["exit_code"], 124)

    def test_clean_memory_check_does_not_capture_graph(self):
        with tempfile.TemporaryDirectory() as folder, patch("benchmark.subprocess.run", return_value=subprocess.CompletedProcess("leaks", 0)) as run:
            result = native_leaks(123, folder)
            self.assertEqual(result["status"], "no_signal")
            self.assertEqual(run.call_count, 1)

    def test_ports_are_unique_and_failure_is_recorded(self):
        self.assertEqual(len(set(ports(3))), 3)
        with tempfile.TemporaryDirectory() as folder:
            server = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)"], start_new_session=True)
            try:
                result = monitor([sys.executable, "-c", "raise SystemExit(7)"], os.environ.copy(), server, Path(folder), "failure", 5, 1024)
                self.assertEqual(result["exit_code"], 7)
                self.assertTrue((Path(folder) / "failure-process.json").exists())
                timeout = monitor([sys.executable, "-c", "import time;time.sleep(30)"], os.environ.copy(), server, Path(folder), "timeout", 0.1, 1024)
                self.assertIn("exceeded", timeout["stopped_reason"])
                self.assertIsNone(server.poll())
            finally:
                stop(server)
            self.assertIsNotNone(server.poll())


if __name__ == "__main__":
    unittest.main()
