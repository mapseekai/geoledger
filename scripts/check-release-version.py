#!/usr/bin/env python3
"""Validate every publishable package against an explicit release tag."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--tag', required=True)
args = parser.parse_args()
if not re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)\.[0-9]+)?', args.tag):
    raise SystemExit('release tag must be vX.Y.Z or vX.Y.Z-alpha/beta/rc.N')
version = args.tag[1:]
python_version = re.sub(r'-alpha\.', 'a', version)
python_version = re.sub(r'-beta\.', 'b', python_version)
python_version = re.sub(r'-rc\.', 'rc', python_version)
versions = {
    'Cargo.toml': tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version'],
    'sdk/ts/package.json': json.loads(Path('sdk/ts/package.json').read_text())['version'],
    'web/package.json': json.loads(Path('web/package.json').read_text())['version'],
}
for path, actual in versions.items():
    if actual != version:
        raise SystemExit(f'{path}: expected {version}, got {actual}')
if tomllib.loads(Path('sdk/python/pyproject.toml').read_text())['project']['version'] != python_version:
    raise SystemExit('Python package version does not match release')
if not any(line == f'## [{version}]' or line.startswith(f'## [{version}] ') for line in Path('CHANGELOG.md').read_text().splitlines()):
    raise SystemExit('CHANGELOG has no exact release section')
sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
if path := os.environ.get('GITHUB_OUTPUT'):
    with open(path, 'a') as output:
        output.write(f'version={version}\nsha={sha}\n')
print(f'Validated all package versions: {version}; source: {sha}')
