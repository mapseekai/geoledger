#!/usr/bin/env python3
"""One-time SDK supplement: reuse the published alpha.7 source and image exactly."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile

import importlib.util

ROOT = Path(__file__).resolve().parents[1]
REPO = 'mapseekai/geoledger'
TAG = 'v0.3.0-alpha.7'
VERSION = TAG[1:]
PYTHON_VERSION = '0.3.0a7'
SHA = 'f0109c8ff424dc974421072cb2ff9b366f12c17a'
IMAGE = 'docker.io/zwishing/geoledger@sha256:fdfb4222ca9414a0acb3820eb3e55f51204fe5423c8d761872cc65c70cc62a42'
GO = 'github.com/mapseekai/geoledger/sdk/go'


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


release = load('source-release')
resume = load('release-resume')


def prepare(source, assets):
    if os.environ.get('GITHUB_REPOSITORY', REPO) != REPO:
        raise ValueError('unexpected repository')
    if os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('dispatch the SDK supplement from main')
    if resume.run('git', '-C', str(source), 'rev-parse', 'HEAD') != SHA or resume.tag_sha(REPO, TAG) != SHA:
        raise ValueError('source checkout or public tag differs from alpha.7')
    subprocess.run([sys.executable, 'scripts/check-release-version.py', '--tag', TAG], cwd=source, check=True)
    record = resume.find_release(REPO, TAG)
    if not record or record['draft'] or not record['prerelease'] or record['tag_name'] != TAG:
        raise ValueError('published alpha.7 prerelease required')
    if resume.tag_sha(REPO, 'sdk/go/' + TAG) not in (None, SHA):
        raise ValueError('existing Go module tag differs')
    assets.mkdir(parents=True, exist_ok=True)
    for name in ('geoledger-images.json', 'acceptance-amd64.json', 'acceptance-arm64.json'):
        matches = [asset for asset in record['assets'] if asset['name'] == name]
        if len(matches) != 1:
            raise ValueError('missing or duplicate published asset: ' + name)
        asset = matches[0]
        data = resume.fetch(asset['browser_download_url'])
        if asset.get('digest') != 'sha256:' + hashlib.sha256(data).hexdigest():
            raise ValueError('published asset checksum differs: ' + name)
        (assets / name).write_bytes(data)
    image = release.inventory(assets, SHA)
    if image['image'] != IMAGE:
        raise ValueError('published image differs from alpha.7')
    release.validate_acceptance(assets, image, VERSION)
    load('image-gate').require_immutable_tags(['docker.io/zwishing/geoledger:' + VERSION], IMAGE.split('@')[1])
    print('Verified original tag, release, asset checksums, both architecture receipts and registry digest')
    return record


def verify_go(source, metadata, module):
    if module.get('Replace') or metadata.get('Path') != GO or metadata.get('Version') != module['Version']:
        raise ValueError('Go consumer must use the remote module without replace')
    files = resume.run('git', '-C', str(source), 'ls-tree', '-r', '--name-only', 'HEAD', 'sdk/go').splitlines()
    expected = {name.removeprefix('sdk/go/'): (source / name).read_bytes() for name in files}
    expected.setdefault('LICENSE', (source / 'LICENSE').read_bytes())
    prefix = GO + '@' + module['Version'] + '/'
    with zipfile.ZipFile(metadata['Zip']) as archive:
        names = [name for name in archive.namelist() if not name.endswith('/')]
        actual = {name.removeprefix(prefix): archive.read(name) for name in names}
    # Go proxies may omit Origin; compare every module file to the fixed source instead.
    if len(actual) != len(names) or actual != expected:
        raise ValueError('downloaded Go module differs from alpha.7 source')


def consumer(source, directory, packages, sdist):
    directory.mkdir(parents=True)
    env = dict(os.environ, GOWORK='off')
    for name in ('PYTHONPATH', 'PYTHONHOME', 'NODE_PATH', 'GOFLAGS'):
        env.pop(name, None)
    def run(*args, cwd=directory):
        return subprocess.check_output(args, cwd=cwd, env=env, text=True).strip()
    def one(pattern):
        paths = list(packages.glob(pattern))
        if len(paths) != 1:
            raise ValueError('expected one package matching ' + pattern)
        return str(paths[0].resolve())
    npm = one('sdk-packages/*.tgz') if packages else '@geoledger/client@' + VERSION
    python = one('sdk/python/dist/*.tar.gz' if sdist else 'sdk/python/dist/*.whl') if packages else 'geoledger-client==' + PYTHON_VERSION
    run('npm', 'init', '--yes')
    run('npm', 'install', '--ignore-scripts', '--save-exact', '--registry=https://registry.npmjs.org', npm)
    package = json.loads((directory / 'node_modules/@geoledger/client/package.json').read_text())
    if package['version'] != VERSION:
        raise ValueError('installed npm version differs')
    run(sys.executable, '-m', 'venv', str(directory / 'python'))
    interpreter = str(directory / 'python/bin/python')
    run(interpreter, '-m', 'pip', 'install', '--index-url', 'https://pypi.org/simple', python)
    actual = run(interpreter, '-c', 'import importlib.metadata; print(importlib.metadata.version("geoledger-client"))')
    if actual != PYTHON_VERSION:
        raise ValueError('installed Python version differs')
    go = directory / 'go'
    go.mkdir()
    run('go', 'mod', 'init', 'geoledger-release-consumer', cwd=go)
    run('go', 'get', GO + '@' + (SHA if packages else TAG), cwd=go)
    metadata = json.loads(run('go', 'mod', 'download', '-json', GO, cwd=go))
    module = json.loads(run('go', 'list', '-m', '-json', GO, cwd=go))
    verify_go(source, metadata, module)
    run('go', 'mod', 'verify', cwd=go)
    if not packages and module['Version'] != TAG:
        raise ValueError('installed Go module version differs')
    text = (source / 'sdk/ts/test/smoke.cjs').read_text()
    if text.count('require("../dist")') != 1:
        raise ValueError('unexpected TypeScript consumer fixture')
    (directory / 'smoke.cjs').write_text(text.replace('require("../dist")', 'require("@geoledger/client")'))
    shutil.copyfile(source / 'sdk/python/tests/smoke.py', directory / 'smoke.py')
    shutil.copyfile(ROOT / 'sdk/go/cmd/smoke/main.go', go / 'main.go')
    installed = {'npm': VERSION, 'pypi': actual, 'go': module['Version'], 'go_source_sha': SHA,
                 'from': 'packages' if packages else 'public registries'}
    (directory / 'installed.json').write_text(json.dumps(installed, indent=2))
    print(json.dumps(installed))


def complete(source, assets):
    record = prepare(source, assets / 'original')
    results = []
    for arch in ('amd64', 'arm64'):
        result = json.loads((assets / ('sdk-acceptance-' + arch + '.json')).read_text())
        expected = {'source_sha': SHA, 'image': IMAGE, 'platform': 'linux/' + arch,
                    'version': VERSION, 'passed': True,
                    'sdks': {'npm': VERSION, 'pypi': PYTHON_VERSION, 'go': TAG,
                             'go_source_sha': SHA, 'from': 'public registries'}}
        if result != expected or result['passed'] is not True:
            raise ValueError('installed SDK acceptance differs: ' + arch)
        results.append(result)
    body = record['body']
    old = '- This prerelease distributes source and the combined Docker image only; npm/PyPI packages and standalone binaries are not published.'
    new = '- SDK supplement: npm `@geoledger/client@' + VERSION + '`, PyPI `geoledger-client==' + PYTHON_VERSION + '`, Go `' + GO + '@' + TAG + '`. Standalone binaries are not published.'
    if body.count(old) == 1:
        body = body.replace(old, new)
        body += '\n- SDK packages installed from public registries and verified against the unchanged image on native AMD64 and ARM64, including CRUD, history, exact numbers, publication retry and restart.\n'
        body += '- SDK acceptance: https://github.com/' + REPO + '/actions/runs/' + os.environ['GITHUB_RUN_ID'] + '\n'
    elif new not in body:
        raise ValueError('unexpected release distribution notes')
    with tempfile.TemporaryDirectory() as temporary:
        path = Path(temporary) / 'notes.md'
        path.write_text(body)
        resume.run('gh', 'api', 'repos/' + REPO + '/releases/' + str(record['id']), '--method', 'PATCH', '-F', 'body=@' + str(path))
    (assets / 'sdk-acceptance.json').write_text(json.dumps(results, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['prepare', 'consumer', 'complete', 'npm', 'pypi', 'go'])
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--directory', type=Path)
    parser.add_argument('--packages', type=Path)
    parser.add_argument('--sdist', action='store_true')
    args = parser.parse_args()
    if args.command in ('npm', 'pypi', 'go'):
        if resume.run('git', '-C', str(args.source), 'rev-parse', 'HEAD') != SHA or resume.tag_sha(REPO, TAG) != SHA:
            raise ValueError('publication source or tag differs')
        if args.command == 'npm':
            paths = list((args.source / 'sdk-packages').glob('*.tgz'))
            if len(paths) != 1:
                raise ValueError('expected one npm package')
            resume.npm(paths[0], VERSION)
        elif args.command == 'pypi':
            resume.pypi(args.source / 'sdk/python/dist', PYTHON_VERSION)
        else:
            resume.go(REPO, TAG, SHA)
        return
    if args.directory is None:
        parser.error('--directory is required for preparation and acceptance')
    if args.command == 'consumer':
        consumer(args.source.resolve(), args.directory.resolve(), args.packages.resolve() if args.packages else None, args.sdist)
    elif args.command == 'prepare':
        prepare(args.source.resolve(), args.directory.resolve())
    else:
        complete(args.source.resolve(), args.directory.resolve())


if __name__ == '__main__':
    main()
