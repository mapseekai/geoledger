#!/usr/bin/env python3
"""Pull and exercise one native architecture of the approved combined image."""
import argparse
import json
import os
from pathlib import Path
import platform
import secrets
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

from auth_test_support import private_file, provision
import importlib.util


def run(*command):
    return subprocess.check_output(command, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--sha', required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--arch', choices=['amd64', 'arm64'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('source_release', Path(__file__).with_name('source-release.py'))
    release = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(release)
    release.validate_identity(args.sha, 'v' + args.version)
    record = release.inventory(args.inventory, args.sha)
    if platform.machine() != {'amd64': 'x86_64', 'arm64': 'aarch64'}[args.arch]:
        raise ValueError('native architecture runner required')
    image = record['image']
    run('docker', 'pull', '--platform', 'linux/' + args.arch, image)
    inspected = json.loads(run('docker', 'image', 'inspect', image))[0]
    if inspected['Architecture'] != args.arch or inspected['Os'] != 'linux':
        raise ValueError('pulled image platform differs')
    if inspected['Config'].get('Labels', {}).get('org.opencontainers.image.revision') != args.sha:
        raise ValueError('pulled image source revision differs')
    if run('docker', 'run', '--rm', '--entrypoint', 'geoledger-server', image, '--version') != 'geoledger-server ' + args.version:
        raise ValueError('pulled server version differs')
    name = 'geoledger-release-smoke-' + uuid.uuid4().hex[:12]
    with tempfile.TemporaryDirectory(prefix='geoledger-release-smoke-') as temporary:
        root = Path(temporary)
        password = secrets.token_urlsafe(32)
        private_file(root / 'initial-admin-test.json', json.dumps({'username': 'admin', 'password': password}))
        private_file(root / 'container.env', 'GL_ADMIN_PASSWORD=' + password + '\nGL_WEB_SESSION_SECRET=' + secrets.token_urlsafe(48) + '\n')
        try:
            run('docker', 'run', '--detach', '--name', name, '--env-file', str(root / 'container.env'),
                '-p', '127.0.0.1::7881', '-p', '127.0.0.1::7882', image)
            for restart in (False, True):
                if restart:
                    run('docker', 'restart', name)
                # Docker can assign new ephemeral host ports on restart.
                http = run('docker', 'port', name, '7881/tcp').rsplit(':', 1)[1]
                grpc = run('docker', 'port', name, '7882/tcp').rsplit(':', 1)[1]
                deadline = time.monotonic() + 120
                while True:
                    status = json.loads(run('docker', 'inspect', name))[0]['State']
                    if not status['Running'] or status.get('Health', {}).get('Status') == 'unhealthy':
                        raise RuntimeError('combined image failed health check')
                    if status.get('Health', {}).get('Status') == 'healthy':
                        break
                    if time.monotonic() > deadline:
                        raise TimeoutError('combined image readiness timeout')
                    time.sleep(1)
                base = 'http://127.0.0.1:' + http
                with urllib.request.urlopen(base + '/ready', timeout=10) as response:
                    if response.status != 200:
                        raise ValueError('backend is not ready')
                if not restart:
                    provision(base, root, root / 'api-key')
                member_id = (root / 'api-key.member-id').read_text().strip()
                if not member_id:
                    raise ValueError('disposable SDK member ID is empty')
                env = {k: v for k, v in os.environ.items() if not k.startswith('GL_')}
                env.update(GL_ENDPOINT='http://127.0.0.1:' + grpc, GL_API_KEY_FILE=str(root / 'api-key'),
                           GL_TEST_MEMBER_ID=member_id)
                subprocess.run([sys.executable, 'sdk/python/tests/smoke.py'], env=env, check=True, timeout=180)
        finally:
            subprocess.run(['docker', 'rm', '-fv', name], check=True, stdout=subprocess.DEVNULL)
    args.output.write_text(json.dumps({'source_sha': args.sha, 'image': image,
                                     'platform': 'linux/' + args.arch, 'version': args.version, 'passed': True}, indent=2))


if __name__ == '__main__':
    main()
