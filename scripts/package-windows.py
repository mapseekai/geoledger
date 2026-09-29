#!/usr/bin/env python3
"""Package a tested Windows CLI with an explicit file allowlist and SHA-256 checksums."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'target' / 'dist')
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    if not args.binary.is_file() or args.binary.read_bytes()[:2] != b'MZ':
        raise SystemExit('Provide a compiled Windows PE executable.')
    args.output.mkdir(parents=True, exist_ok=True)
    name = f'geoledger-v{version}-windows-x86_64'
    stage = args.output / name
    if stage.exists():
        raise SystemExit(f'Output already exists: {stage}')
    stage.mkdir()
    sources = {
        args.binary: 'gl.exe',
        ROOT / 'docs/windows-testing.md': 'README-WINDOWS.md',
        ROOT / 'LICENSE': 'LICENSE',
        ROOT / '.env.example': 'config.env.example',
        ROOT / 'scripts/smoke-windows.ps1': 'smoke-test.ps1',
        ROOT / 'scripts/windows-test-data.sql': 'test-data.sql',
    }
    for source, destination in sources.items():
        shutil.copyfile(source, stage / destination)
    info = {
        'version': version,
        'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'target': 'x86_64-pc-windows-msvc',
        'profile': 'release',
        'features': ['cli', 'http', 'grpc'],
        'format_version': 3,
        'default_author': 'mapseekai',
        'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'c_runtime': 'static',
        'signature': 'unsigned',
    }
    (stage / 'build-info.json').write_text(json.dumps(info, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    hashes = []
    for item in sorted(stage.iterdir()):
        hashes.append(f'{hashlib.sha256(item.read_bytes()).hexdigest()}  {item.name}')
    (stage / 'SHA256SUMS.txt').write_text('\n'.join(hashes) + '\n', encoding='ascii')
    archive = args.output / f'{name}.zip'
    with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        for item in sorted(stage.iterdir()):
            output.write(item, arcname=f'{name}/{item.name}')
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix('.zip.sha256').write_text(f'{digest}  {archive.name}\n', encoding='ascii')
    print(json.dumps({'archive': str(archive), 'sha256': digest, 'bytes': archive.stat().st_size}))

if __name__ == '__main__':
    main()
