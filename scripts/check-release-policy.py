#!/usr/bin/env python3
"""Regression checks for this repository's release graph; actionlint validates YAML separately."""
from __future__ import annotations
from pathlib import Path
import re


def jobs(text: str) -> dict[str, str]:
    body = text.split('\njobs:\n', 1)[1]
    matches = list(re.finditer(r'^  ([a-zA-Z0-9_-]+):\n', body, re.M))
    return {m[1]: body[m.end():matches[i + 1].start() if i + 1 < len(matches) else len(body)] for i, m in enumerate(matches)}


def needs(job: str) -> set[str]:
    match = re.search(r'^    needs: ([^\n]+)', job, re.M)
    if not match:
        return set()
    return {v.strip() for v in match[1].strip('[]').split(',')}


def check(files: dict[str, str]) -> None:
    check_source_release(files)
    check_source_recovery(files)
    release, hub, images, ci = (jobs(files[n]) for n in ('release.yml', 'dockerhub.yml', 'secure-images.yml', 'ci.yml'))
    # A declared dependency is not a gate when the dependent job overrides
    # failure propagation, or a required quality job is allowed to be skipped.
    for graph, names in ((release, ('quality', 'credentials', 'binaries', 'images', 'github-release', 'sdks', 'complete')),
                         (hub, ('quality', 'images')),
                         (images, ('build-scan', 'merge-scan', 'publish')),
                         (ci, ('source', 'linux', 'web', 'msrv', 'windows', 'supply-chain', 'postgis-release'))):
        for name in names:
            if re.search(r'^    if:', graph[name], re.M):
                raise ValueError(f'{name} must not override required-job execution conditions')
            if re.search(r'^\s+continue-on-error:\s*(?!false(?:\s|$))\S+', graph[name], re.M):
                raise ValueError(f'{name} must propagate failures')
    for graph, expected in ((release, '${{ github.sha }}'), (hub, '${{ needs.validate.outputs.sha }}')):
        for name in ('quality', 'images'):
            if 'source_sha: ' + expected not in graph[name]:
                raise ValueError(f'{name} must use the exact selected source SHA')
    for graph, prerequisite in ((release, 'verify'), (hub, 'validate')):
        if './.github/workflows/ci.yml' not in graph['quality'] or 'source_sha:' not in graph['quality']:
            raise ValueError('publication must run same-SHA CI')
        if not {prerequisite, 'quality'} <= needs(graph['images']):
            raise ValueError('image publication bypasses quality/identity gate')
        if './.github/workflows/secure-images.yml' not in graph['images']:
            raise ValueError('image publication bypasses artifact scan')
    if not {'quality', 'verify'} <= needs(release['binaries']):
        raise ValueError('binary publication bypasses quality gate')
    if not {'verify', 'binaries', 'images'} <= needs(release['github-release']):
        raise ValueError('Release bypasses built and scanned artifacts')
    if 'github-release' not in needs(release['sdks']):
        raise ValueError('SDK publication bypasses release gate')
    if not {'github-release', 'sdks', 'verify'} <= needs(release['complete']):
        raise ValueError('public release requires completed SDK publication')
    if 'release-resume.py release' not in release['github-release'] or '--draft=false' not in release['complete']:
        raise ValueError('release must stay draft until all deliverables complete')
    if 'validate_inventories' not in release['github-release']:
        raise ValueError('release requires validated image inventories')
    if any('credentials' not in needs(release[name]) for name in ('binaries', 'images')):
        raise ValueError('publishing requires SDK credential preflight')
    sdk_steps = release['sdks'].replace("      - name: Python client (PyPI trusted publishing)\n        if: steps.python-packages.outputs.pending == 'true'\n", "      - name: Python client (PyPI trusted publishing)\n")
    for required in ('release-resume.py pypi', 'release-resume.py npm', 'release-resume.py go', '--sdk-consumer'):
        if required not in release['sdks']:
            raise ValueError('SDK retry verification and installed-package acceptance are required')
    if re.search(r'^        if:', sdk_steps, re.M):
        raise ValueError('SDK publication cannot silently skip a deliverable')
    if needs(images['merge-scan']) != {'build-scan'} or needs(images['publish']) != {'merge-scan'}:
        raise ValueError('registry writes must wait for all build/scan matrix entries')
    if re.search(r'push:\s*true', files['secure-images.yml']):
        raise ValueError('build must not push before the scan')
    for required in ('image-gate.py scan', 'platforms: linux/${{ matrix.arch }}', 'push: false', 'org.opencontainers.image.revision=${{ inputs.source_sha }}'):
        if required not in images['build-scan']:
            raise ValueError(f'missing image gate requirement: {required}')
    build_job = images['build-scan']
    for required in ('runs-on: ${{ matrix.runner }}', 'arch: amd64\n            runner: ubuntu-24.04',
                     'arch: arm64\n            runner: ubuntu-24.04-arm', 'name: Require native architecture',
                     '--platform "$PLATFORM"', 'name: checked-oci-${{ matrix.name }}-${{ matrix.arch }}'):
        if required not in build_job:
            raise ValueError('missing native build requirement: ' + required)
    if 'setup-qemu' in build_job:
        raise ValueError('release compilation must run on native architectures')
    merged = images['merge-scan']
    for required in ('ref: ${{ inputs.source_sha }}', 'scripts/merge-oci.py',
                     'scripts/image-gate.py scan', 'scripts/image-gate.py verify',
                     'checked-oci-geoledger-amd64/layout', 'checked-oci-geoledger-arm64/layout'):
        if required not in merged:
            raise ValueError('missing merged-image gate: ' + required)
    if '--platform' in merged:
        raise ValueError('merged-image gate must require both architectures')
    for step in re.split(r'^      - ', merged, flags=re.M):
        if 'scripts/merge-oci.py' in step and re.search(r'^(?:        )?if:', step, re.M):
            raise ValueError('merged-image gate must not be skipped')
    if 'kind: combined' not in build_job or 'name: [geoledger]' not in images['publish']:
        raise ValueError('release must publish the unified geoledger image')
    if 'driver: docker-container' not in build_job or not re.search(r'driver-opts: image=moby/buildkit@sha256:[0-9a-f]{64}', build_job):
        raise ValueError('immutable OCI reuse requires the tested, pinned container BuildKit driver')
    for required in ('target: build', '--kind "$IMAGE_KIND-build"',
                     'build-contexts: ${{ steps.build-gate.outputs.context }}',
                     'oci-layout://{root}/build-layout@{gate["root"]}'):
        if required not in build_job:
            raise ValueError(f'missing immutable build-stage gate: {required}')
    if not (build_job.index('target: build') < build_job.index('--kind "$IMAGE_KIND-build"')
            < build_job.index('build-contexts:')):
        raise ValueError('the build environment must be scanned before runtime assembly')
    steps = re.split(r'^      - ', build_job, flags=re.M)
    for step in steps:
        if any(marker in step for marker in ('target: build', 'id: build-gate', 'build-contexts:')):
            if re.search(r'^        if:', step, re.M):
                raise ValueError('build-stage gates and immutable assembly must not be skipped')
    for required in ('image-gate.py verify', 'image-gate.py publish'):
        if required not in images['publish']:
            raise ValueError(f'missing publishing requirement: {required}')
    for name in ('linux', 'web', 'msrv', 'windows', 'supply-chain', 'postgis-release'):
        if 'source' not in needs(ci[name]) or 'ref: ${{ inputs.source_sha || github.sha }}' not in ci[name]:
            raise ValueError(f'{name} does not check the immutable source')
    if ('python3 scripts/test-postgis-release.py' not in ci['postgis-release']
            or re.search(r'^\s+(?:- )?(if|continue-on-error):', ci['postgis-release'], re.M)):
        raise ValueError('database TLS and native restore acceptance must not be skipped')
    if 'install-spatialite.py' not in ci['windows'] or 'smoke-server.py' not in ci['windows']:
        raise ValueError('Windows must install and exercise native runtime')
    if 'smoke-server.py' not in release['binaries']:
        raise ValueError('packaged binaries must exercise storage, not only --version')



