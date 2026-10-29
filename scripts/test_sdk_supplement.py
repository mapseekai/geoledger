"""The one-time supplement cannot change source/image identity or accept failed consumers."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
import zipfile
import sys
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('supplement', ROOT / 'scripts/supplement-alpha7-sdks.py')
sdk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk)


class SdkSupplementTests(unittest.TestCase):
    def setUp(self):
        self.inventory = {'schema': 1, 'kind': 'combined', 'source_sha': sdk.SHA, 'image': sdk.IMAGE,
                          'platforms': ['linux/amd64', 'linux/arm64']}
        self.payloads = {'geoledger-images.json': json.dumps(self.inventory).encode()}
        for arch in ('amd64', 'arm64'):
            value = {'source_sha': sdk.SHA, 'image': sdk.IMAGE, 'platform': 'linux/' + arch,
                     'version': sdk.VERSION, 'passed': True}
            self.payloads['acceptance-' + arch + '.json'] = json.dumps(value).encode()
        self.record = {'id': 1, 'tag_name': sdk.TAG, 'draft': False, 'prerelease': True,
                       'assets': [{'name': name, 'browser_download_url': name,
                                   'digest': 'sha256:' + hashlib.sha256(data).hexdigest()}
                                  for name, data in self.payloads.items()]}

    def prepare(self, directory, record=None, tags=None, sha=None):
        gate = MagicMock()
        with patch.object(sdk.resume, 'run', return_value=sha or sdk.SHA), \
             patch.object(sdk.resume, 'tag_sha', side_effect=tags or [sdk.SHA, None]), \
             patch.object(sdk.resume, 'find_release', return_value=self.record if record is None else record), \
             patch.object(sdk.resume, 'fetch', side_effect=lambda name: self.payloads[name]), \
             patch.object(sdk.subprocess, 'run'), patch.object(sdk, 'load', return_value=gate), \
             patch.dict(os.environ, GITHUB_REPOSITORY=sdk.REPO, GITHUB_ACTIONS='true', GITHUB_REF='refs/heads/main'):
            result = sdk.prepare(ROOT, Path(directory))
        gate.require_immutable_tags.assert_called_once_with(['docker.io/zwishing/geoledger:' + sdk.VERSION], sdk.IMAGE.split('@')[1])
        return result

    def test_original_evidence_is_accepted_without_writes_to_remote(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(self.prepare(directory), self.record)
            self.assertEqual(json.loads((Path(directory) / 'geoledger-images.json').read_text()), self.inventory)

    def test_missing_public_release_assets_or_wrong_identity_stop_before_publication(self):
        cases = []
        for field, value in [('draft', True), ('prerelease', False), ('tag_name', 'v0.3.0-alpha.6'), ('assets', [])]:
            cases.append(dict(self.record, **{field: value}))
        bad = copy.deepcopy(self.record)
        bad['assets'][0]['digest'] = 'sha256:' + '0' * 64
        cases.append(bad)
        bad = copy.deepcopy(self.record)
        bad['assets'].append(bad['assets'][0])
        cases.append(bad)
        for record in cases:
            with self.subTest(record=record), tempfile.TemporaryDirectory() as directory, self.assertRaises(ValueError):
                self.prepare(directory, record=record)
        for tags, sha in [([None], sdk.SHA), (['b' * 40], sdk.SHA), ([sdk.SHA, 'b' * 40], sdk.SHA), ([sdk.SHA], 'b' * 40)]:
            with self.subTest(tags=tags, sha=sha), tempfile.TemporaryDirectory() as directory, self.assertRaises(ValueError):
                self.prepare(directory, tags=tags, sha=sha)

    def test_invalid_installed_receipt_never_updates_public_release(self):
        valid = {'source_sha': sdk.SHA, 'image': sdk.IMAGE, 'platform': 'linux/arm64', 'version': sdk.VERSION,
                 'passed': True, 'sdks': {'npm': sdk.VERSION, 'pypi': sdk.PYTHON_VERSION, 'go': sdk.TAG,
                                        'go_source_sha': sdk.SHA, 'from': 'public registries'}}
        cases = [dict(valid, passed=False), dict(valid, passed=1), dict(valid, image='other')]
        for field, value in [('go_source_sha', 'b' * 40), ('from', 'packages'), ('npm', 'old'), ('go', 'old'), ('pypi', 'old')]:
            cases.append(dict(valid, sdks=dict(valid['sdks'], **{field: value})))
        for bad in cases:
            with self.subTest(bad=bad), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / 'sdk-acceptance-amd64.json').write_text(json.dumps(dict(valid, platform='linux/amd64')))
                (root / 'sdk-acceptance-arm64.json').write_text(json.dumps(bad))
                with patch.object(sdk, 'prepare', return_value={}), patch.object(sdk.resume, 'run') as update:
                    with self.assertRaises(ValueError):
                        sdk.complete(ROOT, root)
                    update.assert_not_called()

    def test_manual_dispatch_cannot_reach_original_image_builds(self):
        policy = sdk.load('check-release-policy')
        text = (ROOT / '.github/workflows/release.yml').read_text()
        jobs = policy.jobs(text)
        self.assertIn("if: github.event_name == 'push'", jobs['tag-release-only'])
        for name in ('quality', 'credentials', 'verify'):
            self.assertEqual(policy.needs(jobs[name]), {'tag-release-only'})
        for name in ('binaries', 'images', 'github-release', 'sdks', 'complete'):
            pending = list(policy.needs(jobs[name]))
            seen = set()
            while pending:
                item = pending.pop()
                if item not in seen:
                    seen.add(item)
                    pending.extend(policy.needs(jobs[item]))
            self.assertIn('tag-release-only', seen)
        for name in ('sdk-supplement', 'sdk-acceptance', 'sdk-complete'):
            for forbidden in ('uses: ./.github/workflows/secure-images.yml', 'docker build', 'docker push', 'docker/login-action', 'DOCKERHUB_TOKEN'):
                self.assertNotIn(forbidden, jobs[name])
        self.assertIn("github.ref == 'refs/heads/main'", jobs['sdk-supplement'])
        self.assertEqual(policy.needs(jobs['sdk-complete']), {'sdk-supplement', 'sdk-acceptance'})

    def test_go_proxy_without_origin_must_match_every_original_module_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'sdk/go').mkdir(parents=True)
            (root / 'sdk/go/client.go').write_bytes(b'original SDK')
            (root / 'LICENSE').write_bytes(b'license')
            module = {'Version': sdk.TAG}
            metadata = {'Path': sdk.GO, 'Version': sdk.TAG, 'Zip': str(root / 'module.zip')}
            prefix = sdk.GO + '@' + sdk.TAG + '/'
            for changed in (None, 'changed', 'missing', 'extra', 'prefix', 'replace'):
                with zipfile.ZipFile(metadata['Zip'], 'w') as archive:
                    archive.writestr(prefix + 'client.go', b'changed' if changed == 'changed' else b'original SDK')
                    if changed != 'missing':
                        archive.writestr(('other/' if changed == 'prefix' else prefix) + 'LICENSE', b'license')
                    if changed == 'extra':
                        archive.writestr(prefix + 'extra.go', b'foreign file')
                selected = dict(module, Replace={'Path': '/local'}) if changed == 'replace' else module
                with self.subTest(changed=changed), patch.object(sdk.resume, 'run', return_value='sdk/go/client.go'):
                    if changed:
                        with self.assertRaises(ValueError): sdk.verify_go(root, metadata, selected)
                    else:
                        sdk.verify_go(root, metadata, selected)

    def test_publication_uses_fixed_versions_and_never_mutates_a_foreign_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'sdk-packages').mkdir()
            package = root / 'sdk-packages/client.tgz'
            package.write_bytes(b'local package')
            for kind in ('npm', 'pypi', 'go'):
                for sha, tag in [(sdk.SHA, sdk.SHA), ('b' * 40, sdk.SHA), (sdk.SHA, 'b' * 40)]:
                    with self.subTest(kind=kind, sha=sha, tag=tag), \
                         patch.object(sys, 'argv', ['supplement-alpha7-sdks.py', kind, '--source', str(root)]), \
                         patch.object(sdk.resume, 'run', return_value=sha), patch.object(sdk.resume, 'tag_sha', return_value=tag), \
                         patch.object(sdk.resume, 'npm') as npm, patch.object(sdk.resume, 'pypi') as pypi, \
                         patch.object(sdk.resume, 'go') as go:
                        calls = {'npm': npm, 'pypi': pypi, 'go': go}
                        if sha != sdk.SHA or tag != sdk.SHA:
                            with self.assertRaises(ValueError): sdk.main()
                            for call in calls.values(): call.assert_not_called()
                        else:
                            sdk.main()
                            if kind == 'npm': npm.assert_called_once_with(package, sdk.VERSION)
                            elif kind == 'pypi': pypi.assert_called_once_with(root / 'sdk/python/dist', sdk.PYTHON_VERSION)
                            else: go.assert_called_once_with(sdk.REPO, sdk.TAG, sdk.SHA)
                            for name, call in calls.items():
                                if name != kind: call.assert_not_called()


if __name__ == '__main__':
    unittest.main()
