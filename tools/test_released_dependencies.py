"""Network-free registry, provenance and ownership failure fixtures."""
# Copyright (c) 2026 Atrinik contributors. SPDX-License-Identifier: MIT
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import released_dependencies as updater

NAME = 'atrinik-protocol'
VERSION = '0.2.0'
DATA = b'reviewed crate archive'
SHA = hashlib.sha256(DATA).hexdigest()
DEP = {'name': NAME, 'repository': 'atrinik/protocol', 'repository_id': 123}


class RegistryTests(unittest.TestCase):
    def fixture(self, api=True, index=True, yanked=False, checksum=SHA, data=DATA):
        def get(url, absent=False):
            if url.endswith('/download'):
                return data
            if 'index.crates.io' in url:
                return json.dumps({'name': NAME, 'vers': VERSION, 'cksum': checksum, 'yanked': yanked}).encode() if index else None
            return json.dumps({'version': {'crate': NAME, 'num': VERSION, 'checksum': checksum, 'yanked': yanked}}).encode() if api else None
        return patch.object(updater, 'fetch', get)

    def test_available(self):
        with self.fixture():
            updater.registry(DEP, VERSION, SHA)

    def test_unavailable(self):
        with self.fixture(api=False, index=False), self.assertRaisesRegex(ValueError, 'missing registry package'):
            updater.registry(DEP, VERSION, SHA)

    def test_pending_index(self):
        with self.fixture(index=False), self.assertRaisesRegex(ValueError, 'missing registry package'):
            updater.registry(DEP, VERSION, SHA)

    def test_yanked(self):
        with self.fixture(yanked=True), self.assertRaisesRegex(ValueError, 'yanked'):
            updater.registry(DEP, VERSION, SHA)

    def test_checksum_mismatch(self):
        with self.fixture(checksum='a' * 64), self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            updater.registry(DEP, VERSION, SHA)

    def test_download_mismatch(self):
        with self.fixture(data=b'changed'), self.assertRaisesRegex(ValueError, 'download checksum mismatch'):
            updater.registry(DEP, VERSION, SHA)

    def test_legacy_exact_pin_uses_registry_proof(self):
        dep = dict(DEP, legacy={'version': VERSION, 'sha256': SHA})
        with self.fixture():
            self.assertEqual(updater.source(dep, VERSION)['sha256'], SHA)

    def test_source_release_unavailable(self):
        with patch.object(updater, 'document', return_value=[]), self.assertRaisesRegex(ValueError, 'no reviewed source release'):
            updater.source(DEP)


class OwnershipTests(unittest.TestCase):
    def pr(self, title='older', bot='atrinik-crate-dependency-update[bot]', branch='automation/released-crate-dependencies'):
        return {'title': title, 'user': {'login': bot}, 'head': {'ref': branch, 'sha': 'a' * 40,
                'repo': {'full_name': 'atrinik/client'}}, 'base': {'ref': 'main'}}

    def test_existing_human_same_version_untouched(self):
        pr = self.pr(title='new', bot='human', branch='feat/access-tokens')
        self.assertEqual(updater.choose_pr([pr], 'new', 'automation/released-crate-dependencies',
                                          'atrinik-crate-dependency-update[bot]', 'atrinik/client'), ('waiting', pr))

    def test_human_automation_branch_rejected(self):
        with self.assertRaisesRegex(ValueError, 'another contributor'):
            updater.choose_pr([self.pr(bot='human')], 'new', 'automation/released-crate-dependencies',
                              'atrinik-crate-dependency-update[bot]', 'atrinik/client')

    def test_owned_bot_branch_identified(self):
        pr = self.pr()
        self.assertEqual(updater.choose_pr([pr], 'new', 'automation/released-crate-dependencies',
                                          'atrinik-crate-dependency-update[bot]', 'atrinik/client'), ('owned', pr))

    def test_concurrent_main_drift_stops_before_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'candidate.json').write_text(json.dumps({'repository': 'atrinik/client', 'base': 'a' * 40}))
            responses = [{'total_count': 1, 'repositories': [{'full_name': 'atrinik/client', 'id': 1}]},
                         {'object': {'sha': 'b' * 40}}]
            with patch.dict(updater.os.environ, {'CRATE_DEPENDENCY_UPDATE_APP_SLUG': 'atrinik-crate-dependency-update'}), patch.object(updater, 'gh_api', side_effect=responses), self.assertRaisesRegex(ValueError, 'main advanced'):
                updater.publish(root, root, {'repository': 'atrinik/client', 'repository_id': 1})
            self.assertFalse((root / 'Cargo.toml').exists())

    def test_inventory_truncation_fails_closed(self):
        with self.assertRaisesRegex(ValueError, 'truncated'):
            updater.choose_pr([self.pr()] * 100, 'new', 'automation/released-crate-dependencies',
                              'atrinik-crate-dependency-update[bot]', 'atrinik/client')


