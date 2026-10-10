#!/usr/bin/env python3
"""Prepare an isolated, SHA256-pinned OSGeo4W runtime; no registry or system changes.

Extraction is testable on any OS. DLL loading is checked by smoke-server.py on Windows.
The installer never runs upstream post-install scripts. Keep the dependency manifest
and sources with deployment records; they describe exactly which packages were used.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import tarfile
import tempfile
import urllib.request


def extract(source: Path, target: Path) -> None:
    with tarfile.open(source, 'r:bz2') as archive:
        for member in archive:
            name = PurePosixPath(member.name)
            if name.is_absolute() or '..' in name.parts or '\\' in member.name or ':' in member.name:
                raise ValueError(f'unsafe archive path: {member.name}')
            destination = target.joinpath(*name.parts)
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                destination.parent.mkdir(parents=True, exist_ok=True)
                stream = archive.extractfile(member)
                if stream is None:
                    raise ValueError('missing archive data')
                with destination.open('wb') as output:
                    shutil.copyfileobj(stream, output)
            else:
                raise ValueError(f'unsupported archive member: {member.name}')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--cache-dir', type=Path)
    packaged = Path(__file__).resolve().with_name('windows-spatialite.lock.json')
    default = packaged if packaged.exists() else Path(__file__).resolve().parents[1] / '.security/windows-spatialite.lock.json'
    parser.add_argument('--manifest', type=Path, default=default)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    if manifest['schema'] != 1 or manifest['platform'] != 'windows-x86_64':
        raise ValueError('unexpected runtime manifest')
    destination = args.destination.resolve()
    if destination.exists():
        raise ValueError('destination must be new; refusing to overwrite an existing runtime')
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='spatialite-install-', dir=destination.parent) as temporary:
        stage = Path(temporary) / 'runtime'
        stage.mkdir()
        cache = args.cache_dir or Path(temporary) / 'downloads'
        cache.mkdir(parents=True, exist_ok=True)
        names = {p['name'] for p in manifest['packages']}
        for package in manifest['packages']:
            if not set(package['requires']) <= names:
                raise ValueError(f'missing dependencies for {package["name"]}')
            url = package['url']
            if not url.startswith('https://download.osgeo.org/osgeo4w/v2/'):
                raise ValueError('untrusted download origin')
            archive = cache / url.rsplit('/', 1)[1]
            if not archive.exists():
                with urllib.request.urlopen(url, timeout=90) as response, archive.open('wb') as output:
                    shutil.copyfileobj(response, output)
            with archive.open('rb') as source:
                sha = hashlib.sha256()
                while data := source.read(1024 * 1024):
                    sha.update(data)
                digest = sha.hexdigest()
            if archive.stat().st_size != package['bytes'] or digest != package['sha256']:
                raise ValueError(f'checksum mismatch: {package["name"]}')
            extract(archive, stage)
            print(f'Verified {package["name"]} {package["version"]}', flush=True)
        if not (stage / 'bin/mod_spatialite.dll').is_file():
            raise ValueError('mod_spatialite.dll is missing from verified runtime')
        shutil.copyfile(args.manifest, stage / 'geoledger-runtime.lock.json')
        stage.replace(destination)
    extension = destination / 'bin/mod_spatialite.dll'
    if os.name == 'nt':
        if output := os.environ.get('GITHUB_ENV'):
            with open(output, 'a', encoding='utf-8') as env:
                env.write(f'GL_SPATIALITE_EXTENSION={extension}\nPROJ_DATA={destination / "share/proj"}\n')
        if output := os.environ.get('GITHUB_PATH'):
            with open(output, 'a', encoding='utf-8') as env:
                env.write(f'{destination / "bin"}\n')
    print(f'Runtime prepared: {destination}\nGL_SPATIALITE_EXTENSION={extension}')


if __name__ == '__main__':
    main()