def check_bases(text: str) -> None:
    """Internal named stages are allowed; all externally resolved bases need a digest."""
    import shlex
    stages: set[str] = set()
    for line in text.splitlines():
        if not re.match(r'^\s*FROM\s', line, re.I):
            continue
        words = shlex.split(line, comments=True)
        if not words or words[0].upper() != 'FROM':
            continue
        words = [word for word in words[1:] if not word.startswith('--platform=')]
        if not words:
            raise ValueError('FROM must identify a base')
        image = words[0]
        if image not in stages and image != 'scratch' and not re.search(r'@sha256:[0-9a-f]{64}$', image):
            raise ValueError(f'unpinned external image: {image}')
        if len(words) == 3 and words[1].upper() == 'AS':
            stages.add(words[2])
        elif len(words) != 1:
            raise ValueError('unrecognized FROM form')



def check_compiler_lock(lock: dict, modules: dict[str, bytes], versions: list[str], recipe: str) -> None:
    import hashlib
    if lock.get('schema') != 1 or any(version != lock['version'] for version in versions):
        raise ValueError('TypeScript source release must match both package manifests')
    source = lock['source']
    if not re.fullmatch(r'[0-9a-f]{40}', source['commit']) or source['url'] != 'https://codeload.github.com/microsoft/typescript-go/tar.gz/' + source['commit']:
        raise ValueError('compiler source must identify an immutable upstream commit')
    if not re.fullmatch(r'[0-9a-f]{64}', source['sha256']):
        raise ValueError('compiler source checksum is required')
    if set(lock['go_archives']) != {'amd64', 'arm64'}:
        raise ValueError('patched compiler SDK must cover both image architectures')
    for architecture, archive in lock['go_archives'].items():
        expected = f"https://go.dev/dl/go{lock['go_version']}.linux-{architecture}.tar.gz"
        if archive['url'] != expected or not re.fullmatch(r'[0-9a-f]{64}', archive['sha256']) or archive['bytes'] <= 0:
            raise ValueError('Go SDK must have a versioned URL and checksum')
    for name in ('go.mod', 'go.sum'):
        if hashlib.sha256(modules[name]).hexdigest() != lock['module_files'][name]:
            raise ValueError('compiler module lock was modified without updating the checksum')
    for package in ('sdk/ts', 'web'):
        install = f'node /opt/build-policy/rebuild-typescript.cjs install /app/{package}'
        if install not in recipe or recipe.index(install) > recipe.index(f'npm --prefix {package} run build'):
            raise ValueError('bundled compiler must be replaced before any compilation')
        if f'npm ci --ignore-scripts --prefix {package}' not in recipe:
            raise ValueError('dependency installation must not execute the unpatched bundled compiler')


