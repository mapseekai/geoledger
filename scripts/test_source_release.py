"""Fail-closed prerelease identity, evidence and immutable publication regressions."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


release = load('source-release')
gate = load('image-gate')
policy = load('check-release-policy')
smoke = load('smoke-release-image')
SHA = 'a' * 40


class SourceReleaseTests(unittest.TestCase):
    def test_performance_fixture_and_aggregate_gate_execute(self):
        script = (ROOT / 'scripts/release-performance.sh').read_text()
        blocks = script.split("<<'PYTHON'\n")
        fixture = blocks[1].split('\nPYTHON', 1)[0]
        aggregate = blocks[2].split('\nPYTHON', 1)[0]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run([sys.executable, '-', str(root)], input=fixture, text=True, check=True)
            lines = (root / 'fixture.jsonl').read_text().splitlines()
            self.assertEqual(len(lines), 10000)
            self.assertEqual(json.loads(lines[-1])['id'], '000000010000')
            self.assertEqual(json.loads((root / 'fixture.meta.json').read_text())['crs'], 'EPSG:4326')
            self.assertEqual((root / 'fixture.jsonl').stat().st_mode & 0o777, 0o644)
            (root / 'load-postgis.exit').write_text('0\n')
            for backend in ('sqlite', 'postgis'):
                (root / f'incremental-{backend}').mkdir()
                (root / f'incremental-{backend}/result.json').write_text('{"success":true}')
                (root / f'incremental-{backend}.exit').write_text('0\n')
            self.assertEqual(subprocess.run([sys.executable, '-', str(root)], input=aggregate, text=True, capture_output=True).returncode, 0)
            for name in ('load-postgis.exit', 'incremental-sqlite.exit', 'incremental-postgis.exit'):
                (root / name).write_text('1\n')
                self.assertNotEqual(subprocess.run([sys.executable, '-', str(root)], input=aggregate, text=True, capture_output=True).returncode, 0)
                (root / name).write_text('0\n')

    def test_only_full_sha_and_prerelease_are_accepted(self):
        release.validate_identity(SHA, 'v0.3.0-alpha.4')
        for sha, tag in [(SHA, 'v0.3.0'), ('main', 'v0.3.0-alpha.4'), (SHA, 'v0.3.0-alpha.4;false')]:
            with self.subTest(sha=sha, tag=tag), self.assertRaises(ValueError):
                release.validate_identity(sha, tag)

    def test_evidence_requires_exact_source_destination_and_both_architectures(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            record = {'kind': 'combined', 'source_sha': SHA,
                      'image': 'docker.io/zwishing/geoledger@sha256:' + 'b' * 64,
                      'platforms': ['linux/amd64', 'linux/arm64']}
            file = root / 'geoledger-images.json'
            file.write_text(json.dumps(record))
            self.assertEqual(release.inventory(root, SHA), record)
            for field, value in [('source_sha', 'c' * 40), ('image', record['image'].replace('zwishing', 'other')),
                                 ('platforms', ['linux/amd64'])]:
                bad = dict(record, **{field: value}); file.write_text(json.dumps(bad))
                with self.subTest(field=field), self.assertRaises(ValueError):
                    release.inventory(root, SHA)

    def test_missing_failed_or_foreign_smoke_blocks_completion(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            record = {'source_sha': SHA, 'image': 'docker.io/zwishing/geoledger@sha256:' + 'b' * 64}
            for arch in ('amd64', 'arm64'):
                (root / f'acceptance-{arch}.json').write_text(json.dumps(dict(record, platform='linux/' + arch, version='0.3.0-alpha.4', passed=True)))
            release.validate_acceptance(root, record, '0.3.0-alpha.4')
            file = root / 'acceptance-arm64.json'; original = json.loads(file.read_text())
            for field, value in [('passed', False), ('source_sha', 'c' * 40), ('version', '0.3.0-alpha.3'), ('platform', 'linux/amd64')]:
                file.write_text(json.dumps(dict(original, **{field: value})))
                with self.subTest(field=field), self.assertRaises(ValueError):
                    release.validate_acceptance(root, record, '0.3.0-alpha.4')
            file.unlink()
            with self.assertRaises(FileNotFoundError):
                release.validate_acceptance(root, record, '0.3.0-alpha.4')

    def test_immutable_tag_accepts_only_absence_or_same_digest(self):
        raw = b'{"manifest":"approved"}'
        digest = 'sha256:' + hashlib.sha256(raw).hexdigest()
        for result in [subprocess.CompletedProcess([], 0, raw, b''),
                       subprocess.CompletedProcess([], 1, b'', b'manifest unknown: manifest unknown')]:
            with patch.object(gate.subprocess, 'run', return_value=result):
                gate.require_immutable_tags(['docker.io/zwishing/geoledger:0.3.0-alpha.4'], digest)
        for result in [subprocess.CompletedProcess([], 0, b'other manifest', b''),
                       subprocess.CompletedProcess([], 1, b'', b'unauthorized'),
                       subprocess.CompletedProcess([], 1, b'', b'connection timeout')]:
            with patch.object(gate.subprocess, 'run', return_value=result), self.assertRaises((ValueError, RuntimeError)):
                gate.require_immutable_tags(['docker.io/zwishing/geoledger:0.3.0-alpha.4'], digest)

    def test_every_release_barrier_and_scope_is_required(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        policy.check_source_release(files)
        changes = [('needs: [validate, quality, performance]', 'needs: [validate, quality]'),
                   ('needs: [validate, images]', 'needs: validate'),
                   ('needs: [validate, quality, performance, images, acceptance]', 'needs: [validate, images]'),
                   ('immutable_tags: true', 'immutable_tags: false'),
                   ('namespace: docker.io/zwishing', 'namespace: docker.io/other'),
                   ('source_sha: ${{ needs.validate.outputs.sha }}', 'source_sha: main'),
                   ('arch: arm64', 'arch: amd64'),
                   ('      - name: Build candidate and require original load thresholds', '      - name: Build candidate and require original load thresholds\n        if: false'),
                   ('cosign verify --certificate-identity', 'cosign version'),
                   ('workflow_dispatch:', 'push:')]
        for old, new in changes:
            changed = copy.deepcopy(files)
            changed['source-docker-release.yml'] = changed['source-docker-release.yml'].replace(old, new)
            with self.subTest(old=old), self.assertRaises(ValueError):
                policy.check_source_release(changed)

    def test_native_smoke_requires_both_sdk_runs_and_cleans_up_on_failure(self):
        for fail in (False, True):
            with self.subTest(fail=fail), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                record = {'kind': 'combined', 'source_sha': SHA,
                          'image': 'docker.io/zwishing/geoledger@sha256:' + 'b' * 64,
                          'platforms': ['linux/amd64', 'linux/arm64']}
                (root / 'geoledger-images.json').write_text(json.dumps(record))
                output = root / 'acceptance-amd64.json'
                def docker(*command):
                    if command[:3] == ('docker', 'image', 'inspect'):
                        return json.dumps([{'Architecture': 'amd64', 'Os': 'linux', 'Config': {'Labels': {'org.opencontainers.image.revision': SHA}}}])
                    if command[:3] == ('docker', 'run', '--rm'):
                        return 'geoledger-server 0.3.0-alpha.4'
                    if command[:2] == ('docker', 'port'):
                        return '127.0.0.1:17000'
                    if command[:2] == ('docker', 'inspect'):
                        return json.dumps([{'State': {'Running': True, 'Health': {'Status': 'healthy'}}}])
                    return ''
                calls = []
                def execute(command, **kwargs):
                    calls.append(command)
                    if fail and 'sdk/python/tests/smoke.py' in command:
                        raise subprocess.CalledProcessError(1, command)
                response = MagicMock()
                response.__enter__.return_value.status = 200
                argv = ['smoke-release-image.py', '--inventory', str(root), '--sha', SHA,
                        '--version', '0.3.0-alpha.4', '--arch', 'amd64', '--output', str(output)]
                with patch.object(smoke.sys, 'argv', argv), patch.object(smoke.platform, 'machine', return_value='x86_64'), \
                     patch.object(smoke, 'run', side_effect=docker), patch.object(smoke, 'provision'), \
                     patch.object(smoke.urllib.request, 'urlopen', return_value=response), \
                     patch.object(smoke.subprocess, 'run', side_effect=execute):
                    if fail:
                        with self.assertRaises(subprocess.CalledProcessError): smoke.main()
                        self.assertFalse(output.exists())
                    else:
                        smoke.main()
                        self.assertTrue(json.loads(output.read_text())['passed'])
                        self.assertEqual(sum('sdk/python/tests/smoke.py' in call for call in calls), 2)
                self.assertEqual(calls[-1][:3], ['docker', 'rm', '-fv'])


if __name__ == '__main__':
    unittest.main()