class ManifestTests(unittest.TestCase):
    def test_git_override_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'Cargo.toml').write_text('[workspace.dependencies]\natrinik-protocol = {git="https://example.com/repo",version="=0.2.0"}\n')
            with self.assertRaisesRegex(ValueError, 'override prohibited'):
                updater.exact_dependencies(root, {'dependencies': [DEP]})

    def test_registry_lock_checksum_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'Cargo.lock').write_text('[[package]]\nname="atrinik-protocol"\nversion="0.2.0"\nsource="git+https://example.com/repo"\n')
            with self.assertRaisesRegex(ValueError, 'lockfile registry identity mismatch'):
                updater.lock_check(root, {NAME: {'version': VERSION, 'sha256': SHA}})


if __name__ == '__main__':
    unittest.main()

class SourceManifestTests(unittest.TestCase):
    def fixture(self, immutable=True, revision='a' * 40, provenance_revision='a' * 40, checksum=SHA):
        tag = 'v3.0.0'
        manifest = {'schema_version': 1, 'repository': DEP['repository'], 'repository_id': str(DEP['repository_id']),
                    'source_tag': tag, 'revision': 'a' * 40, 'crates': [{'name': NAME, 'version': VERSION,
                    'asset': f'{NAME}-{VERSION}.crate', 'sha256': checksum}]}
        files = {'crate-release.json': json.dumps(manifest).encode(),
                 'provenance.json': json.dumps({'revision': provenance_revision, 'version': '3.0.0'}).encode(),
                 'SHA256SUMS': (checksum + f'  {NAME}-{VERSION}.crate\n').encode(),
                 f'{NAME}-{VERSION}.crate': DATA}
        release = {'tag_name': tag, 'draft': False, 'prerelease': False, 'immutable': immutable,
                   'published_at': '2026-10-11', 'assets': []}
        for name, data in files.items():
            release['assets'].append({'name': name, 'state': 'uploaded', 'size': len(data),
                'browser_download_url': f'https://github.com/{DEP["repository"]}/releases/download/{tag}/{name}',
                'digest': 'sha256:' + hashlib.sha256(data).hexdigest()})
        def get(url):
            return [release] if '/releases?' in url else {'object': {'type': 'commit', 'sha': revision}}
        return patch.object(updater, 'document', get), patch.object(updater, 'fetch', lambda url, absent=False: files[url.rsplit('/', 1)[1]])

    def test_immutable_source_manifest(self):
        document, fetch = self.fixture()
        with document, fetch, patch.object(updater, 'registry') as registry:
            record = updater.source(DEP)
            self.assertEqual(record['version'], VERSION)
            registry.assert_called_once_with(DEP, VERSION, SHA)

    def test_mutable_release_rejected(self):
        document, fetch = self.fixture(immutable=False)
        with document, fetch, self.assertRaisesRegex(ValueError, 'mutable'):
            updater.source(DEP)

    def test_tag_revision_mismatch(self):
        document, fetch = self.fixture(revision='b' * 40)
        with document, fetch, self.assertRaisesRegex(ValueError, 'tag drift'):
            updater.source(DEP)

    def test_provenance_revision_mismatch(self):
        document, fetch = self.fixture(provenance_revision='b' * 40)
        with document, fetch, self.assertRaisesRegex(ValueError, 'provenance mismatch'):
            updater.source(DEP)

    def test_release_checksum_mismatch(self):
        document, fetch = self.fixture(checksum='b' * 64)
        with document, fetch, self.assertRaisesRegex(ValueError, 'crate checksum mismatch'):
            updater.source(DEP)

    def test_full_page_does_not_claim_latest(self):
        with patch.object(updater, 'document', return_value=[{}] * 30), self.assertRaisesRegex(ValueError, 'discovery bound'):
            updater.source(DEP)


