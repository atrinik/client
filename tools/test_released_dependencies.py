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
        with patch.object(updater, 'document', side_effect=[{'id': DEP['repository_id'], 'full_name': DEP['repository']}, []]), self.assertRaisesRegex(ValueError, 'no reviewed source release'):
            updater.source(DEP)


class OwnershipTests(unittest.TestCase):
    def pr(self, title='older', bot='atrinik-crate-dependency-update[bot]', branch='automation/released-crate-dependencies'):
        return {'title': title, 'user': {'login': bot}, 'head': {'ref': branch, 'sha': 'a' * 40,
                'repo': {'full_name': 'atrinik/client'}}, 'base': {'ref': 'main'}}

    def test_matching_title_does_not_suppress_update(self):
        pr = self.pr(title='new', bot='human', branch='feat/access-tokens')
        self.assertEqual(updater.choose_pr([pr], 'automation/released-crate-dependencies',
                                          'atrinik-crate-dependency-update[bot]', 'atrinik/client'), ('create', None))

    def test_human_automation_branch_rejected(self):
        with self.assertRaisesRegex(ValueError, 'another contributor'):
            updater.choose_pr([self.pr(bot='human')], 'automation/released-crate-dependencies',
                              'atrinik-crate-dependency-update[bot]', 'atrinik/client')

    def test_owned_bot_branch_identified(self):
        pr = self.pr()
        self.assertEqual(updater.choose_pr([pr], 'automation/released-crate-dependencies',
                                          'atrinik-crate-dependency-update[bot]', 'atrinik/client'), ('owned', pr))

    def test_concurrent_main_drift_stops_before_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'candidate.json').write_text(json.dumps({'repository': 'atrinik/client', 'base': 'a' * 40}))
            responses = [{'total_count': 1, 'repositories': [{'full_name': 'atrinik/client', 'id': 1, 'node_id': 'R_client_fixture'}]},
                         {'object': {'sha': 'b' * 40}}]
            with patch.dict(updater.os.environ, {'CRATE_DEPENDENCY_UPDATE_APP_SLUG': 'atrinik-crate-dependency-update'}), patch.object(updater, 'gh_api', side_effect=responses), self.assertRaisesRegex(ValueError, 'main advanced'):
                updater.publish(root, root, {'repository': 'atrinik/client', 'repository_id': 1})
            self.assertFalse((root / 'Cargo.toml').exists())

    def test_inventory_truncation_fails_closed(self):
        with self.assertRaisesRegex(ValueError, 'truncated'):
            updater.choose_pr([self.pr()] * 100, 'automation/released-crate-dependencies',
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
            if url == 'https://api.github.com/repos/' + DEP['repository']:
                return {'id': DEP['repository_id'], 'full_name': DEP['repository']}
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

    def test_release_discovery_reads_a_second_page(self):
        seen = []
        def get(url):
            seen.append(url)
            if url.endswith('/protocol'):
                return {'id': DEP['repository_id'], 'full_name': DEP['repository']}
            return [] if '&page=2' in url else [{'draft': True}] * 30
        with patch.object(updater, 'document', side_effect=get), self.assertRaisesRegex(ValueError, 'no reviewed source release'):
            updater.source(DEP)
        self.assertEqual(len(seen), 3)
        self.assertTrue(seen[-1].endswith('per_page=30&page=2'))

    def test_full_page_does_not_claim_latest(self):
        with patch.object(updater, 'document', side_effect=[{'id': DEP['repository_id'], 'full_name': DEP['repository']}] + [[{}] * 30] * 10), self.assertRaisesRegex(ValueError, 'discovery bound'):
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
        (output / 'Cargo.lock').write_text('version = 4\n[[package]]\nname="atrinik-protocol"\nversion="0.2.0"\nsource="' + updater.REGISTRY + '"\nchecksum="' + SHA + '"\n')
        record = {'name': NAME, 'version': VERSION, 'sha256': SHA}
        candidate = {'repository': 'atrinik/client', 'base': 'a' * 40, 'records': {NAME: record},
                     'title': 'chore(deps): update atrinik-protocol to 0.2.0',
                     'files': {name: hashlib.sha256((output / name).read_bytes()).hexdigest() for name in ('Cargo.toml', 'Cargo.lock')}}
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

class HardenedBoundaryTests(unittest.TestCase):
    fixture = CandidateBoundaryTests.fixture
    def test_small_crate_sparse_paths(self):
        for name, expected in [('x', '1/x'), ('cc', '2/cc'), ('log', '3/l/log'),
                               ('syn', '3/s/syn'), ('serde', 'se/rd/serde')]:
            self.assertEqual(updater.index_path(name), expected)
        for name in ['../evil', '', 'x/y', 'x' * 65]:
            with self.assertRaises(ValueError):
                updater.index_path(name)

    def test_live_repository_id_mismatch(self):
        with patch.object(updater, 'document', return_value={'id': 999, 'full_name': DEP['repository']}), self.assertRaisesRegex(ValueError, 'repository identity'):
            updater.source(DEP)

    def test_live_repository_rename_mismatch(self):
        with patch.object(updater, 'document', return_value={'id': DEP['repository_id'], 'full_name': 'attacker/protocol'}), self.assertRaisesRegex(ValueError, 'repository identity'):
            updater.source(DEP)

    def test_unexpected_candidate_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            candidate = {'repository': 'atrinik/client', 'base': 'a' * 40, 'records': {}, 'title': '', 'files': {}, 'validated': True}
            with self.assertRaisesRegex(ValueError, 'candidate schema'):
                updater.verify_candidate(root, root, {}, candidate)

    def test_forged_candidate_hashes_do_not_authorize_profile_edit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            path = output / 'Cargo.toml'
            path.write_text(path.read_text() + '\n[profile.release]\noverflow-checks = false\n')
            candidate['files']['Cargo.toml'] = hashlib.sha256(path.read_bytes()).hexdigest()
            with patch.object(updater, 'source', return_value=record), self.assertRaisesRegex(ValueError, 'exceed exact dependency pins'):
                updater.verify_candidate(root, output, policy, candidate)

    def test_unknown_lock_schema_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            path = output / 'Cargo.lock'
            path.write_text('evil = true\n' + path.read_text())
            with patch.object(updater, 'source', return_value=record), self.assertRaisesRegex(ValueError, 'lock schema'):
                updater.verify_candidate(root, output, policy, candidate)

    def test_third_party_checksum_revalidated_including_short_names(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            path = output / 'Cargo.lock'
            with path.open('a') as stream:
                stream.write('\n[[package]]\nname="cc"\nversion="1.0.0"\nsource="' + updater.REGISTRY + '"\nchecksum="' + 'a' * 64 + '"\n')
            with patch.object(updater, 'source', return_value=record), patch.object(updater, 'registry', side_effect=[{}, ValueError('registry checksum mismatch')]) as registry, self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                updater.verify_candidate(root, output, policy, candidate)
            self.assertEqual(registry.call_args.args, ({'name': 'cc'}, '1.0.0', 'a' * 64))

    def test_duplicate_and_missing_graph_entries(self):
        package = {'name': 'cc', 'version': '1.0.0'}
        with self.assertRaisesRegex(ValueError, 'duplicate lock package'):
            updater.validate_graph([package, package], {})
        with self.assertRaisesRegex(ValueError, 'missing internal'):
            updater.validate_graph([package], {'app': '1.0.0'})
        with self.assertRaisesRegex(ValueError, 'missing or ambiguous'):
            updater.validate_graph([dict(package, dependencies=['absent'])], {})
        with self.assertRaisesRegex(ValueError, 'missing or ambiguous'):
            updater.validate_graph([dict(package, dependencies=['cc']), dict(package, version='2.0.0')], {})
        updater.validate_graph([dict(package, dependencies=['cc 2.0.0']), dict(package, version='2.0.0')], {})

    def test_symlink_and_size_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'data').write_text('1234')
            (root / 'link').symlink_to(root / 'data')
            with self.assertRaisesRegex(ValueError, 'bounded regular'):
                updater.bounded_file(root / 'link')
            with self.assertRaisesRegex(ValueError, 'bounded regular'):
                updater.bounded_file(root / 'data', 3)

    def test_equal_version_git_migration_is_not_noop(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = {'git': 'https://github.com/atrinik/protocol', 'rev': 'a' * 40, 'version': VERSION}
            (root / 'policy').mkdir()
            (root / 'policy/released-dependencies.json').write_text(json.dumps({'schema_version': 1, 'repository': 'atrinik/client', 'dependencies': [dict(DEP, legacy_git=legacy)]}))
            (root / 'Cargo.toml').write_text('[workspace.dependencies]\natrinik-protocol = {git="https://github.com/atrinik/protocol", rev="' + 'a' * 40 + '", version="' + VERSION + '"}\n')
            lock = 'version=4\n[[package]]\nname="atrinik-protocol"\nversion="' + VERSION + '"\nsource="' + updater.REGISTRY + '"\nchecksum="' + SHA + '"\n'
            def run(*args, **kwargs):
                if args[0] == 'cargo':
                    self.assertEqual(args, ('cargo', '+1.97.1', 'generate-lockfile'))
                    (root / 'Cargo.lock').write_text(lock)
                return 'a' * 40
            output = root / 'candidate'
            with patch('sys.argv', ['updater', 'prepare', '--root', str(root), '--output', str(output)]), patch.dict(updater.os.environ, {}, clear=True), patch.object(updater, 'source', return_value={'name': NAME, 'version': VERSION, 'sha256': SHA}), patch.object(updater, 'run', side_effect=run):
                updater.main()
            self.assertTrue((output / 'candidate.json').is_file())
            self.assertIn('atrinik-protocol = "=0.2.0"', (root / 'Cargo.toml').read_text())

    def test_authoritative_lock_mismatch_fails_without_executing_candidate(self):
        import io
        import tarfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = self.fixture(root, output)
            (output / 'candidate.json').write_text(json.dumps(candidate))
            archive = io.BytesIO()
            with tarfile.open(fileobj=archive, mode='w') as tar:
                tar.add(root / 'Cargo.toml', arcname='Cargo.toml')
            def run(*args, **kwargs):
                if args[:2] == ('git', 'rev-parse'):
                    return candidate['base']
                if args[0] == 'cargo':
                    self.assertEqual(args, ('cargo', '+1.97.1', 'generate-lockfile'))
                    (kwargs['cwd'] / 'Cargo.lock').write_text('different authoritative lock')
                return ''
            with patch.object(updater, 'verify_candidate'), patch.object(updater.subprocess, 'check_output', return_value=archive.getvalue()), patch.object(updater, 'run', side_effect=run), self.assertRaisesRegex(ValueError, 'authoritative Cargo resolution'):
                updater.verify_lock(root, output, policy)


class BotHeadTests(unittest.TestCase):
    def fixture(self, login='atrinik-crate-dependency-update[bot]', verified=True, changed='Cargo.toml'):
        commit = {'author': {'login': login}, 'committer': {'login': 'web-flow'}, 'commit': {'message': 'chore(deps): update atrinik-protocol to 0.2.0', 'verification': {'verified': verified, 'reason': 'valid'}}, 'parents': [{'sha': 'b' * 40}]}
        old = {'truncated': False, 'tree': [{'path': changed, 'mode': '100644', 'type': 'blob', 'sha': 'a' * 40}]}
        new = {'truncated': False, 'tree': [{'path': changed, 'mode': '100644', 'type': 'blob', 'sha': 'b' * 40}]}
        return patch.object(updater, 'gh_api', side_effect=[commit, {'status': 'ahead'}, old, new])

    def verify(self):
        updater.verify_owned_head('atrinik/client', {'head': {'sha': 'a' * 40}}, 'c' * 40, 'atrinik-crate-dependency-update[bot]')

    def test_verified_owned_head(self):
        with self.fixture():
            self.verify()

    def test_human_tip_on_bot_pr_preserved(self):
        with self.fixture(login='human'), self.assertRaisesRegex(ValueError, 'foreign commit'):
            self.verify()

    def test_spoofed_unsigned_bot_tip_preserved(self):
        with self.fixture(verified=False), self.assertRaisesRegex(ValueError, 'unverified'):
            self.verify()

    def test_unexpected_files_preserved(self):
        with self.fixture(changed='src/main.rs'), self.assertRaisesRegex(ValueError, 'unexpected edits'):
            self.verify()


class PublisherRaceTests(unittest.TestCase):
    def exercise(self, owned=False, pr_change=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = CandidateBoundaryTests().fixture(root, output)
            policy.update(repository='atrinik/client', repository_id=1)
            (output / 'candidate.json').write_text(json.dumps(candidate))
            pr = OwnershipTests().pr()
            pr['number'] = 1
            pr['state'] = 'open'
            calls = []
            def api(endpoint, method='GET', body=None):
                calls.append((endpoint, method, body))
                if endpoint.startswith('installation/'):
                    return {'total_count': 1, 'repositories': [{'full_name': 'atrinik/client', 'id': 1, 'node_id': 'R_client_fixture'}]}
                if endpoint.endswith('/git/ref/heads/main'):
                    count = sum(call[0] == endpoint for call in calls)
                    return {'object': {'sha': candidate['base'] if count == 1 or owned else 'b' * 40}}
                if '/pulls?' in endpoint:
                    return [pr] if owned else []
                if '/git/matching-refs/' in endpoint:
                    return []
                if '/contents/' in endpoint:
                    data = b'[workspace.dependencies]\natrinik-protocol="=0.1.0"\n'
                    return {'encoding': 'base64', 'size': len(data), 'content': updater.base64.b64encode(data).decode()}
                if method == 'POST':
                    return {'sha': 'c' * 40}
                if '/git/commits/' in endpoint:
                    return {'tree': {'sha': 'd' * 40}}
                if endpoint.endswith('/pulls/1'):
                    if pr_change == 'closed':
                        return dict(pr, state='closed')
                    if pr_change == 'retargeted':
                        return dict(pr, base={'ref': 'maintenance'})
                    return dict(pr, head=dict(pr['head'], sha='e' * 40))
                self.fail(f'unexpected API operation: {endpoint} {method}')
            def run(*args, **kwargs):
                if args[:2] == ('git', 'rev-parse'):
                    return candidate['base']
                if args[:2] == ('git', 'status'):
                    return ''
                if args[:2] == ('git', 'diff'):
                    return 'Cargo.lock\nCargo.toml'
                self.fail(f'unexpected command: {args}')
            expected = 'owned bot PR changed' if owned else 'main advanced before branch mutation'
            with patch.dict(updater.os.environ, {'CRATE_DEPENDENCY_UPDATE_APP_SLUG': 'atrinik-crate-dependency-update'}), patch.object(updater, 'verify_candidate'), patch.object(updater, 'verify_owned_head'), patch.object(updater, 'run', side_effect=run), patch.object(updater, 'gh_api', side_effect=api), self.assertRaisesRegex(ValueError, expected):
                updater.publish(root, output, policy)
            self.assertFalse(any((endpoint == 'graphql' or '/git/refs' in endpoint) and method != 'GET' for endpoint, method, body in calls))
            self.assertFalse(any('/pulls' in endpoint and method != 'GET' for endpoint, method, body in calls))

    def test_main_advances_before_ref_write(self):
        self.exercise()

    def test_human_pushes_to_owned_pr_before_ref_write(self):
        self.exercise(owned=True)

    def test_owned_pr_closed_before_ref_write(self):
        self.exercise(owned=True, pr_change='closed')

    def test_owned_pr_retargeted_before_ref_write(self):
        self.exercise(owned=True, pr_change='retargeted')


class ManifestOptionTests(unittest.TestCase):
    def test_table_options_preserved(self):
        text = '[workspace.dependencies]\natrinik-protocol = { version = "=0.1.0", default-features = false, features = ["wire", "serde"] }\n'
        result = updater.updated_manifest(text, {'dependencies': [DEP]}, {NAME: {'version': VERSION}})
        before, after = updater.tomllib.loads(text), updater.tomllib.loads(result)
        before['workspace']['dependencies'][NAME]['version'] = '=' + VERSION
        self.assertEqual(before, after)
        self.assertFalse(after['workspace']['dependencies'][NAME]['default-features'])

    def test_candidate_feature_removal_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = CandidateBoundaryTests().fixture(root, output)
            path = root / 'Cargo.toml'
            path.write_text(path.read_text().replace('atrinik-protocol = "=0.1.0"', 'atrinik-protocol = {version="=0.1.0", features=["wire"], default-features=false}'))
            with patch.object(updater, 'source', return_value=record), self.assertRaisesRegex(ValueError, 'exceed exact dependency pins'):
                updater.verify_candidate(root, output, policy, candidate)
            (output / 'Cargo.toml').write_text(updater.updated_manifest(path.read_text(), policy, {NAME: record}))
            candidate['files']['Cargo.toml'] = hashlib.sha256((output / 'Cargo.toml').read_bytes()).hexdigest()
            with patch.object(updater, 'source', return_value=record), patch.object(updater, 'registry'):
                updater.verify_candidate(root, output, policy, candidate)

    def test_unchanged_table_is_noop(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'policy').mkdir()
            (root / 'policy/released-dependencies.json').write_text(json.dumps({'schema_version': 1, 'dependencies': [DEP]}))
            manifest = '[workspace.dependencies]\natrinik-protocol = {version="=0.2.0", features=["wire"], default-features=false}\n'
            (root / 'Cargo.toml').write_text(manifest)
            with patch('sys.argv', ['updater', 'prepare', '--root', str(root), '--output', str(root / 'candidate')]), patch.dict(updater.os.environ, {}, clear=True), patch.object(updater, 'source', return_value={'name': NAME, 'version': VERSION, 'sha256': SHA}), patch.object(updater, 'run') as run:
                updater.main()
            run.assert_not_called()
            self.assertFalse((root / 'candidate').exists())
            self.assertEqual((root / 'Cargo.toml').read_text(), manifest)


class PublisherReviewRegressionTests(unittest.TestCase):
    def exercise(self, *, retained=False, foreign=False, drift=False, new_pr=False,
                 deleted_fork=False, same_title=False, other_base=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'candidate'
            output.mkdir()
            record, candidate, policy = CandidateBoundaryTests().fixture(root, output)
            policy.update(repository='atrinik/client', repository_id=1)
            (output / 'candidate.json').write_text(json.dumps(candidate))
            pr = OwnershipTests().pr(title=candidate['title'], branch='feature/contributor', bot='human')
            if deleted_fork:
                pr['head']['repo'] = None
            if other_base:
                pr['base']['ref'] = 'maintenance'
            calls = []
            def api(endpoint, method='GET', body=None):
                calls.append((endpoint, method, body))
                if endpoint.startswith('installation/'):
                    return {'total_count': 1, 'repositories': [{'full_name': 'atrinik/client', 'id': 1, 'node_id': 'R_client_fixture'}]}
                if endpoint.endswith('/git/ref/heads/main'):
                    return {'object': {'sha': candidate['base']}}
                if '/pulls?' in endpoint:
                    if new_pr and sum('/pulls?' in call[0] for call in calls) > 1:
                        return [OwnershipTests().pr()]
                    return [pr] if deleted_fork or same_title or other_base else []
                if '/git/matching-refs/' in endpoint:
                    if not retained:
                        return []
                    count = sum('/git/matching-refs/' in call[0] for call in calls)
                    return [{'ref': 'refs/heads/automation/released-crate-dependencies', 'object': {'type': 'commit', 'sha': ('e' if drift and count > 1 else 'b') * 40}}]
                if '/contents/' in endpoint:
                    self.assertFalse(deleted_fork or other_base, 'unavailable or wrong-base PR should not be fetched')
                    data = b'[workspace.dependencies]\natrinik-protocol="=0.1.0"\n'
                    return {'encoding': 'base64', 'size': len(data), 'content': updater.base64.b64encode(data).decode()}
                if endpoint == 'graphql':
                    return {'data': {'updateRefs': {'clientMutationId': None}}}
                if method in {'POST', 'PATCH'}:
                    return {'sha': 'c' * 40}
                if '/git/commits/' in endpoint:
                    return {'tree': {'sha': 'd' * 40}}
                self.fail(f'unexpected API operation: {endpoint} {method}')
            def run(*args, **kwargs):
                if args[:2] == ('git', 'rev-parse'):
                    return candidate['base']
                if args[:2] == ('git', 'status'):
                    return ''
                if args[:2] == ('git', 'diff'):
                    return 'Cargo.lock\nCargo.toml'
                self.fail(f'unexpected command: {args}')
            error = 'foreign commit' if foreign else 'bot branch advanced' if drift else 'acquired an open PR' if new_pr else None
            with patch.dict(updater.os.environ, {'CRATE_DEPENDENCY_UPDATE_APP_SLUG': 'atrinik-crate-dependency-update'}), patch.object(updater, 'verify_candidate'), patch.object(updater, 'verify_owned_head', side_effect=ValueError('foreign commit') if foreign else None) as ownership, patch.object(updater, 'run', side_effect=run), patch.object(updater, 'gh_api', side_effect=api):
                if error:
                    with self.assertRaisesRegex(ValueError, error):
                        updater.publish(root, output, policy)
                else:
                    updater.publish(root, output, policy)
            mutations = [call for call in calls if (call[0] == 'graphql' or '/git/refs' in call[0]) and call[1] != 'GET']
            if error:
                self.assertEqual(mutations, [])
            else:
                self.assertEqual(len(mutations), 1)
                self.assertEqual(mutations[0][0:2], ('graphql', 'POST'))
                update = mutations[0][2]['variables']['input']['refUpdates'][0]
                self.assertFalse(update['force'])
                self.assertEqual(update['beforeOid'], ('b' if retained else '0') * 40)
                if retained:
                    ownership.assert_called_once_with('atrinik/client', {'head': {'sha': 'b' * 40}}, candidate['base'], 'atrinik-crate-dependency-update[bot]')
                    commits = [call for call in calls if call[0].endswith('/git/commits')]
                    self.assertEqual(commits[0][2]['parents'], [candidate['base'], 'b' * 40])
                self.assertEqual(sum(endpoint.endswith('/pulls') and method == 'POST' for endpoint, method, body in calls), 1)
            self.assertFalse(any(method == 'DELETE' for endpoint, method, body in calls))

    def test_retained_closed_or_merged_bot_ref_reused(self):
        self.exercise(retained=True)

    def test_foreign_retained_ref_preserved(self):
        self.exercise(retained=True, foreign=True)

    def test_retained_ref_advance_preserved(self):
        self.exercise(retained=True, drift=True)

    def test_new_pr_on_retained_ref_stops_mutation(self):
        self.exercise(retained=True, new_pr=True)

    def test_deleted_fork_skipped(self):
        self.exercise(deleted_fork=True)

    def test_matching_title_with_wrong_pins_does_not_suppress(self):
        self.exercise(same_title=True)

    def test_non_main_pr_does_not_suppress(self):
        self.exercise(other_base=True)


class MetadataIsolationTests(unittest.TestCase):
    def setUp(self):
        updater._DOCUMENTS.clear()
        updater._OFFLINE_DOCUMENTS = False

    def tearDown(self):
        updater._DOCUMENTS.clear()
        updater._OFFLINE_DOCUMENTS = False

    def test_document_gets_deduplicated(self):
        with patch.object(updater, 'fetch', return_value=b'{"id":1}') as fetch:
            for _ in range(6):
                self.assertEqual(updater.document('https://api.github.com/repos/atrinik/protocol'), {'id': 1})
            self.assertEqual(fetch.call_count, 1)

    def test_cached_documents_never_fallback_to_network(self):
        updater._OFFLINE_DOCUMENTS = True
        with patch.object(updater, 'fetch') as fetch, self.assertRaisesRegex(ValueError, 'cache miss'):
            updater.document('https://api.github.com/repos/atrinik/protocol')
        fetch.assert_not_called()

    def test_auth_token_only_on_bounded_github_metadata_requests(self):
        from unittest.mock import MagicMock
        response = MagicMock()
        response.__enter__.return_value.read.return_value = b'{}'
        opener = MagicMock()
        opener.open.return_value = response
        with patch.dict(updater.os.environ, {'GITHUB_READ_TOKEN': 'fixture-read-token'}), patch.object(updater.urllib.request, 'build_opener', return_value=opener):
            updater.fetch('https://api.github.com/repos/atrinik/protocol')
            request = opener.open.call_args.args[0]
            self.assertEqual(request.get_header('Authorization'), 'Bearer fixture-read-token')
            for url in ['https://crates.io/api/v1/crates/log/0.1.0', 'https://github.com/atrinik/protocol/releases/download/v1.0.0/provenance.json']:
                updater.fetch(url)
                self.assertIsNone(opener.open.call_args.args[0].get_header('Authorization'))
            with self.assertRaisesRegex(ValueError, 'metadata URL'):
                updater.fetch('https://api.github.com/user')
            with self.assertRaisesRegex(ValueError, 'redirect prohibited'):
                updater.Redirect().redirect_request(request, None, 302, 'redirect', {}, 'https://github.com/attacker')

    def test_cargo_commands_reject_metadata_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'policy').mkdir()
            (root / 'policy/released-dependencies.json').write_text('{"schema_version":1}')
            for command in ('prepare', 'consumer', 'verify-lock'):
                with patch('sys.argv', ['updater', command, '--root', str(root)]), patch.dict(updater.os.environ, {'GITHUB_READ_TOKEN': 'fixture-token'}, clear=True), patch.object(updater, 'run') as run, self.assertRaisesRegex(ValueError, 'must not inherit metadata credentials'):
                    updater.main()
                run.assert_not_called()

    def test_direct_cargo_execution_rejects_read_token(self):
        with patch.dict(updater.os.environ, {'GITHUB_READ_TOKEN': 'fixture-token'}), patch.object(updater.subprocess, 'check_output') as execute, self.assertRaisesRegex(ValueError, 'credential-free'):
            updater.run('cargo', '+1.97.1', 'check')
        execute.assert_not_called()

    def test_metadata_file_requires_correct_base_and_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'metadata.json'
            value = {'schema_version': 1, 'repository': 'atrinik/client', 'base': 'a' * 40, 'documents': {'https://api.github.com/repos/atrinik/protocol': {'id': 123}}}
            path.write_text(json.dumps(value))
            policy = {'repository': 'atrinik/client', 'dependencies': [DEP]}
            with patch.object(updater, 'run', return_value='a' * 40):
                updater.load_metadata(path, root, policy)
            self.assertEqual(updater.document('https://api.github.com/repos/atrinik/protocol'), {'id': 123})
            with patch.object(updater, 'run', return_value='b' * 40), self.assertRaisesRegex(ValueError, 'different source base'):
                updater.load_metadata(path, root, policy)
            value['repository'] = 'attacker/client'
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'invalid trusted metadata'):
                updater.load_metadata(path, root, policy)


class MetadataRequestBudgetTests(unittest.TestCase):
    setUp = MetadataIsolationTests.setUp
    tearDown = MetadataIsolationTests.tearDown
    def test_six_dependencies_use_nine_public_api_gets(self):
        responses = {}
        dependencies = []
        for repo_name, names in [('protocol', ['atrinik-protocol']),
                                 ('renderer', ['atrinik-renderer', 'atrinik-renderer-core', 'atrinik-renderer-assets']),
                                 ('toolkit', ['atrinik-toolkit', 'atrinik-toolkit-core'])]:
            repo = 'atrinik/' + repo_name
            tag = 'v1.0.0'
            revision = 'a' * 40
            records = [{'name': name, 'version': VERSION, 'sha256': SHA, 'asset': f'{name}-{VERSION}.crate'} for name in names]
            manifest = {'schema_version': 1, 'repository': repo, 'repository_id': 123,
                        'source_tag': tag, 'revision': revision, 'crates': records}
            files = {'crate-release.json': json.dumps(manifest).encode(),
                     'provenance.json': json.dumps({'revision': revision, 'version': '1.0.0'}).encode(),
                     'SHA256SUMS': ''.join(f'{SHA}  {name}-{VERSION}.crate\n' for name in names).encode()}
            files.update({record['asset']: DATA for record in records})
            assets = []
            for name, data in files.items():
                url = f'https://github.com/{repo}/releases/download/{tag}/{name}'
                responses[url] = data
                assets.append({'name': name, 'state': 'uploaded', 'size': len(data),
                               'browser_download_url': url, 'digest': 'sha256:' + hashlib.sha256(data).hexdigest()})
            prefix = 'https://api.github.com/repos/' + repo
            responses[prefix] = json.dumps({'id': 123, 'full_name': repo}).encode()
            responses[prefix + '/releases?per_page=30'] = json.dumps([{'tag_name': tag, 'immutable': True,
                'draft': False, 'prerelease': False, 'published_at': '2026-10-11', 'assets': assets}]).encode()
            responses[prefix + '/git/ref/tags/' + tag] = json.dumps({'object': {'type': 'commit', 'sha': revision}}).encode()
            dependencies.extend({'name': name, 'repository': repo, 'repository_id': 123} for name in names)
        with patch.object(updater, 'fetch', side_effect=lambda url: responses[url]) as fetch, patch.object(updater, 'registry'):
            for dependency in dependencies:
                self.assertEqual(updater.source(dependency)['version'], VERSION)
        public_calls = [call.args[0] for call in fetch.call_args_list if call.args[0].startswith('https://api.github.com/')]
        self.assertEqual(len(public_calls), 9)
        self.assertEqual(len(set(public_calls)), 9)


class AtomicRefTests(unittest.TestCase):
    def exercise(self, expected, current, after='c' * 40, fast_forward=True, rejected=False):
        state = {'head': current}
        def server(endpoint, method, body):
            self.assertEqual((endpoint, method), ('graphql', 'POST'))
            self.assertIn('updateRefs', body['query'])
            self.assertEqual(body['variables']['input']['repositoryId'], 'R_client_fixture')
            updates = body['variables']['input']['refUpdates']
            self.assertEqual(len(updates), 1)
            update = updates[0]
            self.assertEqual(update['name'], 'refs/heads/automation/released-crate-dependencies')
            self.assertFalse(update['force'])
            if update['beforeOid'] != (state['head'] or '0' * 40) or not fast_forward:
                return {'data': {'updateRefs': None}, 'errors': [{'message': 'precondition failed'}]}
            state['head'] = update['afterOid']
            return {'data': {'updateRefs': {'clientMutationId': None}}}
        with patch.object(updater, 'gh_api', side_effect=server):
            if rejected:
                with self.assertRaisesRegex(ValueError, 'atomic branch update rejected'):
                    updater.update_ref('R_client_fixture', 'automation/released-crate-dependencies', expected, after)
                self.assertEqual(state['head'], current)
            else:
                updater.update_ref('R_client_fixture', 'automation/released-crate-dependencies', expected, after)
                self.assertEqual(state['head'], after)

    def test_exact_old_head_fast_forward(self):
        self.exercise('b' * 40, 'b' * 40)

    def test_concurrent_ancestor_reset_rejected_even_when_fast_forward_possible(self):
        self.exercise('b' * 40, 'a' * 40, rejected=True)

    def test_non_fast_forward_rejected(self):
        self.exercise('b' * 40, 'b' * 40, fast_forward=False, rejected=True)

    def test_create_collision_preserves_concurrent_branch(self):
        self.exercise(None, 'b' * 40, rejected=True)

    def test_create_requires_absent_branch(self):
        self.exercise(None, None)

    def test_main_update_and_deletion_prohibited(self):
        with patch.object(updater, 'gh_api') as api:
            with self.assertRaisesRegex(ValueError, 'unexpected updater branch'):
                updater.update_ref('R_client_fixture', 'main', 'a' * 40, 'b' * 40)
            with self.assertRaisesRegex(ValueError, 'invalid ref update identity'):
                updater.update_ref('R_client_fixture', 'automation/released-crate-dependencies', 'a' * 40, '0' * 40)
        api.assert_not_called()


if __name__ == '__main__':
    unittest.main()
