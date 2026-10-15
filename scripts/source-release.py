#!/usr/bin/env python3
"""Validate and finalize the explicitly scoped source/Docker prerelease."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess


def load(filename):
    spec = importlib.util.spec_from_file_location(filename, Path(__file__).with_name(filename + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validate_identity(sha, tag):
    if not re.fullmatch(r'[0-9a-f]{40}', sha):
        raise ValueError('full source SHA required')
    if not re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+-(?:alpha|beta|rc)\.[0-9]+', tag):
        raise ValueError('this workflow only publishes prereleases')


def inventory(directory, sha):
    records = [json.loads(p.read_text()) for p in directory.glob('*-images.json')]
    load('audit-published-images').validate_inventories(records, sha)
    record = records[0]
    if not re.fullmatch(r'docker\.io/zwishing/geoledger@sha256:[0-9a-f]{64}', record['image']):
        raise ValueError('unexpected publication destination')
    if sorted(record.get('platforms', [])) != ['linux/amd64', 'linux/arm64']:
        raise ValueError('both Linux architectures are required')
    return record


def validate_acceptance(directory, record, version):
    for arch in ('amd64', 'arm64'):
        result = json.loads((directory / f'acceptance-{arch}.json').read_text())
        expected = {'source_sha': record['source_sha'], 'image': record['image'],
                    'platform': 'linux/' + arch, 'version': version, 'passed': True}
        if result != expected or result.get('passed') is not True:
            raise ValueError('image acceptance differs from candidate: ' + arch)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['preflight', 'complete'])
    parser.add_argument('--sha', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--assets', type=Path)
    args = parser.parse_args()
    validate_identity(args.sha, args.tag)
    repo = os.environ['GITHUB_REPOSITORY']
    if repo != 'mapseekai/geoledger':
        raise ValueError('unexpected repository')
    if os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('dispatch this workflow from main')
    resume = load('release-resume')
    if resume.run('git', 'rev-parse', 'HEAD') != args.sha:
        raise ValueError('checkout differs from candidate')
    subprocess.run(['git', 'merge-base', '--is-ancestor', args.sha, 'origin/main'], check=True)
    subprocess.run(['python3', 'scripts/check-release-version.py', '--tag', args.tag], check=True)
    actual = resume.tag_sha(repo, args.tag)
    if actual not in (None, args.sha):
        raise ValueError('existing tag points to another commit')
    existing = resume.gh(f'repos/{repo}/releases/tags/{args.tag}')
    if existing and (not existing['draft'] or not existing['prerelease']):
        raise ValueError('existing public release must not be changed')
    if args.command == 'preflight':
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write(f'sha={args.sha}\nversion={args.tag[1:]}\n')
        return
    if args.assets is None:
        parser.error('--assets is required for completion')
    record = inventory(args.assets, args.sha)
    validate_acceptance(args.assets, record, args.tag[1:])
    text = Path('CHANGELOG.md').read_text()
    section = re.search(r'^## \[' + re.escape(args.tag[1:]) + r'\][^\n]*\n(.*?)(?=^## \[|\Z)', text, re.M | re.S)
    if section is None:
        raise ValueError('missing release notes')
    notes = section[1].strip() + '\n\n### Distribution\n\n'
    notes += f'- Source: `{args.sha}`\n- Image: `{record["image"]}`\n'
    notes += f'- Tag: `zwishing/geoledger:{args.tag[1:]}`; Linux AMD64 / ARM64.\n'
    notes += '- This prerelease distributes source and the combined Docker image only; npm/PyPI packages and standalone binaries are not published.\n'
    if (args.assets / 'recovery.json').exists():
        recovery = load('recover-source-release')
        if (json.loads((args.assets / 'recovery.json').read_text()) != recovery.evidence()
                or args.sha != recovery.SHA or args.tag != recovery.TAG or record['image'] != recovery.IMAGE):
            raise ValueError('recovery evidence differs from the approved publication')
        notes += f'- Same-SHA CI, performance acceptance, build/runtime scans and signing passed in https://github.com/{repo}/actions/runs/{recovery.RUN}.\n'
        notes += '- This recovery workflow verified the original artifact checksums, exact signing identity and unchanged registry tag, then completed anonymous native digest-pull SDK smoke checks on both architectures.\n'
    else:
        notes += '- Same-SHA CI, performance acceptance, build/runtime scans, signing and native digest-pull SDK smoke checks completed in this workflow.\n'
    Path('notes.md').write_text(notes)
    if actual is None:
        # The workflow GITHUB_TOKEN prevents recursive SDK tag workflows.
        resume.run('gh', 'api', f'repos/{repo}/git/refs', '-f', 'ref=refs/tags/' + args.tag, '-f', 'sha=' + args.sha)
    resume.release(repo, args.tag, args.sha, args.assets)
    resume.run('gh', 'release', 'edit', args.tag, '--draft=false', '--prerelease', '--repo', repo)


if __name__ == '__main__':
    main()
