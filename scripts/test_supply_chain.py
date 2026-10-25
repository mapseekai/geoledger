"""Offline failure injection for release gates. No registry writes or scanners are executed."""
import copy
from datetime import datetime, timedelta, timezone
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tomllib
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


gate = load('gate', 'image-gate.py')
merger = load('merger', 'merge-oci.py')
policy = load('policy', 'check-release-policy.py')
installer = load('installer', 'install-spatialite.py')
SHA = 'a' * 40


def fixture(directory, platforms=('amd64', 'arm64'), attestation=None):
    directory.mkdir()
    blobs = directory / 'blobs/sha256'
    blobs.mkdir(parents=True)
    def blob(value, media):
        data = json.dumps(value).encode()
        digest = hashlib.sha256(data).hexdigest()
        (blobs / digest).write_bytes(data)
        return {'digest': 'sha256:' + digest, 'size': len(data), 'mediaType': media}
    manifests = []
    for arch in platforms:
        config = blob({'os': 'linux', 'architecture': arch, 'config': {'Labels': {'org.opencontainers.image.revision': SHA}}}, 'application/vnd.oci.image.config.v1+json')
        manifests.append(blob({'schemaVersion': 2, 'config': config, 'layers': []}, 'application/vnd.oci.image.manifest.v1+json'))
    if attestation is not None:
        value = copy.deepcopy(attestation)
        value['config'] = blob(value['config'], value.pop('config_media'))
        value['layers'] = [blob({}, media) for media in value['layers']]
        platform = value.pop('platform')
        descriptor = blob(value, 'application/vnd.oci.image.manifest.v1+json')
        descriptor.update(platform=platform, annotations={'vnd.docker.reference.type': 'attestation-manifest'})
        manifests.append(descriptor)
    index = blob({'schemaVersion': 2, 'manifests': manifests}, 'application/vnd.oci.image.index.v1+json')
    (directory / 'index.json').write_text(json.dumps({'schemaVersion': 2, 'manifests': [index]}))
    (directory / 'oci-layout').write_text('{"imageLayoutVersion":"1.0.0"}')


def report():
    return {'SchemaVersion': 2, 'Metadata': {'OS': {'Family': 'debian'}},
            'Results': [{'Class': 'os-pkgs', 'Packages': [{'Name': 'libssl3'}, {'Name': 'libsqlite3-mod-spatialite'}], 'Vulnerabilities': []},
                        {'Class': 'lang-pkgs', 'Type': 'cargo', 'Packages': [{'Name': p['name'], 'Version': p['version']} for p in tomllib.loads((ROOT / 'Cargo.lock').read_text())['package']]}]}


class MergeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.inputs = [self.root / arch for arch in ('amd64', 'arm64')]
        for layout in self.inputs:
            fixture(layout, (layout.name,), attestation={
                'schemaVersion': 2, 'config': {'os': 'unknown', 'architecture': 'unknown'},
                'config_media': 'application/vnd.oci.image.config.v1+json',
                'platform': {'os': 'unknown', 'architecture': 'unknown'},
                'layers': ['application/vnd.in-toto+json']})
        self.output = self.root / 'merged'

    def test_preserves_manifests_attestations_and_every_blob(self):
        expected = {}
        for layout in self.inputs:
            expected.update(gate.inventory(layout)[1])
        merger.merge(self.inputs, self.output, SHA)
        root, actual = gate.inventory(self.output)
        self.assertEqual(actual, expected)
        document = json.loads((self.output / 'blobs/sha256' / root[7:]).read_text())
        self.assertEqual(len(document['manifests']), 4)
        for layout in self.inputs:
            for blob in (layout / 'blobs/sha256').iterdir():
                self.assertEqual(blob.read_bytes(), (self.output / 'blobs/sha256' / blob.name).read_bytes())
        self.assertFalse((self.output / 'gate.json').exists())

    def test_missing_or_duplicate_architecture_rejected(self):
        for inputs in (self.inputs[:1], [self.inputs[0], self.inputs[0]]):
            with self.subTest(inputs=inputs), self.assertRaises(ValueError):
                merger.merge(inputs, self.output, SHA)

    def test_wrong_source_rejected(self):
        with self.assertRaises(ValueError):
            merger.merge(self.inputs, self.output, 'b' * 40)

    def test_corrupt_blob_rejected(self):
        root, _ = gate.inventory(self.inputs[0])
        (self.inputs[0] / 'blobs/sha256' / root[7:]).write_text('{}')
        with self.assertRaises(ValueError):
            merger.merge(self.inputs, self.output, SHA)

    def test_existing_output_rejected(self):
        self.output.mkdir()
        with self.assertRaises(FileExistsError):
            merger.merge(self.inputs, self.output, SHA)


class GateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.layout = self.root / 'layout'
        self.output = self.root / 'reports'
        fixture(self.layout)
    def scan(self, response=None, cargo_lock=None):
        def scanner(command, **kwargs):
            self.assertIn('--ignorefile', command)
            self.assertIn('--list-all-pkgs', command)
            value = copy.deepcopy(response or report())
            os_name, arch = command[command.index('--platform') + 1].split('/')
            value.setdefault('Metadata', {})['ImageConfig'] = {'os': os_name, 'architecture': arch}
            layout = Path(command[command.index('--input') + 1])
            descriptor = json.loads((layout / 'index.json').read_text())['manifests'][0]
            manifest = json.loads((layout / 'blobs/sha256' / descriptor['digest'][7:]).read_text())
            value['Metadata']['ImageID'] = manifest['config']['digest']
            Path(command[command.index('--output') + 1]).write_text(json.dumps(value))
            return subprocess.CompletedProcess(command, 0)
        version = json.loads((ROOT / '.security/trivy.lock.json').read_text())['version']
        with patch.object(gate.subprocess, 'check_output', return_value=f'Version: {version}'), patch.object(gate.subprocess, 'run', side_effect=scanner), patch('builtins.print'):
            return gate.scan(self.layout, self.output, 'server', gate.DEFAULT_PLATFORMS, SHA, cargo_lock=cargo_lock)

    def test_release_baseline_is_used_for_both_scans_and_receipt_verification(self):
        baseline = self.root / 'release-Cargo.lock'
        baseline.write_text('[[package]]\nname = "historical-dependency"\nversion = "1.0.0"\n')
        value = report()
        value['Results'][1]['Packages'] = [{'Name': 'historical-dependency', 'Version': '1.0.0'}]
        receipt = self.scan(value, baseline)
        self.assertEqual(receipt['cargo_lock_sha256'], gate.digest_file(baseline))
        self.assertEqual(set(receipt['platforms']), gate.DEFAULT_PLATFORMS)
        gate.verify(self.layout, self.output, SHA, cargo_lock=baseline)
        with self.assertRaisesRegex(ValueError, 'baseline'):
            gate.verify(self.layout, self.output, SHA)
        baseline.write_text(baseline.read_text().replace('1.0.0', '1.0.1'))
        with self.assertRaisesRegex(ValueError, 'baseline'):
            gate.verify(self.layout, self.output, SHA, cargo_lock=baseline)

    def test_release_baseline_does_not_relax_missing_dependencies_or_vulnerabilities(self):
        baseline = self.root / 'release-Cargo.lock'
        baseline.write_text('[[package]]\nname = "historical-dependency"\nversion = "1.0.0"\n')
        with self.assertRaisesRegex(ValueError, 'historical-dependency@1.0.0'):
            self.scan(cargo_lock=baseline)
        self.assertFalse((self.output / 'gate.json').exists())
        value = report()
        value['Results'][1]['Packages'] = [{'Name': 'historical-dependency', 'Version': '1.0.0'}]
        value['Results'][1]['Vulnerabilities'] = [{'VulnerabilityID': 'TEST-RELEASE', 'PkgName': 'historical-dependency', 'Severity': 'CRITICAL'}]
        with self.assertRaisesRegex(ValueError, 'blocking HIGH/CRITICAL'):
            self.scan(value, baseline)
        self.assertFalse((self.output / 'gate.json').exists())
    def test_two_architectures_and_same_digest_can_pass(self):
        self.scan()
        result = gate.verify(self.layout, self.output, SHA)
        self.assertEqual(set(result['platforms']), gate.DEFAULT_PLATFORMS)
    def test_attestation_formats_and_masquerading_images(self):
        artifact = {'config': {}, 'config_media': 'application/vnd.oci.empty.v1+json',
                    'artifactType': 'application/vnd.docker.attestation.manifest.v1+json',
                    'platform': {'os': 'unknown', 'architecture': 'unknown'},
                    'layers': ['application/vnd.in-toto+json']}
        compatible = dict(artifact, config={'os': 'unknown', 'architecture': 'unknown'},
                          config_media='application/vnd.oci.image.config.v1+json')
        cases = [(artifact, True), (compatible, True),
                 (dict(artifact, config={'os': 'linux', 'architecture': 'amd64'}), False),
                 (dict(artifact, config_media='application/vnd.oci.image.config.v1+json'), False),
                 (dict(artifact, artifactType='unrelated'), False),
                 (dict(artifact, platform={'os': 'linux', 'architecture': 'amd64'}), False),
                 (dict(artifact, layers=['application/vnd.oci.image.layer.v1.tar+gzip']), False),
                 (dict(artifact, layers=[]), False)]
        for number, (value, valid) in enumerate(cases):
            with self.subTest(case=number):
                self.layout = self.root / f'attestation-{number}'
                fixture(self.layout, attestation=value)
                if valid:
                    self.scan()
                    self.assertEqual(set(gate.verify(self.layout, self.output, SHA)['platforms']), gate.DEFAULT_PLATFORMS)
                else:
                    with self.assertRaises(ValueError):
                        gate.inventory(self.layout)

    def test_high_unfixed_vulnerability_blocks_gate_and_publish(self):
        r = report()
        r['Results'][0]['Vulnerabilities'] = [{'VulnerabilityID': 'TEST-HIGH', 'PkgName': 'native', 'Severity': 'HIGH'}]
        with self.assertRaises(ValueError):
            self.scan(r)
        self.assertFalse((self.output / 'gate.json').exists())
        with patch.object(gate.subprocess, 'run') as registry:
            with self.assertRaises(FileNotFoundError):
                gate.publish(self.layout, self.output, SHA, ['ghcr.io/test/image:test'], self.root / 'out.json')
            registry.assert_not_called()
    def test_missing_scanner_never_leaves_a_pass_receipt(self):
        self.scan()
        with patch.object(gate.subprocess, 'check_output', side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError):
                gate.scan(self.layout, self.output, 'server', gate.DEFAULT_PLATFORMS, SHA)
        self.assertFalse((self.output / 'gate.json').exists())
    def test_missing_native_inventory_blocks(self):
        r = report(); r['Results'][0]['Packages'] = [{'Name': 'unrelated'}]
        with self.assertRaises(ValueError): self.scan(r)
    def test_missing_architecture_blocks_before_scanner(self):
        single = self.root / 'single'; fixture(single, ('arm64',))
        with patch.object(gate.subprocess, 'run') as scanner:
            with self.assertRaises(ValueError):
                gate.scan(single, self.output, 'server', gate.DEFAULT_PLATFORMS, SHA)
            scanner.assert_not_called()
    def test_blob_mutation_blocks_before_registry(self):
        self.scan()
        blob = next((self.layout / 'blobs/sha256').iterdir()); blob.write_bytes(b'changed')
        with patch.object(gate.subprocess, 'run') as registry:
            with self.assertRaises(ValueError):
                gate.publish(self.layout, self.output, SHA, ['ghcr.io/test/image:test'], self.root / 'out.json')
            registry.assert_not_called()
    def test_report_mutation_blocks(self):
        self.scan(); (self.output / 'linux-amd64.json').write_text('{}')
        with self.assertRaises(ValueError): gate.verify(self.layout, self.output, SHA)
    def test_wrong_commit_blocks(self):
        self.scan()
        with self.assertRaises(ValueError): gate.verify(self.layout, self.output, 'b' * 40)
    def test_stale_receipt_blocks(self):
        self.scan(); path = self.output / 'gate.json'; value = json.loads(path.read_text())
        value['created_at'] = (datetime.now(timezone.utc) - timedelta(days=2)).isoformat(); path.write_text(json.dumps(value))
        with self.assertRaises(ValueError): gate.verify(self.layout, self.output, SHA)
    def test_scanner_error_blocks(self):
        with patch.object(gate.subprocess, 'check_output', return_value='Version: 0.75.0'), patch.object(gate.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['trivy'])):
            with self.assertRaises(subprocess.CalledProcessError):
                gate.scan(self.layout, self.output, 'server', gate.DEFAULT_PLATFORMS, SHA)
        self.assertFalse((self.output / 'gate.json').exists())

    def test_publish_copies_only_scanned_blobs_with_preserved_digests(self):
        result = self.scan()
        commands = []
        def copy_image(command, **kwargs):
            commands.append(command)
            self.assertEqual(command[:4], ['skopeo', 'copy', '--all', '--preserve-digests'])
            Path(command[command.index('--digestfile') + 1]).write_text(result['root'])
            return subprocess.CompletedProcess(command, 0)
        with patch.object(gate.subprocess, 'run', side_effect=copy_image):
            gate.publish(self.layout, self.output, SHA, ['ghcr.io/test/image:1.0.0', 'ghcr.io/test/image:1.0'], self.root / 'inventory.json')
        self.assertEqual(len(commands), 2)
        self.assertEqual(json.loads((self.root / 'inventory.json').read_text())['image'], 'ghcr.io/test/image@' + result['root'])
    def test_changed_scanner_policy_blocks_receipt(self):
        self.scan(); path = self.output / 'gate.json'; value = json.loads(path.read_text()); value['scanner'] = '0.0.0'; path.write_text(json.dumps(value))
        with self.assertRaises(ValueError): gate.verify(self.layout, self.output, SHA)
    def test_no_os_detection_cannot_be_reported_as_clean(self):
        value = report(); value['Metadata'] = {}
        with self.assertRaises(ValueError): self.scan(value)

    def test_missing_rust_inventory_blocks(self):
        value = report(); value['Results'] = value['Results'][:1]
        with self.assertRaises(ValueError): self.scan(value)

    def test_same_architecture_wrong_image_report_blocks(self):
        self.scan()
        receipt = self.output / 'gate.json'; gate_record = json.loads(receipt.read_text())
        record = gate_record['platforms']['linux/amd64']
        path = self.output / record['report']; value = json.loads(path.read_text())
        value['Metadata']['ImageID'] = 'sha256:' + 'f' * 64
        path.write_text(json.dumps(value)); record['sha256'] = gate.digest_file(path)
        receipt.write_text(json.dumps(gate_record))
        with self.assertRaisesRegex(ValueError, 'different image configuration'):
            gate.verify(self.layout, self.output, SHA)

    def test_mismatched_image_kind_blocks(self):
        self.scan()
        with self.assertRaises(ValueError): gate.verify(self.layout, self.output, SHA, 'console')

    def test_wrong_source_image_blocks_even_with_requested_receipt_sha(self):
        self.scan()
        with patch.object(gate.subprocess, 'check_output') as scanner:
            with self.assertRaisesRegex(ValueError, 'revision label'):
                gate.scan(self.layout, self.output, 'server', gate.DEFAULT_PLATFORMS, 'b' * 40)
            scanner.assert_not_called()

    def test_end_of_life_image_blocks(self):
        value = report(); value['Metadata']['OS']['EOSL'] = True
        with self.assertRaises(ValueError): self.scan(value)

    def test_scanner_version_prefix_does_not_satisfy_pin(self):
        version = json.loads((ROOT / '.security/trivy.lock.json').read_text())['version']
        with patch.object(gate.subprocess, 'check_output', return_value=f'Version: {version}1'):
            with self.assertRaises(ValueError):
                gate.scan(self.layout, self.output, 'server', gate.DEFAULT_PLATFORMS, SHA)


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
    def test_current_graph(self): policy.check(self.files)
    def test_native_and_merge_gates_cannot_be_removed(self):
        changes = [('runner: ubuntu-24.04-arm', 'runner: ubuntu-24.04'),
                   ('needs: merge-scan', 'needs: build-scan'),
                   ('scripts/merge-oci.py', 'scripts/unrelated.py'),
                   ('      - name: Merge immutable manifests and scan both architectures',
                    '      - if: false\n        name: Merge immutable manifests and scan both architectures')]
        for before, after in changes:
            files = copy.deepcopy(self.files)
            files['secure-images.yml'] = files['secure-images.yml'].replace(before, after)
            with self.subTest(before=before), self.assertRaises(ValueError):
                policy.check(files)
    def test_removing_quality_blocks(self):
        for name in ('release.yml', 'dockerhub.yml'):
            files = copy.deepcopy(self.files)
            files[name] = files[name].replace('needs: [verify, quality, credentials]', 'needs: verify').replace('needs: [validate, quality]', 'needs: validate')
            with self.subTest(name=name), self.assertRaises(ValueError): policy.check(files)
    def test_removing_scan_barrier_blocks(self):
        self.files['secure-images.yml'] = self.files['secure-images.yml'].replace('needs: build-scan', 'needs: unrelated')
        with self.assertRaises(ValueError): policy.check(self.files)
    def test_early_push_blocks(self):
        self.files['secure-images.yml'] = self.files['secure-images.yml'].replace('push: false', 'push: true')
        with self.assertRaises(ValueError): policy.check(self.files)
    def test_mutable_checkout_blocks(self):
        self.files['ci.yml'] = self.files['ci.yml'].replace('ref: ${{ inputs.source_sha || github.sha }}', 'ref: main')
        with self.assertRaises(ValueError): policy.check(self.files)
    def test_native_install_missing_blocks(self):
        self.files['ci.yml'] = self.files['ci.yml'].replace('install-spatialite.py', 'unrelated.py')
        with self.assertRaises(ValueError): policy.check(self.files)

    def test_always_publication_cannot_bypass_failed_quality(self):
        self.files['release.yml'] = self.files['release.yml'].replace('  images:\n', '  images:\n    if: always()\n')
        with self.assertRaises(ValueError): policy.check(self.files)

    def test_ignored_scanner_failure_is_not_a_quality_gate(self):
        self.files['ci.yml'] = self.files['ci.yml'].replace('  supply-chain:\n', '  supply-chain:\n    continue-on-error: true\n')
        with self.assertRaises(ValueError): policy.check(self.files)

    def test_skipped_quality_job_is_not_a_gate(self):
        self.files['ci.yml'] = self.files['ci.yml'].replace('  windows:\n', '  windows:\n    if: false\n')
        with self.assertRaises(ValueError): policy.check(self.files)

    def test_different_quality_source_is_rejected(self):
        self.files['release.yml'] = self.files['release.yml'].replace('source_sha: ${{ github.sha }}', 'source_sha: ' + 'b' * 40)
        with self.assertRaises(ValueError): policy.check(self.files)

    def test_image_without_source_label_is_rejected(self):
        self.files['secure-images.yml'] = self.files['secure-images.yml'].replace('          labels: org.opencontainers.image.revision=${{ inputs.source_sha }}\n', '')
        with self.assertRaises(ValueError): policy.check(self.files)


