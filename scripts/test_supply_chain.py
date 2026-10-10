"""Offline failure injection for release gates. No registry writes or scanners are executed."""
import copy
from datetime import datetime, timedelta, timezone
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile
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
policy = load('policy', 'check-release-policy.py')
installer = load('installer', 'install-spatialite.py')
SHA = 'a' * 40


def fixture(directory, platforms=('amd64', 'arm64')):
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
    index = blob({'schemaVersion': 2, 'manifests': manifests}, 'application/vnd.oci.image.index.v1+json')
    (directory / 'index.json').write_text(json.dumps({'schemaVersion': 2, 'manifests': [index]}))
    (directory / 'oci-layout').write_text('{"imageLayoutVersion":"1.0.0"}')


def report():
    return {'SchemaVersion': 2, 'Metadata': {'OS': {'Family': 'debian'}},
            'Results': [{'Class': 'os-pkgs', 'Packages': [{'Name': 'libssl3'}, {'Name': 'libsqlite3-mod-spatialite'}], 'Vulnerabilities': []}, {'Class': 'lang-pkgs', 'Type': 'cargo', 'Packages': [{'Name': 'rusqlite'}]}]}


class GateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.layout = self.root / 'layout'
        self.output = self.root / 'reports'
        fixture(self.layout)
    def scan(self, response=None):
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
            return gate.scan(self.layout, self.output, 'server', gate.DEFAULT_PLATFORMS, SHA)
    def test_two_architectures_and_same_digest_can_pass(self):
        self.scan()
        result = gate.verify(self.layout, self.output, SHA)
        self.assertEqual(set(result['platforms']), gate.DEFAULT_PLATFORMS)
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
    def test_removing_quality_blocks(self):
        for name in ('release.yml', 'dockerhub.yml'):
            files = copy.deepcopy(self.files)
            files[name] = files[name].replace('needs: [verify, quality]', 'needs: verify').replace('needs: [validate, quality]', 'needs: validate')
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
    def report(self):
        value = report()
        value['Results'].append({'Class': 'lang-pkgs', 'Type': 'node-pkg',
                                 'Packages': [{'Name': 'npm'}, {'Name': 'typescript'}]})
        return value
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
        self.recipe = (ROOT / 'web/Dockerfile').read_text()
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
    def test_legacy_embedded_driver_cannot_replace_verified_oci_builder(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        files['secure-images.yml'] = files['secure-images.yml'].replace('driver: docker-container', 'driver: docker')
        with self.assertRaises(ValueError): policy.check(files)

if __name__ == '__main__': unittest.main()
