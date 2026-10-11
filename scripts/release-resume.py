#!/usr/bin/env python3
"""Resume immutable SDK/Release publication; an existing mismatched artifact fails closed."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import urllib.error
import urllib.request
import zipfile


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def gh(path):
    result = subprocess.run(['gh', 'api', path], text=True, capture_output=True)
    if result.returncode:
        if '(HTTP 404)' in result.stderr:
            return None
        raise RuntimeError(f'GitHub request failed: {path}')
    return json.loads(result.stdout)


def fetch(url, optional=False):
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read(100 * 1024 * 1024 + 1)
            if len(data) > 100 * 1024 * 1024:
                raise ValueError('SDK artifact exceeds 100 MiB')
            return data
    except urllib.error.HTTPError as error:
        if optional and error.code == 404:
            return None
        raise


def contents(data, wheel=False):
    # Compare payloads and permissions; rebuilt archives may differ only in timestamps/compression.
    result = {}
    if wheel:
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            entries = [(entry.filename, archive.read(entry), entry.external_attr >> 16) for entry in archive.infolist() if not entry.is_dir()]
    else:
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            entries = []
            for entry in archive:
                if entry.isdir():
                    continue
                if not entry.isfile():
                    raise ValueError('SDK archive contains a non-file entry')
                entries.append((entry.name, archive.extractfile(entry).read(), entry.mode))
    for name, data, mode in entries:
        if name in result:
            raise ValueError('duplicate archive entry')
        result[name] = (hashlib.sha256(data).hexdigest(), mode)
    if not result:
        raise ValueError('empty SDK archive')
    return result


def same_archive(path, data):
    if contents(path.read_bytes(), path.suffix == '.whl') != contents(data, path.suffix == '.whl'):
        raise ValueError(f'published package differs: {path.name}')


def npm(path, version):
    body = fetch('https://registry.npmjs.org/@geoledger%2fclient/' + version, optional=True)
    if body is not None:
        metadata = json.loads(body)
        same_archive(path, fetch(metadata['dist']['tarball']))
        print('Verified existing npm package')
    else:
        subprocess.run(['npm', 'publish', str(path), '--provenance', '--access', 'public', '--tag', 'next' if '-' in version else 'latest'], check=True)


def pypi(directory, version):
    files = sorted(directory.iterdir())
    if not files or any(p.suffix != '.whl' and not p.name.endswith('.tar.gz') for p in files):
        raise ValueError('expected Python wheel and source archive')
    body = fetch('https://pypi.org/pypi/geoledger-client/' + version + '/json', optional=True)
    published = {entry['filename']: entry for entry in json.loads(body)['urls']} if body else {}
    for path in files:
        if entry := published.get(path.name):
            data = fetch(entry['url'])
            if hashlib.sha256(data).hexdigest() != entry['digests']['sha256']:
                raise ValueError('PyPI artifact digest mismatch')
            same_archive(path, data)
            path.unlink()
    pending = any(directory.iterdir())
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write(f'pending={str(pending).lower()}\n')


def tag_sha(repo, tag):
    ref = gh(f'repos/{repo}/git/ref/tags/{tag}')
    if ref is None:
        return None
    obj = ref['object']
    for _ in range(8):
        if obj['type'] == 'commit':
            return obj['sha']
        if obj['type'] != 'tag':
            break
        obj = gh(f'repos/{repo}/git/tags/{obj["sha"]}')['object']
    raise ValueError('tag does not resolve to a commit')


def go(repo, tag, sha):
    actual = tag_sha(repo, 'sdk/go/' + tag)
    if actual is None:
        run('gh', 'api', f'repos/{repo}/git/refs', '-f', 'ref=refs/tags/sdk/go/' + tag, '-f', 'sha=' + sha)
    elif actual != sha:
        raise ValueError('existing Go tag points to a different commit')


def release(repo, tag, sha, directory):
    files = sorted(directory.iterdir())
    if not files or not all(path.is_file() for path in files):
        raise ValueError('release assets must be nonempty files')
    if tag_sha(repo, tag) != sha:
        raise ValueError('release tag differs from candidate commit')
    existing = gh(f'repos/{repo}/releases/tags/{tag}')
    if existing is None:
        args = ['gh', 'release', 'create', tag, '--draft', '--verify-tag', '--title', 'GeoLedger ' + tag[1:], '--notes-file', 'notes.md', '--repo', repo]
        if '-' in tag:
            args.append('--prerelease')
        run(*args)
        existing = gh(f'repos/{repo}/releases/tags/{tag}')
    assets = gh(f'repos/{repo}/releases/{existing["id"]}/assets?per_page=100')
    if len(assets) == 100:
        raise ValueError('unexpected release asset count')
    assets = {asset['name']: asset for asset in assets}
    for path in files:
        if asset := assets.get(path.name):
            data = subprocess.check_output(['gh', 'api', f'repos/{repo}/releases/assets/{asset["id"]}', '-H', 'Accept: application/octet-stream'])
            if hashlib.sha256(data).digest() != hashlib.sha256(path.read_bytes()).digest():
                raise ValueError(f'existing release asset differs: {path.name}')
        elif existing['draft']:
            run('gh', 'release', 'upload', tag, str(path), '--repo', repo)
        else:
            raise ValueError('published release is incomplete')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kind', choices=['release', 'go', 'npm', 'pypi'])
    parser.add_argument('--path', type=Path)
    args = parser.parse_args()
    tag = os.environ['GITHUB_REF_NAME']
    if args.kind == 'npm':
        npm(args.path, tag[1:])
    elif args.kind == 'pypi':
        version = tag[1:].replace('-alpha.', 'a').replace('-beta.', 'b').replace('-rc.', 'rc')
        pypi(args.path, version)
    elif args.kind == 'go':
        go(os.environ['GITHUB_REPOSITORY'], tag, os.environ['GITHUB_SHA'])
    else:
        release(os.environ['GITHUB_REPOSITORY'], tag, os.environ['GITHUB_SHA'], args.path)


if __name__ == '__main__':
    main()
