#!/usr/bin/env python3
"""Reaudit exact image digests from a published Release's machine-readable inventories."""
from __future__ import annotations
import argparse
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import tempfile


def validate_inventories(records, source_sha=None):
    if len(records) != 2 or {r.get('kind') for r in records} != {'server', 'console'}:
        raise ValueError('release requires server and console image inventories')
    shas = {r.get('source_sha') for r in records}
    if len(shas) != 1 or not all(re.fullmatch(r'[0-9a-f]{40}', value or '') for value in shas):
        raise ValueError('image inventories must share one immutable source SHA')
    if source_sha and shas != {source_sha}:
        raise ValueError('image source differs from release source')
    for record in records:
        if not re.fullmatch(r'(ghcr\.io|docker\.io)/[a-z0-9_.\-/]+@sha256:[0-9a-f]{64}', record.get('image', '')):
            raise ValueError('inventory must identify an immutable registry digest')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--tag', default='')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repository):
        raise ValueError('invalid repository')
    tag = args.tag
    if not tag:
        releases = json.loads(subprocess.check_output(['gh', 'release', 'list', '--repo', args.repository, '--exclude-drafts', '--limit', '1', '--json', 'tagName'], text=True, timeout=60))
        if not releases:
            raise ValueError('no release exists; nothing was audited')
        tag = releases[0]['tagName']
    if not re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9][A-Za-z0-9.-]*)?', tag):
        raise ValueError('invalid release tag')
    source_sha = subprocess.check_output(['gh', 'api', f'repos/{args.repository}/commits/{tag}', '--jq', '.sha'], text=True, timeout=60).strip()
    if not re.fullmatch(r'[0-9a-f]{40}', source_sha):
        raise ValueError('release tag did not resolve to an immutable source SHA')
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'audit-status.json').write_text(json.dumps({'tag': tag, 'status': 'incomplete', 'reason': 'inventory download or scan has not completed'}, indent=2))
    spec = importlib.util.spec_from_file_location('image_gate', Path(__file__).with_name('image-gate.py'))
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    with tempfile.TemporaryDirectory(prefix='geoledger-published-audit-') as temporary:
        root = Path(temporary)
        manifests = root / 'inventories'; manifests.mkdir()
        subprocess.run(['gh', 'release', 'download', tag, '--repo', args.repository, '--pattern', '*-images.json', '--dir', str(manifests)], check=True, timeout=90)
        files = sorted(manifests.glob('*-images.json'))
        records = [json.loads(p.read_text()) for p in files]
        validate_inventories(records, source_sha)
        failures = []
        for record in records:
            image = record['image']
            if not re.fullmatch(r'(ghcr\.io|docker\.io)/[a-z0-9_.\-/]+@sha256:[0-9a-f]{64}', image):
                raise ValueError('release inventory does not identify an immutable image')
            kind = record['kind']
            layout = root / kind
            try:
                subprocess.run(['skopeo', 'copy', '--all', '--preserve-digests', 'docker://' + image, 'oci:' + str(layout)], check=True, timeout=600)
                digest, _ = gate.inventory(layout)
                if digest != image.rsplit('@', 1)[1]:
                    raise ValueError('pulled image digest differs from release inventory')
                gate.scan(layout, args.output / kind, kind, gate.DEFAULT_PLATFORMS, record['source_sha'])
            except (ValueError, OSError, subprocess.SubprocessError) as error:
                failures.append(f'{kind}: {error}')
        if failures:
            raise RuntimeError('; '.join(failures))
    (args.output / 'audit-status.json').write_text(json.dumps({'tag': tag, 'status': 'passed'}, indent=2))
    print(f'PASS published {tag}: both image digests and both architectures audited')


if __name__ == '__main__':
    main()
