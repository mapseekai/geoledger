#!/usr/bin/env python3
"""Package a tested Windows CLI with an explicit file allowlist and SHA-256 checksums."""
from __future__ import annotations
import argparse
import hashlib
import json
import re
from pathlib import Path
import shutil
import subprocess
import tomllib
import zipfile
from urllib.parse import quote, unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]


def render_document(source: Path, names: dict[Path, str], commit: str) -> str:
    """Keep guide links local; pin source references to the packaged revision."""
    def rewrite(match: re.Match[str]) -> str:
        target = urlsplit(match.group(2))
        if target.scheme:
            return match.group(0)
        path = (source.parent / unquote(target.path)).resolve() if target.path else source.resolve()
        relative = path.relative_to(ROOT).as_posix()
        if not path.is_file():
            raise ValueError(f'Missing documentation target: {relative}')
        destination = names.get(path)
        if destination is None:
            destination = f'https://github.com/mapseekai/geoledger/blob/{commit}/{quote(relative)}'
        if target.fragment:
            destination += '#' + target.fragment
        return match.group(1) + destination + match.group(3)
    return re.sub(r'(\[[^\]\n]+\]\()([^\s)]+)(\))', rewrite, source.read_text(encoding='utf-8'))

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
        ROOT / 'README.md': 'README.md',
        ROOT / 'docs/getting-started.md': 'README-WINDOWS.md',
        ROOT / 'docs/user-guide.md': 'USER-GUIDE.md',
        ROOT / 'docs/api.md': 'API.md',
        ROOT / 'docs/development.md': 'DEVELOPMENT.md',
        ROOT / 'crates/server/proto/geoledger.proto': 'geoledger.proto',
        ROOT / 'crates/thrift-gen/idl/geoledger.thrift': 'geoledger.thrift',
        ROOT / 'LICENSE': 'LICENSE',
        ROOT / '.env.example': 'config.env.example',
        ROOT / 'scripts/smoke-windows.ps1': 'smoke-test.ps1',
        ROOT / 'scripts/windows-test-data.sql': 'test-data.sql',
    }
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    names = {source.resolve(): destination for source, destination in sources.items()}
    for source, destination in sources.items():
        if source.suffix == '.md':
            (stage / destination).write_text(render_document(source, names, commit), encoding='utf-8')
        else:
            shutil.copyfile(source, stage / destination)
    info = {
        'version': version,
        'commit': commit,
        'target': 'x86_64-pc-windows-msvc',
        'profile': 'release',
        'features': ['cli', 'http', 'grpc'],
        'format_version': 3,
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