def check_source_release(files: dict[str, str]) -> None:
    text = files['source-docker-release.yml']
    check_signature_identity(text)
    graph = jobs(text)
    barriers = {
        'quality': {'validate'}, 'performance': {'validate'},
        'images': {'validate', 'quality', 'performance'},
        'acceptance': {'validate', 'images'},
        'complete': {'validate', 'quality', 'performance', 'images', 'acceptance'},
    }
    for name, prerequisites in barriers.items():
        job = graph[name]
        if needs(job) != prerequisites or re.search(r'^    (if|continue-on-error):', job, re.M):
            raise ValueError('source release gate bypass: ' + name)
    if ('preflight --reserve-tag' not in graph['validate']
            or '      contents: write' not in graph['validate']):
        raise ValueError('source release must reserve its tag with write permission before builds')
    for name in ('quality', 'images'):
        if 'source_sha: ${{ needs.validate.outputs.sha }}' not in graph[name]:
            raise ValueError('source release must use the selected SHA')
    for name in ('performance', 'acceptance', 'complete'):
        if 'ref: ${{ needs.validate.outputs.sha }}' not in graph[name]:
            raise ValueError('source release checkout differs')
    for required in ('./.github/workflows/ci.yml', 'bash scripts/release-performance.sh',
                     './.github/workflows/secure-images.yml', 'namespace: docker.io/zwishing',
                     'immutable_tags: true', 'arch: amd64', 'arch: arm64',
                     'scripts/smoke-release-image.py', 'cosign verify --certificate-identity',
                     'scripts/source-release.py complete', 'GH_TOKEN: ${{ github.token }}'):
        if required not in text:
            raise ValueError('missing source release protection: ' + required)
    if 'workflow_dispatch:' not in text or re.search(r'^  (push|pull_request):', text, re.M):
        raise ValueError('source prerelease must be explicitly dispatched')
    if re.search(r'npm publish|pypi-publish|NPM_TOKEN|PYPI_TRUSTED_PUBLISHING', text):
        raise ValueError('source/Docker scope must not publish SDKs')
    for required in ('IMMUTABLE_TAGS: ${{ inputs.immutable_tags }}', 'args+=(--immutable-tags)'):
        if required not in files['secure-images.yml']:
            raise ValueError('immutable tag guard must reach the publisher')
    for name in ('validate', 'acceptance', 'complete'):
        if re.search(r'^        (if|continue-on-error):', graph[name], re.M):
            raise ValueError('source release required steps must not be skipped')
    for step in re.split(r'^      - ', graph['performance'], flags=re.M):
        if 'cargo +1.92.0 build' in step or 'release-performance.sh' in step:
            if re.search(r'^        (if|continue-on-error):', step, re.M):
                raise ValueError('performance acceptance must not be skipped')


