#!/usr/bin/env python3
"""Recover only the reviewed alpha.4 publication; never rebuild or push an image."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import urllib.request
import zipfile

REPO = 'mapseekai/geoledger'
RUN = 38732087093
SHA = '6dccccd5ebc655f520a08c374823cc1389651f81'
TAG = 'v0.3.0-alpha.4'
DIGEST = 'sha256:15474870959a39598366f6fb455044e7e5bcd06b222c71e2dd60f54646dd1b3b'
IMAGE = 'docker.io/zwishing/geoledger@' + DIGEST
ARTIFACTS = {
    'image-inventory-geoledger': (11974096924, 'sha256:9808587c06f35403722ec4a3f4ab939ca4bf72e9ff58e2296fe4a54a31162c26', 'geoledger-images.json'),
    'source-release-performance': (11971966391, 'sha256:e1061a2cf2880560a7b4c00ed8763f97bf205a3936b837635b53c5a3885d212c', 'performance.zip'),
}
SUCCEEDED = {'validate', 'performance', 'images / merge-scan', 'images / publish (geoledger)'}
SUCCEEDED.update('quality / ' + name for name in ('source', 'postgis-release', 'linux', 'web', 'msrv', 'supply-chain', 'windows'))
SUCCEEDED.update(f'images / build-scan (geoledger, combined, Dockerfile, {arch}, {runner})'
                 for arch, runner in (('amd64', 'ubuntu-24.04'), ('arm64', 'ubuntu-24.04-arm')))


def validate_run(run, jobs):
    expected = {'id': RUN, 'head_sha': SHA, 'path': '.github/workflows/source-docker-release.yml',
                'event': 'workflow_dispatch', 'head_branch': 'main', 'status': 'completed',
                'conclusion': 'failure', 'run_attempt': 1}
    if any(run.get(k) != v for k, v in expected.items()) or run.get('repository', {}).get('full_name') != REPO:
        raise ValueError('not the reviewed original release run')
    expected_jobs = dict.fromkeys(SUCCEEDED, 'success')
    expected_jobs.update({'acceptance (amd64, ubuntu-24.04)': 'failure',
                          'acceptance (arm64, ubuntu-24.04-arm)': 'failure', 'complete': 'skipped'})
    records = jobs['jobs']
    if (jobs['total_count'] != len(records) or len(records) != len(expected_jobs)
            or {j['name']: j['conclusion'] for j in records} != expected_jobs
            or any(j['status'] != 'completed' or j['run_id'] != RUN or j['head_sha'] != SHA for j in records)):
        raise ValueError('original quality, performance or publication gates did not pass')


def validate_artifacts(response):
    records = response['artifacts']
    if response['total_count'] != len(records):
        raise ValueError('incomplete artifact listing')
    for name, (identity, digest, _) in ARTIFACTS.items():
        found = [a for a in records if a['name'] == name]
        if (len(found) != 1 or found[0]['id'] != identity or found[0].get('digest') != digest
                or found[0]['expired'] or found[0]['workflow_run']['id'] != RUN
                or found[0]['workflow_run']['head_sha'] != SHA):
            raise ValueError('original artifact missing, expired or replaced: ' + name)


def unpack(data, digest, filename, output):
    if 'sha256:' + hashlib.sha256(data).hexdigest() != digest:
        raise ValueError('downloaded artifact checksum differs')
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        if archive.namelist() != [filename] or archive.getinfo(filename).file_size > 50 * 1024 * 1024:
            raise ValueError('unexpected artifact contents')
        # Write only the exact expected filename, never archive-controlled paths.
        (output / filename).write_bytes(archive.read(filename))


def verify_tag():
    with urllib.request.urlopen('https://auth.docker.io/token?service=registry.docker.io&scope=repository:zwishing/geoledger:pull', timeout=60) as response:
        token = json.load(response)['token']
    request = urllib.request.Request('https://registry-1.docker.io/v2/zwishing/geoledger/manifests/' + TAG[1:], headers={
        'Authorization': 'Bearer ' + token, 'Accept': 'application/vnd.oci.image.index.v1+json'})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read(1024 * 1024 + 1)
    if 'sha256:' + hashlib.sha256(data).hexdigest() != DIGEST:
        raise ValueError('Docker tag no longer identifies the reviewed image')


def evidence():
    return {'run_id': RUN, 'source_sha': SHA, 'tag': TAG, 'image': IMAGE,
            'artifacts': {name: {'id': value[0], 'digest': value[1]} for name, value in ARTIFACTS.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['prepare', 'verify'])
    parser.add_argument('--assets', type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get('GITHUB_REPOSITORY') != REPO or os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('recovery must run in the authorized repository on main')
    if args.command == 'prepare':
        def api(path):
            return subprocess.check_output(['gh', 'api', f'repos/{REPO}/' + path])
        base = f'actions/runs/{RUN}'
        validate_run(json.loads(api(base)), json.loads(api(base + '/attempts/1/jobs?per_page=100')))
        validate_artifacts(json.loads(api(base + '/artifacts?per_page=100')))
        args.assets.mkdir(parents=True, exist_ok=False)
        for identity, digest, filename in ARTIFACTS.values():
            unpack(api(f'actions/artifacts/{identity}/zip'), digest, filename, args.assets)
        import importlib.util
        spec = importlib.util.spec_from_file_location('source_release', Path(__file__).with_name('source-release.py'))
        release = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(release)
        if release.inventory(args.assets, SHA)['image'] != IMAGE:
            raise ValueError('original inventory differs from the reviewed registry image')
        (args.assets / 'recovery.json').write_text(json.dumps(evidence(), indent=2) + '\n')
    elif json.loads((args.assets / 'recovery.json').read_text()) != evidence():
        raise ValueError('recovery evidence differs')
    verify_tag()


if __name__ == '__main__':
    main()
