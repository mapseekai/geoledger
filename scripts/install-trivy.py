#!/usr/bin/env python3
"""Install a checksum-pinned scanner in an explicit directory, never globally."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import tarfile
import tempfile
import urllib.request


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination', type=Path, required=True)
    args = parser.parse_args()
    lock = json.loads((Path(__file__).resolve().parents[1] / '.security/trivy.lock.json').read_text())
    arch = platform.machine().lower()
    arch = {'arm64': 'aarch64', 'amd64': 'x86_64'}.get(arch, arch)
    system = platform.system().lower()
    if system == 'darwin' and arch == 'aarch64':
        arch = 'arm64'
    asset = lock['assets'][f'{system}-{arch}']
    args.destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='trivy-install-', dir=args.destination) as temporary:
        archive = Path(temporary) / 'scanner.tgz'
        with urllib.request.urlopen(asset['url'], timeout=90) as response, archive.open('wb') as output:
            while block := response.read(1024 * 1024):
                output.write(block)
        if hashlib.sha256(archive.read_bytes()).hexdigest() != asset['sha256']:
            raise ValueError('Trivy archive checksum mismatch')
        with tarfile.open(archive) as source:
            member = source.getmember('trivy')
            if not member.isfile():
                raise ValueError('scanner is not a regular file')
            stream = source.extractfile(member)
            if stream is None:
                raise ValueError('scanner missing')
            binary = Path(temporary) / 'trivy'
            with binary.open('wb') as output:
                while block := stream.read(1024 * 1024):
                    output.write(block)
        binary.chmod(0o755)
        binary.replace(args.destination / 'trivy')
    if path := os.environ.get('GITHUB_PATH'):
        with open(path, 'a', encoding='utf-8') as output:
            output.write(str(args.destination.resolve()) + '\n')
    print(f'Installed Trivy {lock["version"]}: {args.destination.resolve() / "trivy"}')


if __name__ == '__main__':
    main()