class InstallerTests(unittest.TestCase):
    def test_archive_traversal_and_links_rejected(self):
        for name, link in (('../escape', False), ('C:/escape', False), ('bin/link', True)):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); archive = root / 'bad.tar.bz2'
                with tarfile.open(archive, 'w:bz2') as output:
                    member = tarfile.TarInfo(name)
                    if link: member.type = tarfile.SYMTYPE; member.linkname = '/outside'
                    else: member.size = 1
                    output.addfile(member, None if link else io.BytesIO(b'x'))
                with self.assertRaises(ValueError): installer.extract(archive, root / 'target')
    def test_runtime_lock_has_closed_dependencies_and_sha256(self):
        lock = json.loads((ROOT / '.security/windows-spatialite.lock.json').read_text())
        names = {p['name'] for p in lock['packages']}
        self.assertIn('libspatialite', names)
        for package in lock['packages']:
            self.assertLessEqual(set(package['requires']), names)
            self.assertRegex(package['sha256'], r'^[0-9a-f]{64}$')



class BuildEnvironmentTests(unittest.TestCase):
    def test_cargo_inventory_requires_every_locked_name_and_version(self):
        value = report()
        gate.validate_source_inventory(value, 'combined-build')
        value['Results'][1]['Packages'].pop()
        with self.assertRaisesRegex(ValueError, 'Cargo.lock'):
            gate.validate_source_inventory(value, 'combined-build')
        value = report()
        value['Results'][1]['Packages'][0]['Version'] = '0.0.0-mismatch'
        with self.assertRaisesRegex(ValueError, 'Cargo.lock'):
            gate.validate_source_inventory(value, 'combined')

    def report(self):
        value = report()
        value['Results'].append({'Class': 'lang-pkgs', 'Type': 'node-pkg',
                                 'Packages': [{'Name': 'npm'}, {'Name': 'typescript'}]})
        return value
    def test_unnamed_graph_root_is_not_a_dependency(self):
        value = self.report()
        value['Results'][1]['Packages'].append({'ID': 'root', 'Relationship': 'root'})
        value['Results'][-1]['Packages'].append({'Name': 'next'})
        gate.validate_report(value, 'combined-build')
        gate.validate_report(value, 'combined')
        for kind in ('server', 'server-build', 'combined', 'combined-build'):
            broken = copy.deepcopy(value)
            broken['Results'][1]['Packages'] = [{'ID': 'root', 'Relationship': 'root'}]
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, 'Rust dependency inventory'):
                gate.validate_report(broken, kind)
        value['Results'][1]['Vulnerabilities'] = [{'VulnerabilityID': 'TEST-ROOT', 'PkgName': 'native', 'Severity': 'HIGH'}]
        with self.assertRaisesRegex(ValueError, 'blocking HIGH/CRITICAL'):
            gate.validate_report(value, 'combined-build')
    def test_builder_requires_actual_npm_and_compiler_inventory(self):
        value = self.report()
        gate.validate_report(value, 'console-build')
        for package in ('npm', 'typescript'):
            damaged = copy.deepcopy(value)
            damaged['Results'][-1]['Packages'] = [p for p in damaged['Results'][-1]['Packages'] if p['Name'] != package]
            with self.subTest(package=package), self.assertRaises(ValueError):
                gate.validate_report(damaged, 'console-build')
    def test_high_vulnerability_in_builder_blocks_even_if_runtime_is_clean(self):
        value = self.report()
        value['Results'][-1]['Vulnerabilities'] = [{'VulnerabilityID': 'TEST-BUILDER', 'PkgName': 'npm', 'Severity': 'HIGH'}]
        with self.assertRaises(ValueError): gate.validate_report(value, 'console-build')
    def test_rust_builder_requires_rust_inventory(self):
        gate.validate_report(report(), 'server-build')
        value = report(); value['Results'] = value['Results'][:1]
        with self.assertRaises(ValueError): gate.validate_report(value, 'server-build')
    def test_builder_receipt_cannot_be_used_for_registry_publication(self):
        with patch.object(gate, 'verify', return_value={'kind': 'console-build'}), patch.object(gate.subprocess, 'run') as registry:
            with self.assertRaises(ValueError):
                gate.publish(Path('.'), Path('.'), SHA, ['ghcr.io/test/app:1.0.0'], Path('unused'))
            registry.assert_not_called()
    def test_internal_stage_does_not_need_external_digest(self):
        policy.check_bases('FROM example@sha256:' + 'a' * 64 + ' AS tools\nFROM tools AS build')
        with self.assertRaises(ValueError): policy.check_bases('FROM tools AS build')
    def test_mutable_external_base_is_rejected(self):
        with self.assertRaises(ValueError): policy.check_bases('FROM node:22 AS build')
    def test_removing_builder_scan_or_reuse_fails_policy(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        for marker in ('target: build', '--kind "$IMAGE_KIND-build"', 'build-contexts: ${{ steps.build-gate.outputs.context }}'):
            damaged = copy.deepcopy(files)
            damaged['secure-images.yml'] = damaged['secure-images.yml'].replace(marker, '')
            with self.subTest(marker=marker), self.assertRaises(ValueError): policy.check(damaged)
    def test_builder_gate_cannot_be_conditionally_skipped(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        files['secure-images.yml'] = files['secure-images.yml'].replace('        id: build-gate', '        if: false\n        id: build-gate')
        with self.assertRaises(ValueError): policy.check(files)


class CompilerBootstrapTests(unittest.TestCase):
    def setUp(self):
        self.lock = json.loads((ROOT / '.security/typescript-build.lock.json').read_text())
        self.modules = {name: (ROOT / '.security' / ('typescript-' + name)).read_bytes() for name in ('go.mod', 'go.sum')}
        self.recipe = (ROOT / 'Dockerfile').read_text()
    def check(self):
        policy.check_compiler_lock(self.lock, self.modules, [self.lock['version']] * 2, self.recipe)
    def test_pinned_bootstrap_contract(self): self.check()
    def test_modified_module_graph_blocks(self):
        self.modules['go.mod'] += b'\n'
        with self.assertRaises(ValueError): self.check()
    def test_floating_source_blocks(self):
        self.lock['source']['commit'] = 'main'
        with self.assertRaises(ValueError): self.check()
    def test_missing_architecture_blocks(self):
        del self.lock['go_archives']['amd64']
        with self.assertRaises(ValueError): self.check()
    def test_package_version_drift_blocks(self):
        with self.assertRaises(ValueError):
            policy.check_compiler_lock(self.lock, self.modules, ['99.0.0', self.lock['version']], self.recipe)
    def test_install_scripts_cannot_execute_unpatched_binary(self):
        self.recipe = self.recipe.replace('npm ci --ignore-scripts', 'npm ci')
        with self.assertRaises(ValueError): self.check()


class NativeImageInventoryTests(unittest.TestCase):
    def test_combined_image_requires_both_application_inventories(self):
        value = report()
        with self.assertRaisesRegex(ValueError, 'console dependency inventory'):
            gate.validate_report(value, 'combined')
        value['Results'].append({'Class': 'lang-pkgs', 'Type': 'node-pkg', 'Packages': [{'Name': 'next'}]})
        gate.validate_report(value, 'combined')
        broken = copy.deepcopy(value)
        broken['Results'] = [r for r in broken['Results'] if r.get('Type') != 'cargo']
        with self.assertRaisesRegex(ValueError, 'Rust dependency inventory'):
            gate.validate_report(broken, 'combined')
        with self.assertRaisesRegex(ValueError, 'npm/compiler inventory'):
            gate.validate_report(value, 'combined-build')
        value['Results'][-1]['Packages'] += [{'Name': 'npm'}, {'Name': 'typescript'}]
        gate.validate_report(value, 'combined-build')

    def test_alpine_native_components_are_required_not_just_any_os_packages(self):
        value = report(); value['Metadata']['OS']['Family'] = 'alpine'
        value['Results'][0]['Packages'] = [{'Name': name} for name in ('libspatialite', 'libssl3', 'libcrypto3')]
        gate.validate_report(value, 'server')
        for name in ('libspatialite', 'libssl3', 'libcrypto3'):
            broken = copy.deepcopy(value)
            broken['Results'][0]['Packages'] = [package for package in broken['Results'][0]['Packages'] if package['Name'] != name]
            with self.subTest(name=name), self.assertRaises(ValueError): gate.validate_report(broken, 'server')
    def test_unrecognized_os_does_not_bypass_native_inventory(self):
        value = report(); value['Metadata']['OS']['Family'] = 'unknown'
        with self.assertRaises(ValueError): gate.validate_report(value, 'server')


class OciBuilderDriverTests(unittest.TestCase):
    def test_split_images_cannot_replace_unified_release(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        files['secure-images.yml'] = files['secure-images.yml'].replace('name: [geoledger]', 'name: [geoledger, geoledger-console]')
        with self.assertRaisesRegex(ValueError, 'unified geoledger image'):
            policy.check(files)

    def test_legacy_embedded_driver_cannot_replace_verified_oci_builder(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        files['secure-images.yml'] = files['secure-images.yml'].replace('driver: docker-container', 'driver: docker')
        with self.assertRaises(ValueError): policy.check(files)


class ReleaseInventoryTests(unittest.TestCase):
    def test_requires_complete_matching_immutable_images(self):
        audit = load('published_audit', 'audit-published-images.py')
        records = [{'kind': kind, 'source_sha': SHA, 'image': 'ghcr.io/test/' + kind + '@sha256:' + 'b' * 64} for kind in ('combined',)]
        audit.validate_inventories(records, SHA)
        for broken in ([], records * 2, [dict(records[0], source_sha='c'*40)], [dict(records[0], image='ghcr.io/test/combined:latest')]):
            with self.assertRaises(ValueError): audit.validate_inventories(broken, SHA)

    def test_baseline_download_requires_sha_and_never_falls_back_to_checkout(self):
        audit = load('published_audit', 'audit-published-images.py')
        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / 'Cargo.lock'
            with patch.object(audit.subprocess, 'check_output') as read:
                with self.assertRaises(ValueError):
                    audit.download_cargo_lock('test/repo', 'main', destination)
                read.assert_not_called()
            for response in (b'not a lockfile', b'version = 4\n'):
                with patch.object(audit.subprocess, 'check_output', return_value=response):
                    with self.assertRaises(ValueError):
                        audit.download_cargo_lock('test/repo', SHA, destination)
            with patch.object(audit.subprocess, 'check_output', side_effect=subprocess.CalledProcessError(1, ['gh'])):
                with self.assertRaises(subprocess.CalledProcessError):
                    audit.download_cargo_lock('test/repo', SHA, destination)
            self.assertFalse(destination.exists())

    def test_published_audit_uses_release_data_and_current_policy_end_to_end(self):
        audit = load('published_audit', 'audit-published-images.py')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            layout = root / 'original'
            fixture(layout)
            digest, _ = gate.inventory(layout)
            records = [{'kind': 'combined', 'source_sha': SHA, 'image': 'docker.io/test/combined@' + digest}]
            output = root / 'reports'
            baseline = b'[[package]]\nname = "historical-dependency"\nversion = "1.0.0"\n'
            version = json.loads((ROOT / '.security/trivy.lock.json').read_text())['version']
            scanned = []
            def read(command, **kwargs):
                if command[:2] == ['gh', 'api']:
                    if '/commits/' in command[2]:
                        return SHA + '\n'
                    self.assertEqual(command[2], f'repos/test/repo/contents/Cargo.lock?ref={SHA}')
                    return baseline
                self.assertEqual(command, ['trivy', '--version'])
                return f'Version: {version}\n'
            def run(command, **kwargs):
                if command[:3] == ['gh', 'release', 'download']:
                    directory = Path(command[command.index('--dir') + 1])
                    (directory / 'geoledger-images.json').write_text(json.dumps(records[0]))
                elif command[:2] == ['skopeo', 'copy']:
                    self.assertIn('--all', command)
                    self.assertIn('--preserve-digests', command)
                    self.assertEqual(command[-2], 'docker://' + records[0]['image'])
                    shutil.copytree(layout, command[-1].removeprefix('oci:'))
                else:
                    self.assertEqual(command[:2], ['trivy', 'image'])
                    platform = command[command.index('--platform') + 1]
                    scanned.append(platform)
                    view = Path(command[command.index('--input') + 1])
                    descriptor = json.loads((view / 'index.json').read_text())['manifests'][0]
                    manifest = json.loads((view / 'blobs/sha256' / descriptor['digest'][7:]).read_text())
                    value = report()
                    value['Results'][1]['Packages'] = [{'Name': 'historical-dependency', 'Version': '1.0.0'}]
                    value['Results'].append({'Class': 'lang-pkgs', 'Type': 'node-pkg', 'Packages': [{'Name': 'next'}]})
                    os_name, arch = platform.split('/')
                    value['Metadata'].update(ImageID=manifest['config']['digest'], ImageConfig={'os': os_name, 'architecture': arch})
                    Path(command[command.index('--output') + 1]).write_text(json.dumps(value))
                return subprocess.CompletedProcess(command, 0)
            with patch.object(sys, 'argv', ['audit', '--repository', 'test/repo', '--tag', 'v1.0.0', '--output', str(output)]), patch.object(audit.subprocess, 'check_output', side_effect=read), patch.object(audit.subprocess, 'run', side_effect=run), patch('builtins.print'):
                audit.main()
            self.assertEqual(set(scanned), gate.DEFAULT_PLATFORMS)
            self.assertEqual(len(scanned), 2)
            self.assertEqual((output / 'source/Cargo.lock').read_bytes(), baseline)
            status = json.loads((output / 'audit-status.json').read_text())
            self.assertEqual(status['status'], 'passed')
            self.assertEqual(status['source_sha'], SHA)
            self.assertEqual(status['images'], records)
            self.assertEqual(json.loads((output / 'combined/gate.json').read_text())['scanner'], version)


class ReleaseCompletionTests(unittest.TestCase):
    def test_incomplete_deliverables_block_public_release(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        for old,new in [('needs: [verify, github-release, sdks]', 'needs: [verify, github-release]'), ('release-resume.py release', 'echo release'), ('needs: [verify, quality, credentials]', 'needs: [verify, quality]')]:
            broken = dict(files); broken['release.yml'] = files['release.yml'].replace(old,new)
            with self.assertRaises(ValueError): policy.check(broken)


class ProductionRoleTests(unittest.TestCase):
    def test_database_tls_and_restore_acceptance_cannot_be_skipped(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        for before, after in [('python3 scripts/test-postgis-release.py', 'echo skipped'),
                              ('      - name: Database TLS, restricted role, native restore and browser binding',
                               '      - if: false\n        name: Database TLS, restricted role, native restore and browser binding')]:
            broken = dict(files)
            broken['ci.yml'] = files['ci.yml'].replace(before, after)
            with self.subTest(before=before), self.assertRaises(ValueError):
                policy.check(broken)

    def test_runtime_role_is_separate_and_unprivileged(self):
        compose=(ROOT/'deploy/compose.production.yaml').read_text()
        initialize=(ROOT/'deploy/postgis/initialize.sh').read_text()
        self.assertIn('POSTGRES_USER: geoledger_admin',compose)
        self.assertIn('postgresql://geoledger_app:${GL_DB_APP_PASSWORD:',compose)
        self.assertIn('LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION',initialize)
        self.assertIn('REVOKE CREATE ON SCHEMA public FROM PUBLIC',initialize)


resume = load('resume', 'release-resume.py')

class ReleaseResumeTests(unittest.TestCase):
    def archive(self, value=b'client', mtime=1):
        import io, tarfile
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode='w:gz') as archive:
            entry = tarfile.TarInfo('package/index.js')
            entry.size = len(value); entry.mtime = mtime
            archive.addfile(entry, io.BytesIO(value))
        return output.getvalue()

    def test_rebuilt_archive_ignores_time_but_rejects_changed_code(self):
        self.assertEqual(resume.contents(self.archive()), resume.contents(self.archive(mtime=2)))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'client.tgz'; path.write_bytes(self.archive())
            with self.assertRaises(ValueError): resume.same_archive(path, self.archive(b'other'))

    def test_existing_npm_package_never_republished(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'client.tgz'; path.write_bytes(self.archive())
            with patch.object(resume, 'fetch', side_effect=[b'{"dist":{"tarball":"https://example.test/package"}}', self.archive(mtime=2)]), patch.object(resume.subprocess, 'run') as write:
                resume.npm(path, '0.3.0-alpha.2'); write.assert_not_called()

    def test_github_network_failure_is_not_absence(self):
        import subprocess
        with patch.object(resume.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'HTTP 503')):
            with self.assertRaises(RuntimeError): resume.gh('repos/test/ref')

    def test_go_tag_is_immutable(self):
        with patch.object(resume, 'tag_sha', return_value='a' * 40), patch.object(resume, 'run') as write:
            resume.go('test/repo', 'v1', 'a' * 40); write.assert_not_called()
            with self.assertRaises(ValueError): resume.go('test/repo', 'v1', 'b' * 40)

    def test_pypi_partial_upload_only_keeps_missing_files(self):
        import hashlib, os
        data = self.archive()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); dist = root / 'dist'; dist.mkdir()
            (dist / 'client.tar.gz').write_bytes(data)
            (dist / 'client.whl').write_bytes(b'pending wheel')
            metadata = json.dumps({'urls': [{'filename': 'client.tar.gz', 'url': 'https://example.test/file', 'digests': {'sha256': hashlib.sha256(data).hexdigest()}}]}).encode()
            with patch.object(resume, 'fetch', side_effect=[metadata, data]), patch.dict(os.environ, GITHUB_OUTPUT=str(root / 'output')):
                resume.pypi(dist, '0.3.0-alpha.2')
            self.assertEqual([p.name for p in dist.iterdir()], ['client.whl'])
            self.assertEqual((root / 'output').read_text(), 'pending=true\n')

    def test_partial_draft_upload_preserves_existing_assets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'one').write_bytes(b'one'); (root / 'two').write_bytes(b'two')
            with patch.object(resume, 'tag_sha', return_value='sha'), patch.object(resume, 'gh', side_effect=[[{'id': 1, 'tag_name': 'v1', 'draft': True}], [{'name': 'one', 'id': 2}]]), patch.object(resume.subprocess, 'check_output', return_value=b'one'), patch.object(resume, 'run') as write:
                resume.release('test/repo', 'v1', 'sha', root)
                write.assert_called_once_with('gh', 'release', 'upload', 'v1', str(root / 'two'), '--repo', 'test/repo')

    def test_different_release_asset_blocks_upload(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'one').write_bytes(b'one')
            with patch.object(resume, 'tag_sha', return_value='sha'), patch.object(resume, 'gh', side_effect=[[{'id': 1, 'tag_name': 'v1', 'draft': True}], [{'name': 'one', 'id': 2}]]), patch.object(resume.subprocess, 'check_output', return_value=b'wrong'), patch.object(resume, 'run') as write:
                with self.assertRaises(ValueError): resume.release('test/repo', 'v1', 'sha', root)
                write.assert_not_called()

    def test_existing_python_release_skips_only_after_verification(self):
        import hashlib, os
        data = self.archive()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); dist = root / 'dist'; dist.mkdir()
            (dist / 'client.tar.gz').write_bytes(data)
            metadata = json.dumps({'urls': [{'filename': 'client.tar.gz', 'url': 'https://example.test/file', 'digests': {'sha256': hashlib.sha256(data).hexdigest()}}]}).encode()
            with patch.object(resume, 'fetch', side_effect=[metadata, data]), patch.dict(os.environ, GITHUB_OUTPUT=str(root / 'output')):
                resume.pypi(dist, '0.3.0-alpha.2')
            self.assertEqual((root / 'output').read_text(), 'pending=false\n')

    def test_published_release_cannot_receive_missing_assets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'one').write_bytes(b'one')
            with patch.object(resume, 'tag_sha', return_value='sha'), patch.object(resume, 'gh', side_effect=[[{'id': 1, 'tag_name': 'v1', 'draft': False}], []]), patch.object(resume, 'run') as write:
                with self.assertRaises(ValueError): resume.release('test/repo', 'v1', 'sha', root)
                write.assert_not_called()

    def test_new_release_stays_draft_and_uploads_assets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'one').write_bytes(b'one')
            draft = {'id': 1, 'tag_name': 'v1-alpha.1', 'draft': True, 'prerelease': True}
            with patch.object(resume, 'tag_sha', return_value='sha') as tag, patch.object(resume, 'gh', side_effect=[[], []]), patch.object(resume, 'run', return_value=json.dumps(draft)) as write:
                resume.release('test/repo', 'v1-alpha.1', 'sha', root)
                tag.assert_called_once_with('test/repo', 'v1-alpha.1')
                self.assertEqual(write.call_args_list[0].args[:5], ('gh', 'api', 'repos/test/repo/releases', '--method', 'POST'))
                for field in ('draft=true', 'prerelease=true', 'body=@notes.md'):
                    self.assertIn(field, write.call_args_list[0].args)
                self.assertFalse(any(arg.startswith('target_commitish=') for arg in write.call_args_list[0].args))
                self.assertEqual(write.call_args_list[1].args[1:3], ('release', 'upload'))

    def test_created_release_identity_must_match_before_upload(self):
        draft = {'id': 1, 'tag_name': 'v1-alpha.1', 'draft': True, 'prerelease': True}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'one').write_bytes(b'one')
            for changed in ({'tag_name': 'v2'}, {'draft': False}, {'prerelease': False}):
                with self.subTest(changed=changed), patch.object(resume, 'tag_sha', return_value='sha'), patch.object(resume, 'gh', return_value=[]), patch.object(resume, 'run', return_value=json.dumps({**draft, **changed})) as write:
                    with self.assertRaisesRegex(ValueError, 'identity'):
                        resume.release('test/repo', 'v1-alpha.1', 'sha', root)
                    self.assertEqual(write.call_count, 1)

    def test_release_lookup_finds_drafts_on_later_pages(self):
        draft = {'id': 101, 'tag_name': 'v2', 'draft': True}
        with patch.object(resume, 'gh', side_effect=[[{'tag_name': 'v1'}] * 100, [draft]]) as api:
            self.assertEqual(resume.find_release('test/repo', 'v2'), draft)
            self.assertEqual(api.call_args_list[-1].args, ('repos/test/repo/releases?per_page=100&page=2',))

    def test_release_listing_errors_never_create_a_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'one').write_bytes(b'one')
            for error in (None, RuntimeError('HTTP 403')):
                with patch.object(resume, 'tag_sha', return_value='sha'), patch.object(resume, 'gh', side_effect=[error]), patch.object(resume, 'run') as write:
                    with self.assertRaises(RuntimeError):
                        resume.release('test/repo', 'v1', 'sha', root)
                    write.assert_not_called()

    def test_new_draft_uses_creation_response_when_listing_lags(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'one').write_bytes(b'one')
            draft = {'id': 42, 'tag_name': 'v1', 'draft': True, 'prerelease': False}
            def api(endpoint):
                if endpoint == 'repos/test/repo/releases?per_page=100&page=1': return []
                self.assertEqual(endpoint, 'repos/test/repo/releases/42/assets?per_page=100')
                return []
            with patch.object(resume, 'tag_sha', return_value='sha'), patch.object(resume, 'gh', side_effect=api) as reads, patch.object(resume, 'run', return_value=json.dumps(draft)) as write:
                resume.release('test/repo', 'v1', 'sha', root)
                self.assertEqual(reads.call_count, 2)
                self.assertEqual(write.call_count, 2)
                self.assertEqual(write.call_args_list[1].args[1:3], ('release', 'upload'))

    def test_unknown_npm_lookup_outcome_never_publishes(self):
        with patch.object(resume, 'fetch', side_effect=TimeoutError), patch.object(resume.subprocess, 'run') as write:
            with self.assertRaises(TimeoutError): resume.npm(Path('package.tgz'), '0.3.0-alpha.2')
            write.assert_not_called()


if __name__ == '__main__':
    unittest.main()
