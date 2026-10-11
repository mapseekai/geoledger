"""Exercise the real two-service launcher with disposable child processes."""
import os
import contextlib
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ContainerLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'web').mkdir()
        shutil.copy(ROOT / 'deploy/container.cjs', self.root / 'container.cjs')
        self.env = dict(os.environ, PATH=str(self.root) + os.pathsep + os.environ['PATH'],
                        GL_WEB_SESSION_SECRET='x' * 48)
        self.child('geoledger-server')
        self.child('web/server.js')

    def child(self, name, exit_code=None):
        marker = self.root / (Path(name).name + '.events')
        code = f"const fs=require('node:fs');const file={str(marker)!r};fs.appendFileSync(file,'start\\n');"
        code += "process.on('SIGTERM',()=>{fs.appendFileSync(file,'stop\\n');process.exit(0)});"
        code += 'setInterval(()=>{},1000);'
        if exit_code is not None:
            other = self.root / ('server.js.events' if name == 'geoledger-server' else 'geoledger-server.events')
            code += f'setInterval(()=>{{if(fs.existsSync({str(other)!r})) process.exit({exit_code})}},100);'
        file = self.root / name
        file.write_text('#!/usr/bin/env node\n' + code)
        file.chmod(0o755)

    def start(self):
        process = subprocess.Popen([shutil.which('node'), str(self.root / 'container.cjs')], env=self.env,
                                   start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        def cleanup():
            with contextlib.suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
        self.addCleanup(cleanup)
        return process

    def test_backend_failure_stops_console(self):
        self.child('geoledger-server', 7)
        self.assertEqual(self.start().wait(timeout=5), 7)
        self.assertIn('stop', (self.root / 'server.js.events').read_text())

    def test_unexpected_console_clean_exit_stops_backend_and_fails(self):
        self.child('web/server.js', 0)
        self.assertEqual(self.start().wait(timeout=5), 1)
        self.assertIn('stop', (self.root / 'geoledger-server.events').read_text())

    def test_signal_reaches_both_services(self):
        process = self.start()
        markers = [self.root / name for name in ('geoledger-server.events', 'server.js.events')]
        deadline = time.monotonic() + 5
        while not all(p.exists() for p in markers):
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.02)
        process.send_signal(signal.SIGTERM)
        self.assertEqual(process.wait(timeout=5), 0)
        for marker in markers:
            self.assertIn('stop', marker.read_text())

    def test_missing_secret_fails_before_starting_services(self):
        self.env.pop('GL_WEB_SESSION_SECRET')
        self.assertEqual(self.start().wait(timeout=5), 1)
        self.assertFalse((self.root / 'geoledger-server.events').exists())

    def test_missing_backend_executable_fails_and_reaps_console(self):
        (self.root / 'geoledger-server').unlink()
        self.assertEqual(self.start().wait(timeout=5), 1)

    def test_invalid_shutdown_deadline_fails_before_starting_services(self):
        self.env['GL_SHUTDOWN_TIMEOUT_SECS'] = 'invalid'
        self.assertEqual(self.start().wait(timeout=5), 1)
        self.assertFalse((self.root / 'geoledger-server.events').exists())


if __name__ == '__main__':
    unittest.main()
