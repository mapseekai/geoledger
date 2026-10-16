"""Fail-closed prerelease identity, evidence and immutable publication regressions."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile
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
recovery = load('recover-source-release')
SHA = 'a' * 40


class SourceReleaseTests(unittest.TestCase):
    def test_signature_identity_is_exact_for_normal_and_recovery_paths(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        for name in ('source-docker-release.yml', 'source-release-recovery.yml'):
            text = files[name]
            policy.check_signature_identity(text)
            for before, after in [('workflows/secure-images.yml@', 'workflows/source-docker-release.yml@'),
                                  ('refs/heads/main --certificate', 'refs/heads/other --certificate'),
                                  ('--certificate-identity ', '--certificate-identity-regexp '),
                                  ('https://token.actions.githubusercontent.com', 'https://other.example')]:
                with self.subTest(name=name, before=before), self.assertRaises(ValueError):
                    policy.check_signature_identity(text.replace(before, after))

    def test_recovery_cannot_skip_gates_or_rebuild(self):
        files = {p.name: p.read_text() for p in (ROOT / '.github/workflows').glob('*.yml')}
        policy.check_source_recovery(files)
        for before, after in [('needs: [validate, acceptance]', 'needs: validate'),
                              ('recover-source-release.py prepare', 'unrelated.py'),
                              ('recover-source-release.py verify', 'unrelated.py'),
                              ('runner: ubuntu-24.04-arm', 'runner: ubuntu-24.04'),
                              ('name: Recheck immutable registry tag', 'if: false\n        name: Recheck immutable registry tag'),
                              ('name: Recheck immutable registry tag', 'uses: docker/login-action@main'),
                              ('workflow_dispatch:', 'push:')]:
            changed = copy.deepcopy(files)
            changed['source-release-recovery.yml'] = changed['source-release-recovery.yml'].replace(before, after)
            with self.subTest(before=before), self.assertRaises(ValueError):
                policy.check_source_recovery(changed)

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

    def test_preflight_rejects_public_release_before_reserving_tag(self):
        resume = MagicMock(); resume.run.return_value = SHA
        resume.find_release.return_value = {'draft': False, 'prerelease': True}
        argv = ['source-release.py', 'preflight', '--reserve-tag', '--sha', SHA, '--tag', 'v1.0.0-alpha.1']
        with patch.object(sys, 'argv', argv), patch.dict(os.environ, GITHUB_REPOSITORY='mapseekai/geoledger', GITHUB_REF='refs/heads/main'), patch.object(release, 'load', return_value=resume), patch.object(release.subprocess, 'run'), patch.object(release, 'require_tag') as reserve:
            with self.assertRaisesRegex(ValueError, 'public release'):
                release.main()
            reserve.assert_not_called()

    def test_reserve_current_main_tag_before_builds(self):
        resume = MagicMock()
        resume.tag_sha.side_effect = [None, SHA]
        resume.gh.return_value = {'object': {'sha': SHA}}
        release.require_tag(resume, 'test/repo', 'v1-alpha.1', SHA, True)
        resume.run.assert_called_once_with('gh', 'api', 'repos/test/repo/git/refs', '-f',
                                          'ref=refs/tags/v1-alpha.1', '-f', 'sha=' + SHA)

    def test_reserved_tag_survives_main_advancing(self):
        for reserve in (False, True):
            resume = MagicMock(); resume.tag_sha.return_value = SHA
            release.require_tag(resume, 'test/repo', 'v1-alpha.1', SHA, reserve)
            resume.run.assert_not_called(); resume.gh.assert_not_called()

    def test_missing_historical_or_conflicting_tag_stops_before_publication(self):
        for actual, head, reserve in [(None, SHA, False), (None, 'b' * 40, True), ('b' * 40, SHA, True)]:
            resume = MagicMock(); resume.tag_sha.return_value = actual
            resume.gh.return_value = {'object': {'sha': head}}
            with self.subTest(actual=actual, reserve=reserve), self.assertRaises(ValueError):
                release.require_tag(resume, 'test/repo', 'v1-alpha.1', SHA, reserve)
            resume.run.assert_not_called()

    def test_tag_creation_denial_or_mismatch_stops_preflight(self):
        resume = MagicMock(); resume.tag_sha.side_effect = [None, 'b' * 40]
        resume.gh.return_value = {'object': {'sha': SHA}}
        with self.assertRaises(ValueError):
            release.require_tag(resume, 'test/repo', 'v1-alpha.1', SHA, True)
        resume.tag_sha.side_effect = [None]
        resume.run.side_effect = subprocess.CalledProcessError(1, 'gh')
        with self.assertRaises(subprocess.CalledProcessError):
            release.require_tag(resume, 'test/repo', 'v1-alpha.1', SHA, True)

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
        changes = [('preflight --reserve-tag', 'preflight'),
                   ('      contents: write', '      contents: read'),
                   ('needs: [validate, quality, performance]', 'needs: [validate, quality]'),
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
        for fail in (False, 'sdk', 'empty-member', 'missing-member'):
            with self.subTest(fail=fail), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                record = {'kind': 'combined', 'source_sha': SHA,
                          'image': 'docker.io/zwishing/geoledger@sha256:' + 'b' * 64,
                          'platforms': ['linux/amd64', 'linux/arm64']}
                (root / 'geoledger-images.json').write_text(json.dumps(record))
                output = root / 'acceptance-amd64.json'
                restarts = []
                def docker(*command):
                    if command[:2] == ('docker', 'restart'):
                        restarts.append(command)
                    if command[:3] == ('docker', 'image', 'inspect'):
                        return json.dumps([{'Architecture': 'amd64', 'Os': 'linux', 'Config': {'Labels': {'org.opencontainers.image.revision': SHA}}}])
                    if command[:3] == ('docker', 'run', '--rm'):
                        return 'geoledger-server 0.3.0-alpha.4'
                    if command[:2] == ('docker', 'port'):
                        return '127.0.0.1:' + ('18000' if restarts else '17000')
                    if command[:2] == ('docker', 'inspect'):
                        return json.dumps([{'State': {'Running': True, 'Health': {'Status': 'healthy'}}}])
                    return ''
                calls = []
                sdk_environments = []
                provisioned = []
                def provision(base, data_dir, key_file):
                    provisioned.append(base)
                    smoke.private_file(key_file, 'disposable-test-key\n')
                    if fail != 'missing-member':
                        smoke.private_file(Path(str(key_file) + '.member-id'),
                                           ' \n' if fail == 'empty-member' else ' disposable-member \n')
                def execute(command, **kwargs):
                    calls.append(command)
                    if 'sdk/python/tests/smoke.py' in command:
                        env = kwargs['env']
                        sdk_environments.append(env)
                        self.assertEqual(env['GL_TEST_MEMBER_ID'], 'disposable-member')
                        self.assertEqual(env['GL_ENDPOINT'], 'http://127.0.0.1:' + ('18000' if len(sdk_environments) == 2 else '17000'))
                        self.assertEqual(Path(env['GL_API_KEY_FILE']).read_text().strip(), 'disposable-test-key')
                        self.assertEqual({key for key in env if key.startswith('GL_')},
                                         {'GL_ENDPOINT', 'GL_API_KEY_FILE', 'GL_TEST_MEMBER_ID'})
                        if fail == 'sdk':
                            raise subprocess.CalledProcessError(1, command)
                response = MagicMock()
                response.__enter__.return_value.status = 200
                ready_urls = []
                def ready(url, **kwargs):
                    ready_urls.append(url)
                    return response
                argv = ['smoke-release-image.py', '--inventory', str(root), '--sha', SHA,
                        '--version', '0.3.0-alpha.4', '--arch', 'amd64', '--output', str(output)]
                with patch.object(smoke.sys, 'argv', argv), patch.object(smoke.platform, 'machine', return_value='x86_64'), \
                     patch.object(smoke, 'run', side_effect=docker), patch.object(smoke, 'provision', side_effect=provision), \
                     patch.dict(smoke.os.environ, {'GL_TEST_MEMBER_ID': 'foreign-member', 'GL_API_KEY': 'foreign-key'}), \
                     patch.object(smoke.urllib.request, 'urlopen', side_effect=ready), \
                     patch.object(smoke.subprocess, 'run', side_effect=execute):
                    if fail:
                        with self.assertRaises({'sdk': subprocess.CalledProcessError, 'empty-member': ValueError,
                                                'missing-member': FileNotFoundError}[fail]): smoke.main()
                        self.assertFalse(output.exists())
                    else:
                        smoke.main()
                        self.assertTrue(json.loads(output.read_text())['passed'])
                        self.assertEqual(sum('sdk/python/tests/smoke.py' in call for call in calls), 2)
                self.assertEqual(calls[-1][:3], ['docker', 'rm', '-fv'])
                self.assertEqual(len(provisioned), 1)
                self.assertEqual(len(sdk_environments), 2 if not fail else 1 if fail == 'sdk' else 0)
                if not fail:
                    self.assertEqual(len(restarts), 1)
                    self.assertEqual(ready_urls, ['http://127.0.0.1:17000/ready', 'http://127.0.0.1:18000/ready'])
                    for key in ('GL_TEST_MEMBER_ID', 'GL_API_KEY_FILE'):
                        self.assertEqual(sdk_environments[0][key], sdk_environments[1][key])


class RecoveryEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.run = {'id': recovery.RUN, 'head_sha': recovery.SHA,
                    'repository': {'full_name': recovery.REPO},
                    'path': '.github/workflows/source-docker-release.yml', 'event': 'workflow_dispatch',
                    'head_branch': 'main', 'status': 'completed', 'conclusion': 'failure', 'run_attempt': 1}
        jobs = [{'name': name, 'conclusion': 'success'} for name in sorted(recovery.SUCCEEDED)]
        jobs += [{'name': f'acceptance ({arch}, {runner})', 'conclusion': 'failure'}
                 for arch, runner in (('amd64', 'ubuntu-24.04'), ('arm64', 'ubuntu-24.04-arm'))]
        jobs += [{'name': 'complete', 'conclusion': 'skipped'}]
        for job in jobs:
            job.update(status='completed', run_id=recovery.RUN, head_sha=recovery.SHA)
        self.jobs = {'total_count': len(jobs), 'jobs': jobs}
        self.artifacts = {'total_count': 2, 'artifacts': [
            {'name': name, 'id': identity, 'digest': digest, 'expired': False,
             'workflow_run': {'id': recovery.RUN, 'head_sha': recovery.SHA}}
            for name, (identity, digest, _) in recovery.ARTIFACTS.items()]}

    def test_only_reviewed_run_and_all_successful_gates_are_reusable(self):
        recovery.validate_run(self.run, self.jobs)
        for field, value in [('head_sha', 'b' * 40), ('head_branch', 'other'), ('run_attempt', 2),
                             ('event', 'pull_request'), ('id', 123), ('path', '.github/workflows/other.yml')]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                recovery.validate_run(dict(self.run, **{field: value}), self.jobs)
        for conclusion in ('failure', 'skipped', None):
            for index in range(len(recovery.SUCCEEDED)):
                jobs = copy.deepcopy(self.jobs)
                jobs['jobs'][index]['conclusion'] = conclusion
                with self.subTest(index=index, conclusion=conclusion), self.assertRaises(ValueError):
                    recovery.validate_run(self.run, jobs)
        jobs = copy.deepcopy(self.jobs); jobs['jobs'].pop()
        with self.assertRaises(ValueError): recovery.validate_run(self.run, jobs)

    def test_artifact_identity_digest_expiry_and_origin_are_required(self):
        recovery.validate_artifacts(self.artifacts)
        for field, value in [('id', 123), ('digest', 'sha256:' + '0' * 64), ('expired', True),
                             ('workflow_run', {'id': 123, 'head_sha': recovery.SHA})]:
            artifacts = copy.deepcopy(self.artifacts)
            artifacts['artifacts'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                recovery.validate_artifacts(artifacts)

    def test_archive_checksum_and_exact_filenames_are_enforced(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for filename in ('geoledger-images.json', '../escape.json', 'unexpected.json'):
                output = io.BytesIO()
                with zipfile.ZipFile(output, 'w') as archive: archive.writestr(filename, '{}')
                data = output.getvalue(); digest = 'sha256:' + hashlib.sha256(data).hexdigest()
                if filename == 'geoledger-images.json':
                    recovery.unpack(data, digest, filename, root)
                    self.assertEqual((root / filename).read_text(), '{}')
                else:
                    with self.assertRaises(ValueError): recovery.unpack(data, digest, 'geoledger-images.json', root)
                with self.assertRaises(ValueError): recovery.unpack(data + b'tamper', digest, filename, root)

    def test_registry_tag_must_still_have_the_pinned_digest(self):
        raw = b'{"manifests":[]}'
        digest = 'sha256:' + hashlib.sha256(raw).hexdigest()
        for value in (digest, 'sha256:' + '0' * 64):
            with patch.object(recovery, 'DIGEST', value), patch.object(recovery.urllib.request, 'urlopen',
                    side_effect=[io.BytesIO(b'{"token":"anonymous-pull"}'), io.BytesIO(raw)]):
                if value == digest:
                    recovery.verify_tag()
                else:
                    with self.assertRaises(ValueError): recovery.verify_tag()


if __name__ == '__main__':
    unittest.main()
