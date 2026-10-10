#!/usr/bin/env python3
"""Scan immutable OCI images, then publish the same checked blobs without rebuilding.

A failing/missing scanner, incomplete architecture coverage, modified blob or stale
receipt fails closed. The gate covers HIGH/CRITICAL OS and language advisories,
including unpatched advisories; no implicit .trivyignore is accepted.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

DIGEST = re.compile(r'sha256:[0-9a-f]{64}\Z')
DEFAULT_PLATFORMS = {'linux/amd64', 'linux/arm64'}


def digest_file(path: Path) -> str:
    sha = hashlib.sha256()
    with path.open('rb') as source:
        while data := source.read(1024 * 1024):
            sha.update(data)
    return 'sha256:' + sha.hexdigest()


def inventory(layout: Path) -> tuple[str, dict[str, str]]:
    checked: dict[str, Path] = {}
    images: dict[str, str] = {}
    def blob(descriptor: dict) -> Path:
        digest = descriptor['digest']
        if not DIGEST.fullmatch(digest):
            raise ValueError('unsupported or malformed blob digest')
        path = layout / 'blobs/sha256' / digest[7:]
        if digest not in checked:
            if path.is_symlink() or not path.is_file() or path.stat().st_size != descriptor['size'] or digest_file(path) != digest:
                raise ValueError(f'OCI blob verification failed: {digest}')
            checked[digest] = path
        return path
    def walk(descriptor: dict) -> None:
        document = json.loads(blob(descriptor).read_text())
        if 'manifests' in document:
            for child in document['manifests']:
                walk(child)
            return
        config = json.loads(blob(document['config']).read_text())
        for layer in document['layers']:
            blob(layer)
        if descriptor.get('annotations', {}).get('vnd.docker.reference.type') == 'attestation-manifest':
            if (config.get('os'), config.get('architecture')) != ('unknown', 'unknown'):
                raise ValueError('runnable image cannot be excluded as an attestation')
            return
        declared = descriptor.get('platform', {})
        if declared and any(declared.get(k) != config.get(k) for k in ('os', 'architecture')):
            raise ValueError('manifest platform differs from image configuration')
        platform = config['os'] + '/' + config['architecture']
        if platform in images:
            raise ValueError(f'duplicate runnable platform: {platform}')
        images[platform] = descriptor['digest']
    index = json.loads((layout / 'index.json').read_text())
    roots = index['manifests']
    if len(roots) != 1:
        raise ValueError('OCI layout must contain exactly one image/index root')
    walk(roots[0])
    return roots[0]['digest'], images


def verify_image_identity(layout: Path, manifest_digest: str, platform: str, sha: str, report: dict | None = None) -> None:
    """Tie scan evidence to the image config and the actual source checked out by CI."""
    manifest = json.loads((layout / 'blobs/sha256' / manifest_digest[7:]).read_text())
    descriptor = manifest['config']
    config = json.loads((layout / 'blobs/sha256' / descriptor['digest'][7:]).read_text())
    if config.get('config', {}).get('Labels', {}).get('org.opencontainers.image.revision') != sha:
        raise ValueError('image revision label differs from checked source commit')
    if report is not None:
        metadata = report.get('Metadata', {})
        if metadata.get('ImageID') != descriptor['digest']:
            raise ValueError('scanner report belongs to a different image configuration')
        actual = metadata.get('ImageConfig', {})
        if actual.get('os', '') + '/' + actual.get('architecture', '') != platform:
            raise ValueError('scanner reported a different architecture from the manifest')


def validate_report(report: dict, kind: str) -> None:
    if kind not in ('server', 'console', 'server-build', 'console-build'):
        raise ValueError('unknown image policy')
    if report.get('SchemaVersion') != 2 or not isinstance(report.get('Results'), list):
        raise ValueError('invalid Trivy report schema')
    if report.get('Metadata', {}).get('OS', {}).get('EOSL'):
        raise ValueError('image operating system is end-of-life')
    if not report.get('Metadata', {}).get('OS', {}).get('Family'):
        raise ValueError('scanner did not identify image OS')
    os_results = [r for r in report['Results'] if r.get('Class') == 'os-pkgs']
    packages = {p['Name'] for r in os_results for p in (r.get('Packages') or [])}
    if not packages:
        raise ValueError('OS package inventory is empty')
    if kind == 'server':
        family = report['Metadata']['OS']['Family']
        if family == 'debian':
            native_present = 'libsqlite3-mod-spatialite' in packages and any(p.startswith('libssl3') for p in packages)
        elif family == 'alpine':
            native_present = {'libspatialite', 'libssl3', 'libcrypto3'} <= packages
        else:
            native_present = False
        if not native_present:
            raise ValueError('native SpatiaLite/OpenSSL packages absent from supported OS inventory')
        if not any(r.get('Class') == 'lang-pkgs' and r.get('Type') == 'cargo' and r.get('Packages') for r in report['Results']):
            raise ValueError('server Rust dependency inventory is empty')
    elif kind == 'server-build':
        if not any(r.get('Class') == 'lang-pkgs' and r.get('Type') == 'cargo' and r.get('Packages') for r in report['Results']):
            raise ValueError('build-stage Rust dependency inventory is empty')
    elif not any(r.get('Class') == 'lang-pkgs' and r.get('Packages') for r in report['Results']):
        raise ValueError('console language package inventory is empty')
    if kind == 'console-build':
        language = {p['Name'] for r in report['Results'] if r.get('Class') == 'lang-pkgs'
                    for p in (r.get('Packages') or [])}
        if not {'npm', 'typescript'} <= language:
            raise ValueError('build-stage npm/compiler inventory is incomplete')
    issues = [(v['VulnerabilityID'], v.get('PkgName'), v['Severity'])
              for r in report['Results'] for v in (r.get('Vulnerabilities') or [])
              if v.get('Severity') in ('HIGH', 'CRITICAL')]
    if issues:
        raise ValueError(f'{len(issues)} blocking HIGH/CRITICAL advisories: {issues[:12]}')


def scan(layout: Path, output: Path, kind: str, platforms: set[str], sha: str) -> dict:
    if not re.fullmatch(r'[0-9a-f]{40}', sha):
        raise ValueError('source must be a full commit SHA')
    output.mkdir(parents=True, exist_ok=True)
    receipt = output / 'gate.json'
    receipt.unlink(missing_ok=True)
    root, images = inventory(layout)
    if set(images) != platforms:
        raise ValueError(f'platform coverage mismatch: expected {sorted(platforms)}, got {sorted(images)}')
    for platform, digest in images.items():
        verify_image_identity(layout, digest, platform, sha)
    lock = json.loads((Path(__file__).resolve().parents[1] / '.security/trivy.lock.json').read_text())
    # Do not inherit environment flags capable of skipping packages/advisories.
    environment = {k: v for k, v in os.environ.items() if not k.startswith('TRIVY_')}
    if cache := os.environ.get('TRIVY_CACHE_DIR'):
        environment['TRIVY_CACHE_DIR'] = cache
    version = subprocess.check_output(['trivy', '--version'], env=environment, text=True, timeout=15)
    match = re.search(r'^Version: ([^\s]+)\s*$', version, re.M)
    if match is None or match[1] != lock['version']:
        raise ValueError('scanner version differs from pinned policy')
    reports = {}
    failures = []
    for platform, digest in sorted(images.items()):
        filename = platform.replace('/', '-') + '.json'
        path = output / filename
        # Trivy's OCI loader resolves only top-level references and descends to
        # the first child of an index. A per-platform view prevents accidentally
        # scanning amd64 twice; verified original blobs are shared, not rewritten.
        with tempfile.TemporaryDirectory(prefix='geoledger-oci-scan-') as temporary:
            view = Path(temporary) / 'layout'
            view.mkdir()
            (view / 'blobs').symlink_to(layout.resolve() / 'blobs', target_is_directory=True)
            blob = layout / 'blobs/sha256' / digest[7:]
            document = json.loads(blob.read_text())
            descriptor = {'digest': digest, 'size': blob.stat().st_size,
                          'mediaType': document.get('mediaType', 'application/vnd.oci.image.manifest.v1+json')}
            (view / 'index.json').write_text(json.dumps({'schemaVersion': 2, 'manifests': [descriptor]}))
            (view / 'oci-layout').write_text('{"imageLayoutVersion":"1.0.0"}')
            config = Path(temporary) / 'trivy.yaml'
            config.write_text('{}\n')
            command = ['trivy', 'image', '--input', str(view), '--platform', platform,
                       '--config', str(config), '--scanners', 'vuln', '--pkg-types', 'os,library', '--list-all-pkgs',
                       '--db-repository', 'ghcr.io/aquasecurity/trivy-db:2',
                       '--ignorefile', os.devnull, '--exit-code', '0', '--timeout', '15m',
                       '--format', 'json', '--output', str(path.resolve())]
            subprocess.run(command, env=environment, check=True, timeout=960)
        report = json.loads(path.read_text())
        verify_image_identity(layout, digest, platform, sha, report)
        try:
            validate_report(report, kind)
        except ValueError as error:
            failures.append(f'{platform}: {error}')
        reports[platform] = {'manifest': digest, 'report': filename, 'sha256': digest_file(path)}
    if failures:
        raise ValueError('; '.join(failures))
    gate = {'schema': 1, 'source_sha': sha, 'root': root, 'kind': kind, 'platforms': reports,
            'scanner': lock['version'], 'created_at': datetime.now(timezone.utc).isoformat()}
    receipt.write_text(json.dumps(gate, indent=2) + '\n')
    print(f'PASS {kind} {root}: all {len(images)} architectures scanned')
    return gate


def verify(layout: Path, output: Path, sha: str, kind: str | None = None) -> dict:
    gate = json.loads((output / 'gate.json').read_text())
    if kind is not None and gate['kind'] != kind:
        raise ValueError('scan receipt belongs to a different image policy')
    if gate['schema'] != 1 or gate['source_sha'] != sha:
        raise ValueError('gate belongs to a different source commit')
    age = (datetime.now(timezone.utc) - datetime.fromisoformat(gate['created_at'])).total_seconds()
    if not 0 <= age <= 24 * 60 * 60:
        raise ValueError('scan receipt is stale; rescan before publishing')
    lock = json.loads((Path(__file__).resolve().parents[1] / '.security/trivy.lock.json').read_text())
    if gate['scanner'] != lock['version']:
        raise ValueError('receipt uses a different scanner from current policy')
    root, images = inventory(layout)
    if root != gate['root'] or images != {p: r['manifest'] for p, r in gate['platforms'].items()}:
        raise ValueError('scanned OCI image has changed')
    if set(images) != DEFAULT_PLATFORMS:
        raise ValueError('release requires both linux/amd64 and linux/arm64')
    for platform, record in gate['platforms'].items():
        name = record['report']
        if Path(name).name != name:
            raise ValueError('invalid report filename')
        report = output / name
        if digest_file(report) != record['sha256']:
            raise ValueError('scan report has changed')
        value = json.loads(report.read_text())
        verify_image_identity(layout, record['manifest'], platform, sha, value)
        validate_report(value, gate['kind'])
    return gate


def publish(layout: Path, output: Path, sha: str, tags: list[str], destination: Path, kind: str | None = None) -> None:
    gate = verify(layout, output, sha, kind)
    if gate['kind'] not in ('server', 'console'):
        raise ValueError('build environments must never be published as application images')
    if not tags or any(not re.fullmatch(r'(ghcr\.io|docker\.io)/[a-z0-9_.\-/]+:[A-Za-z0-9_.-]+', tag) for tag in tags):
        raise ValueError('explicit ghcr.io/docker.io destination tags required')
    repos = {tag.rsplit(':', 1)[0] for tag in tags}
    if len(repos) != 1:
        raise ValueError('all tags must name the same image repository')
    digestfile = output / 'published-digest.txt'
    for tag in tags:
        subprocess.run(['skopeo', 'copy', '--all', '--preserve-digests', '--digestfile', str(digestfile),
                        f'oci:{layout.resolve()}', f'docker://{tag}'], check=True, timeout=1800)
        if digestfile.read_text().strip() != gate['root']:
            raise ValueError('registry digest differs from scanned image')
    destination.write_text(json.dumps({'schema': 1, 'source_sha': sha, 'kind': gate['kind'],
                                      'image': next(iter(repos)) + '@' + gate['root'],
                                      'platforms': sorted(gate['platforms'])}, indent=2) + '\n')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['scan', 'verify', 'publish'])
    parser.add_argument('--layout', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source-sha', required=True)
    parser.add_argument('--kind', choices=['server', 'console', 'server-build', 'console-build'], default='server')
    parser.add_argument('--platform', action='append')
    parser.add_argument('--tag', action='append', default=[])
    parser.add_argument('--inventory', type=Path)
    args = parser.parse_args()
    if args.command == 'scan':
        scan(args.layout, args.output, args.kind, set(args.platform or DEFAULT_PLATFORMS), args.source_sha)
    elif args.command == 'verify':
        print(json.dumps(verify(args.layout, args.output, args.source_sha, args.kind)))
    else:
        if not args.inventory:
            parser.error('--inventory is required for publish')
        publish(args.layout, args.output, args.source_sha, args.tag, args.inventory, args.kind)


if __name__ == '__main__':
    main()
