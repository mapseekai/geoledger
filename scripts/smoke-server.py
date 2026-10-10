#!/usr/bin/env python3
"""Exercise a packaged binary: native spatial write, publish, bbox read and restart.

Only uses loopback ephemeral listeners and a temporary data directory. No business
connection settings or caller credentials are inherited. No tokens are logged.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid


def port() -> int:
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def exercise(binary: Path, runtime: Path | None) -> None:
    env = {k: v for k, v in os.environ.items() if not k.startswith('GL_')}
    if runtime:
        env['GL_SPATIALITE_EXTENSION'] = str(runtime / 'bin/mod_spatialite.dll')
        env['PATH'] = str(runtime / 'bin') + os.pathsep + env['PATH']
        env['PROJ_DATA'] = str(runtime / 'share/proj')
    elif 'GL_SPATIALITE_EXTENSION' in os.environ:
        env['GL_SPATIALITE_EXTENSION'] = os.environ['GL_SPATIALITE_EXTENSION']
    env['GL_SHUTDOWN_TIMEOUT_SECS'] = '5'
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with tempfile.TemporaryDirectory(prefix='geoledger-binary-smoke-') as temporary:
        root = Path(temporary)
        selected: set[int] = set()
        while len(selected) < 2:
            selected.add(port())
        http, grpc = sorted(selected)
        base = f'http://127.0.0.1:{http}'
        command = [str(binary), '--data-dir', str(root / 'data'), '--http', f'127.0.0.1:{http}', '--grpc', f'127.0.0.1:{grpc}']
        project = dataset = None
        for restart in (False, True):
            with (root / 'server.log').open('ab') as log:
                child = subprocess.Popen(command, env=env, stdout=log, stderr=log)
                try:
                    until = time.monotonic() + 30
                    while True:
                        if child.poll() is not None:
                            raise RuntimeError(f'server exited during startup with code {child.returncode}')
                        try:
                            with opener.open(base + '/ready', timeout=1) as response:
                                if response.status == 200:
                                    break
                        except (urllib.error.URLError, TimeoutError):
                            pass
                        if time.monotonic() >= until:
                            raise TimeoutError('server never became ready')
                        time.sleep(0.1)
                    credentials = json.loads((root / 'data/admin-credentials.json').read_text())
                    token = credentials[0]['token']
                    def call(operation: str, body: dict):
                        request = urllib.request.Request(base + '/api/v1/' + operation, data=json.dumps(body).encode(), headers={'Content-Type': 'application/json', 'Authorization': 'Bearer ' + token})
                        try:
                            with opener.open(request, timeout=10) as response:
                                return json.load(response)
                        except urllib.error.HTTPError as error:
                            raise RuntimeError(f'{operation} returned HTTP {error.code}') from None
                    if not restart:
                        project = call('create_project', {'name': 'packaged spatial smoke'})['project']
                        dataset = call('create_dataset', {'project': project, 'name': 'points', 'geometry_type': 'point', 'coordinate_dimension': 3})['dataset']
                        workspace = call('create_workspace', {'project': project})['workspace']
                        feature = {'type': 'Feature', 'id': 'native-point', 'properties': {'exact': 9007199254740993}, 'geometry': {'type': 'Point', 'coordinates': [120, 30, 12]}}
                        saved = call('save', {'project': project, 'workspace': workspace, 'expected_workspace_version': 0, 'edits': [{'dataset': dataset, 'feature_id': feature['id'], 'feature': feature}]})
                        published = call('publish', {'project': project, 'workspace': workspace, 'expected_workspace_version': saved['version'], 'request_id': str(uuid.uuid4()), 'message': 'native smoke'})
                        if published['changes'] != 1:
                            raise AssertionError('publication did not contain one feature')
                    page = call('features', {'project': project, 'dataset': dataset, 'bbox': [119, 29, 121, 31]})
                    items = page['features']
                    if len(items) != 1 or items[0]['geometry']['coordinates'] != [120, 30, 12] or items[0]['properties']['exact'] != 9007199254740993:
                        raise AssertionError('native spatial query lost data')
                    print('PASS packaged startup, native XYZ write/read and persistence' if restart else 'PASS native spatial publication')
                finally:
                    if child.poll() is None:
                        child.terminate()
                    try:
                        child.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--runtime', type=Path)
    args = parser.parse_args()
    exercise(args.binary.resolve(), args.runtime.resolve() if args.runtime else None)


if __name__ == '__main__':
    main()