class LegacyGitTests(unittest.TestCase):
    def test_only_recorded_git_revision_allowed(self):
        legacy = {'git': 'https://github.com/atrinik/renderer', 'rev': 'a' * 40, 'version': '0.1.0'}
        policy = {'dependencies': [dict(DEP, legacy_git=legacy)]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            text = '[workspace.dependencies]\natrinik-protocol = {git="https://github.com/atrinik/renderer",rev="%s",version="0.1.0"}\n'
            (root / 'Cargo.toml').write_text(text % ('a' * 40))
            self.assertEqual(updater.exact_dependencies(root, policy), {NAME: '0.1.0'})
            (root / 'Cargo.toml').write_text(text % ('b' * 40))
            with self.assertRaisesRegex(ValueError, 'override prohibited'):
                updater.exact_dependencies(root, policy)

    def test_cargo_source_config_override_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / '.cargo').mkdir()
            (root / '.cargo/config.toml').write_text('[source.crates-io]\nreplace-with="custom"\n')
            with self.assertRaisesRegex(ValueError, 'configuration override'):
                updater.exact_dependencies(root, {'dependencies': []})

class LegacyLockTests(unittest.TestCase):
    def test_recorded_git_lock_revision_required(self):
        legacy = {'git': 'https://github.com/atrinik/renderer', 'rev': 'a' * 40, 'version': '0.1.0'}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix = '[[package]]\nname="atrinik-protocol"\nversion="0.1.0"\nsource="git+https://github.com/atrinik/renderer?rev=' + 'a' * 40 + '#'
            (root / 'Cargo.lock').write_text(prefix + 'a' * 40 + '"\n')
            updater.lock_check(root, {NAME: {'version': '0.1.0', 'legacy_git': legacy}})
            (root / 'Cargo.lock').write_text(prefix + 'b' * 40 + '"\n')
            with self.assertRaisesRegex(ValueError, 'legacy Git lock mismatch'):
                updater.lock_check(root, {NAME: {'version': '0.1.0', 'legacy_git': legacy}})

class CandidateBoundaryTests(unittest.TestCase):
    def fixture(self, root, output):
        manifest = '[workspace.package]\nversion="0.1.0"\n[workspace.dependencies]\natrinik-protocol = "=0.1.0"\n'
        (root / 'Cargo.toml').write_text(manifest)
        (output / 'Cargo.toml').write_text(manifest.replace('"=0.1.0"', '"=0.2.0"'))
        (output / 'Cargo.lock').write_text('[[package]]\nname="atrinik-protocol"\nversion="0.2.0"\nsource="' + updater.REGISTRY + '"\nchecksum="' + SHA + '"\n')
        record = {'name': NAME, 'version': VERSION, 'sha256': SHA}
        candidate = {'repository': 'atrinik/client', 'base': 'a' * 40, 'records': {NAME: record},
                     'title': 'chore(deps): update atrinik-protocol to 0.2.0',
                     'files': {'Cargo.toml': SHA, 'Cargo.lock': SHA}}
        return record, candidate, {'dependencies': [DEP]}

    def test_forged_manifest_exceeds_authenticated_pin_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            with (output / 'Cargo.toml').open('a') as file:
                file.write('malicious = "1"\n')
            with patch.object(updater, 'source', return_value=record), self.assertRaisesRegex(ValueError, 'exceed exact dependency pins'):
                updater.verify_candidate(root, output, policy, candidate)

    def test_forged_git_lock_source_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            path = output / 'Cargo.lock'
            path.write_text(path.read_text().replace(updater.REGISTRY, 'git+https://attacker.example/repo'))
            with patch.object(updater, 'source', return_value=record), self.assertRaisesRegex(ValueError, 'non-registry source'):
                updater.verify_candidate(root, output, policy, candidate)

    def test_extra_candidate_dependency_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            candidate['records']['malicious'] = record
            with self.assertRaisesRegex(ValueError, 'dependency scope changed'):
                updater.verify_candidate(root, output, policy, candidate)

    def test_valid_pin_and_registry_lock_reverified(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            with patch.object(updater, 'source', return_value=record), patch.object(updater, 'registry') as registry:
                updater.verify_candidate(root, output, policy, candidate)
                registry.assert_called_once_with({'name': NAME}, VERSION, SHA)
