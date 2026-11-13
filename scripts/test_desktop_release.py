"""Exercise release input round trips, integrity failures and command routing."""
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


artifact = load('desktop-artifact')
release = load('tauri-release')


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        self.dest = self.root / 'dest'
        self.dest.mkdir()
        for name, content in [(artifact.BINARY, b'executable'), (artifact.RESOURCES + 'frontend/index.html', b'frontend')]:
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        self.expected = {'schema': 1, 'revision': 'a' * 40, 'version': '1.2.9', 'platform': 'Linux-X64'}
        self.archive = self.root / 'inputs.tar.gz'
        artifact.create(self.source, self.archive, self.expected)

    def rewrite(self, transform):
        with tarfile.open(self.archive) as tar:
            entries = [(m.name, tar.extractfile(m).read()) for m in tar.getmembers()]
        with tarfile.open(self.archive, 'w:gz') as tar:
            for name, data in transform(entries):
                info = tarfile.TarInfo(name)
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))

    def test_round_trip_preserves_binary_resources_and_executable_permission(self):
        stale = self.dest / artifact.RESOURCES / 'old.js'
        stale.parent.mkdir(parents=True)
        stale.write_text('obsolete')
        artifact.restore(self.dest, self.archive, self.expected)
        self.assertEqual((self.dest / artifact.BINARY).read_bytes(), b'executable')
        self.assertTrue((self.dest / artifact.BINARY).stat().st_mode & 0o111)
        self.assertEqual((self.dest / artifact.RESOURCES / 'frontend/index.html').read_bytes(), b'frontend')
        self.assertFalse(stale.exists())

    def test_wrong_revision_version_and_architecture_are_rejected_before_writes(self):
        for key in ['revision', 'version', 'platform']:
            with self.subTest(key=key), self.assertRaises(ValueError):
                artifact.restore(self.dest, self.archive, {**self.expected, key: 'wrong'})
        self.assertEqual(list(self.dest.iterdir()), [])

    def test_modified_binary_is_rejected_before_writes(self):
        self.rewrite(lambda entries: [(n, b'tampered' if n == artifact.BINARY else d) for n, d in entries])
        with self.assertRaisesRegex(ValueError, 'checksum'):
            artifact.restore(self.dest, self.archive, self.expected)
        self.assertEqual(list(self.dest.iterdir()), [])

    def test_traversal_and_duplicate_members_are_rejected(self):
        original = self.archive.read_bytes()
        for name in ['../outside', artifact.RESOURCES + '../outside', artifact.BINARY]:
            self.archive.write_bytes(original)
            self.rewrite(lambda entries: entries + [(name, b'bad')])
            with self.subTest(name=name), self.assertRaises(ValueError):
                artifact.restore(self.dest, self.archive, self.expected)
        self.assertEqual(list(self.dest.iterdir()), [])

    def test_links_are_rejected(self):
        with tarfile.open(self.archive, 'w:gz') as tar:
            link = tarfile.TarInfo(artifact.BINARY)
            link.type = tarfile.SYMTYPE
            link.linkname = '/etc/passwd'
            tar.addfile(link)
        with self.assertRaises(ValueError):
            artifact.restore(self.dest, self.archive, self.expected)

    def test_missing_frontend_rejects_creation(self):
        (self.source / artifact.RESOURCES / 'frontend/index.html').unlink()
        with self.assertRaises(ValueError):
            artifact.create(self.source, self.archive, self.expected)

    def test_only_successful_matching_main_job_can_supply_artifacts(self):
        good = {'id': 1, 'head_sha': self.expected['revision'], 'head_branch': 'main', 'event': 'push', 'head_repository': {'full_name': 'owner/repo'}}
        name = f"desktop-inputs-{self.expected['revision']}-Linux-X64"
        for overrides, conclusion, expired, accepted in [({}, 'success', False, True), ({}, 'failure', False, False), ({}, 'success', True, False), ({'event': 'pull_request'}, 'success', False, False), ({'head_sha': 'b' * 40}, 'success', False, False), ({'head_branch': 'branch'}, 'success', False, False), ({'head_repository': {'full_name': 'other/repo'}}, 'success', False, False)]:
            def api(endpoint):
                if '/workflows/' in endpoint:
                    return {'workflow_runs': [{**good, **overrides}]}
                if '/jobs?' in endpoint:
                    return {'jobs': [{'name': 'build (ubuntu-22.04)', 'conclusion': conclusion}]}
                return {'artifacts': [{'name': name, 'expired': expired}]}
            with patch.object(artifact, 'api', api):
                result = artifact.find_artifact('owner/repo', self.expected, 'build (ubuntu-22.04)')
            self.assertEqual(result is not None, accepted)


class RoutingTests(unittest.TestCase):
    def test_verified_inputs_bundle_without_compiling_and_keep_package_formats(self):
        self.assertEqual(release.command(['build', '--bundles', 'deb,rpm,appimage', '--', '--locked'], True), ['pnpm', 'tauri', 'bundle', '--bundles', 'deb,rpm,appimage'])

    def test_missing_inputs_use_normal_locked_build(self):
        args = ['build', '--bundles', 'dmg,app', '--', '--locked']
        self.assertEqual(release.command(args, False), ['pnpm', 'tauri', *args])

    def test_unexpected_cargo_options_are_not_silently_dropped(self):
        with self.assertRaises(ValueError):
            release.command(['build', '--', '--features', 'different'], True)


if __name__ == '__main__':
    unittest.main()