def check_signature_identity(text: str) -> None:
    expected = ('cosign verify --certificate-identity '
                'https://github.com/mapseekai/geoledger/.github/workflows/secure-images.yml@refs/heads/main '
                '--certificate-oidc-issuer https://token.actions.githubusercontent.com "$image"')
    if expected not in text or '--certificate-identity-regexp' in text or '--insecure-ignore' in text:
        raise ValueError('signature must use the exact reusable signer identity and GitHub issuer')


def check_source_recovery(files: dict[str, str]) -> None:
    text = files['source-release-recovery.yml']
    check_signature_identity(text)
    graph = jobs(text)
    if set(graph) != {'validate', 'acceptance', 'complete'}:
        raise ValueError('unexpected recovery jobs')
    for name, prerequisites in (('validate', set()), ('acceptance', {'validate'}), ('complete', {'validate', 'acceptance'})):
        if needs(graph[name]) != prerequisites or re.search(r'^\s+(?:- )?(if|continue-on-error):', graph[name], re.M):
            raise ValueError('recovery gate must not be bypassed')
        if 'ref: ${{ github.sha }}' not in graph[name] or 'ref: 6dccccd5ebc655f520a08c374823cc1389651f81' not in graph[name]:
            raise ValueError('recovery must distinguish immutable tooling and original candidate')
    for required in ('workflow_dispatch:', 'group: release-refs/tags/v0.3.0-alpha.4',
                     'SOURCE_SHA: 6dccccd5ebc655f520a08c374823cc1389651f81', 'RELEASE_TAG: v0.3.0-alpha.4',
                     'recover-source-release.py prepare', 'recover-source-release.py verify',
                     'source-release.py preflight', 'source-release.py complete',
                     'arch: amd64', 'runner: ubuntu-24.04', 'arch: arm64', 'runner: ubuntu-24.04-arm',
                     'scripts/smoke-release-image.py', 'DOCKER_CONFIG=$RUNNER_TEMP/anonymous-docker',
                     'pattern: image-acceptance-*', 'name: recovered-source-evidence'):
        if required not in text:
            raise ValueError('missing recovery protection: ' + required)
    if re.search(r'build-push-action|docker/login-action|secure-images.yml\n|npm publish|pypi-publish|secrets\.|id-token: write', text):
        raise ValueError('recovery must not rebuild, sign, push or publish SDKs')
    if re.search(r'^  (push|pull_request):', text, re.M):
        raise ValueError('recovery must be dispatched explicitly')


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    workflow = root / '.github/workflows'
    check({p.name: p.read_text() for p in workflow.glob('*.yml')})
    for path in (root / 'Dockerfile',):
        check_bases(path.read_text())
    import json
    lock = json.loads((root / '.security/node-build.lock.json').read_text())
    recipe = (root / 'Dockerfile').read_text()
    for entry in [lock['npm'], *lock['bundled_overrides']]:
        url = f"https://registry.npmjs.org/{entry['name']}/-/{entry['name']}-{entry['version']}.tgz"
        if entry['url'] != url or not re.fullmatch(r'[0-9a-f]{64}', entry['sha256']):
            raise ValueError('invalid pinned build-tool archive')
        if f"ADD --checksum=sha256:{entry['sha256']} {url} " not in recipe:
            raise ValueError(f"build-tool checksum/version drift: {entry['name']}")
    if 'node /opt/build-policy/verify-node-build.cjs && npm --version' not in recipe:
        raise ValueError('npm must pass its bundled dependency contract before use')
    compiler_lock = json.loads((root / '.security/typescript-build.lock.json').read_text())
    check_compiler_lock(compiler_lock,
                        {name: (root / '.security' / ('typescript-' + name)).read_bytes() for name in ('go.mod', 'go.sum')},
                        [json.loads((root / package / 'package.json').read_text())['devDependencies']['typescript'] for package in ('sdk/ts', 'web')], recipe)
    print('Release graph, same-SHA checks, native smoke and immutable base-image policy passed.')


if __name__ == '__main__':
    main()
