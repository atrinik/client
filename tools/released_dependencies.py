#!/usr/bin/env python3
"""Verify released registry dependencies and propose fenced application updates."""
# Copyright (c) 2026 Atrinik contributors. SPDX-License-Identifier: MIT
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tarfile
import tempfile
import tomllib
import urllib.error
import urllib.request

REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'
MAX_BYTES = 32 * 1024 * 1024
MAX_PACKAGES = 2000


def require(condition, message):
    if not condition:
        raise ValueError(message)


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True, timeout=600).strip()


class Redirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, url):
        require(urllib.parse.urlparse(url).scheme == 'https' and
                urllib.parse.urlparse(url).hostname in {
                    'github.com', 'release-assets.githubusercontent.com',
                    'objects.githubusercontent.com', 'static.crates.io', 'crates.io'},
                'unexpected registry/release redirect')
        return super().redirect_request(request, fp, code, msg, headers, url)


def fetch(url, absent=False):
    try:
        with urllib.request.build_opener(Redirect).open(
                urllib.request.Request(url, headers={'User-Agent': 'atrinik-dependency-readiness'}),
                timeout=30) as response:
            data = response.read(MAX_BYTES + 1)
            require(len(data) <= MAX_BYTES, 'download exceeds bound')
            return data
    except urllib.error.HTTPError as error:
        if absent and error.code == 404:
            return None
        raise


def document(url):
    return json.loads(fetch(url))


def exact_dependencies(root, policy):
    for config in (root / '.cargo/config', root / '.cargo/config.toml'):
        if config.exists():
            configuration = tomllib.loads(config.read_text())
            require(not configuration.get('source') and not configuration.get('patch'), 'Cargo source configuration override prohibited')
    require(not any(key.startswith('CARGO_SOURCE_') or key.startswith('CARGO_REGISTRIES_') for key in os.environ), 'Cargo registry environment override prohibited')
    manifest = tomllib.loads((root / 'Cargo.toml').read_text())
    require(not manifest.get('patch') and not manifest.get('replace'), 'source override prohibited')
    result = {}
    for dep in policy['dependencies']:
        value = manifest['workspace']['dependencies'][dep['name']]
        if isinstance(value, dict):
            if value == dep.get('legacy_git'):
                result[dep['name']] = value['version'].removeprefix('=')
                continue
            require(set(value) <= {'version', 'features', 'default-features'}, 'Git/path/registry override prohibited')
            value = value['version']
        require(isinstance(value, str) and re.fullmatch(r'=\d+\.\d+\.\d+', value) is not None, 'dependency requires exact registry version')
        result[dep['name']] = value[1:]
    for path in root.glob('crates/*/Cargo.toml'):
        child = tomllib.loads(path.read_text())
        require(child['package'].get('publish') is False, f'internal crate publication enabled: {path}')
        require(not child.get('patch') and not child.get('replace'), 'member source override prohibited')
        dependency_tables = [child] + list(child.get('target', {}).values())
        for section in ('dependencies', 'dev-dependencies', 'build-dependencies'):
            for name, value in ((name, value) for table in dependency_tables for name, value in table.get(section, {}).items()):
                if name in result:
                    require(value == {'workspace': True}, 'member dependency bypasses workspace registry pin')
    return result


def index_path(name):
    require(isinstance(name, str) and re.fullmatch(r'[A-Za-z0-9_-]{1,64}', name), 'invalid crate name')
    name = name.lower()
    if len(name) <= 2:
        return f'{len(name)}/{name}'
    if len(name) == 3:
        return f'3/{name[0]}/{name}'
    return f'{name[:2]}/{name[2:4]}/{name}'


def registry(dep, version, checksum):
    name = dep['name']
    path = index_path(name)
    require(isinstance(version, str) and re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.+-]+)?', version), 'invalid registry version')
    require(isinstance(checksum, str) and re.fullmatch('[0-9a-f]{64}', checksum), 'invalid registry checksum')
    api = fetch(f'https://crates.io/api/v1/crates/{name}/{version}', absent=True)
    index = fetch(f'https://index.crates.io/{path}', absent=True)
    require(api is not None and index is not None, f'missing registry package {name} = {version}')
    value = json.loads(api)['version']
    entries = [json.loads(line) for line in index.splitlines() if line]
    matches = [entry for entry in entries if entry['vers'] == version]
    require(len(matches) == 1, f'missing registry index package {name} = {version}')
    entry = matches[0]
    require(value.get('crate') == name and value.get('num') == version and
            entry.get('name') == name, 'registry identity mismatch')
    require(value.get('yanked') is False and entry.get('yanked') is False, 'registry package yanked')
    require(value.get('checksum') == checksum and entry.get('cksum') == checksum, 'registry checksum mismatch')
    data = fetch(f'https://crates.io/api/v1/crates/{name}/{version}/download')
    require(hashlib.sha256(data).hexdigest() == checksum, 'registry download checksum mismatch')
    return entry


def source(dep, version=None):
    legacy = dep.get('legacy', [])
    if isinstance(legacy, dict):
        legacy = [legacy]
    historical = [record for record in legacy if record['version'] == version]
    require(len(historical) <= 1, 'duplicate historical registry pin')
    if version is not None and historical:
        record = dict(historical[0], name=dep['name'])
        registry(dep, version, record['sha256'])
        return record
    repo = dep['repository']
    api = f'https://api.github.com/repos/{repo}'
    identity = document(api)
    require(identity.get('id') == dep['repository_id'] and identity.get('full_name') == repo, 'upstream repository identity changed')
    releases = document(f'{api}/releases?per_page=30')
    require(isinstance(releases, list) and len(releases) <= 30, 'release inventory truncated; narrow release discovery')
    require(len(releases) < 30, 'release history exceeds discovery bound; cannot establish latest candidate')
    records = []
    for release in releases:
        if release['draft'] or release['prerelease']:
            continue
        assets = [item for item in release['assets'] if item['name'] == 'crate-release.json']
        if not assets:
            continue
        require(release.get('immutable') is True and release.get('published_at'), 'source release is mutable')
        tag = release['tag_name']
        require(re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', tag), 'invalid source tag')
        def asset(name):
            matches = [item for item in release['assets'] if item['name'] == name]
            require(len(matches) == 1 and matches[0]['state'] == 'uploaded', 'missing source release asset')
            item = matches[0]
            url = f'https://github.com/{repo}/releases/download/{tag}/{name}'
            require(item['browser_download_url'] == url and item['size'] <= MAX_BYTES, 'source asset location mismatch')
            data = fetch(url)
            require(len(data) == item['size'] and item.get('digest') == 'sha256:' + hashlib.sha256(data).hexdigest(), 'source asset digest mismatch')
            return data
        manifest = json.loads(asset('crate-release.json'))
        require(manifest['schema_version'] == 1 and manifest['repository'] == repo and
                manifest['source_tag'] == tag and str(manifest['repository_id']) == str(dep['repository_id']), 'source manifest identity mismatch')
        revision = manifest['revision']
        require(re.fullmatch('[0-9a-f]{40}', revision), 'invalid release revision')
        ref = document(f'{api}/git/ref/tags/{tag}')['object']
        if ref['type'] == 'tag':
            ref = document(f'{api}/git/tags/{ref["sha"]}')['object']
        require(ref['type'] == 'commit' and ref['sha'] == revision, 'upstream tag drift')
        provenance = json.loads(asset('provenance.json'))
        require(provenance['revision'] == revision and provenance['version'] == tag[1:], 'source provenance mismatch')
        sums = asset('SHA256SUMS').decode('ascii').splitlines()
        for crate in manifest['crates']:
            if crate['name'] != dep['name'] or (version is not None and crate['version'] != version):
                continue
            require(re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', crate['version']) and re.fullmatch('[0-9a-f]{64}', crate['sha256']), 'invalid crate identity')
            name = f'{dep["name"]}-{crate["version"]}.crate'
            require(crate['asset'] == name and sums.count(crate['sha256'] + '  ' + name) == 1, 'source checksum manifest mismatch')
            require(hashlib.sha256(asset(name)).hexdigest() == crate['sha256'], 'source crate checksum mismatch')
            records.append(dict(crate, revision=revision, source_tag=tag))
    require(records, f'no reviewed source release for {dep["name"]}' + (f' = {version}' if version else ''))
    identities = [(record['name'], record['version']) for record in records]
    require(len(identities) == len(set(identities)), 'duplicate released crate version')
    records.sort(key=lambda record: tuple(map(int, record['version'].split('.'))), reverse=True)
    record = records[0]
    registry(dep, record['version'], record['sha256'])
    return record


def lock_check(root, records):
    packages = tomllib.loads((root / 'Cargo.lock').read_text())['package']
    for name, record in records.items():
        matches = [item for item in packages if item['name'] == name]
        if record.get('legacy_git'):
            legacy = record['legacy_git']
            expected = f'git+{legacy["git"]}?rev={legacy["rev"]}#{legacy["rev"]}'
            require(len(matches) == 1 and matches[0]['version'] == record['version'] and matches[0].get('source') == expected, 'legacy Git lock mismatch')
            continue
        require(len(matches) == 1 and matches[0]['version'] == record['version'] and
                matches[0].get('source') == REGISTRY and matches[0].get('checksum') == record['sha256'],
                f'lockfile registry identity mismatch: {name}')


def gh_api(endpoint, method='GET', body=None):
    args = ['gh', 'api', endpoint, '--method', method]
    if body is None:
        return json.loads(run(*args))
    process = subprocess.run(args + ['--input', '-'], input=json.dumps(body), text=True,
                             capture_output=True, check=True, timeout=60)
    return json.loads(process.stdout)


def choose_pr(pulls, version_title, branch, bot, repo):
    require(len(pulls) < 100, 'PR inventory truncated; refusing mutation')
    same = [pr for pr in pulls if pr['title'] == version_title]
    if same:
        return 'waiting', same[0]
    owned = [pr for pr in pulls if pr['head']['ref'] == branch]
    if owned:
        pr = owned[0]
        require(pr['user']['login'] == bot and pr['head']['repo']['full_name'] == repo and
                pr['base']['ref'] == 'main', 'updater branch belongs to another contributor')
        return 'owned', pr
    return 'create', None


def bounded_file(path, maximum=MAX_BYTES):
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except OSError as error:
        raise ValueError('candidate file is not a bounded regular file') from error
    with os.fdopen(descriptor, 'rb') as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_size <= maximum, 'candidate file is not a bounded regular file')
        data = stream.read(maximum + 1)
        require(len(data) <= maximum, 'candidate file exceeds bound')
        return data


def validate_graph(packages, internal):
    require(isinstance(packages, list) and 0 < len(packages) <= MAX_PACKAGES, 'invalid lock package inventory')
    identities = [(item['name'], item['version'], item.get('source')) for item in packages]
    require(len(identities) == len(set(identities)), 'duplicate lock package')
    require(set(internal) <= {item['name'] for item in packages}, 'missing internal lock package')
    for package in packages:
        require(set(package) <= {'name', 'version', 'source', 'checksum', 'dependencies'} and
                {'name', 'version'} <= set(package), 'unexpected lock package schema')
        edges = package.get('dependencies', [])
        require(isinstance(edges, list) and len(edges) == len(set(edges)), 'duplicate lock dependency')
        for edge in edges:
            match = re.fullmatch(r'([A-Za-z0-9_-]+)(?: ([^ ()]+))?(?: \(([^()]+)\))?', edge)
            require(match is not None, 'invalid lock dependency reference')
            name, version, source_name = match.groups()
            matches = [item for item in packages if item['name'] == name and
                       (version is None or item['version'] == version) and
                       (source_name is None or item.get('source') == source_name)]
            require(len(matches) == 1, 'missing or ambiguous lock dependency')


def verify_owned_head(repo, pr, main, bot):
    """A PR author alone never authorizes replacing its current branch tree."""
    head = pr['head']['sha']
    commit = gh_api(f'repos/{repo}/commits/{head}')
    require(commit.get('committer', {}).get('login') == bot and
            commit.get('author', {}).get('login') == bot and
            commit['commit']['verification']['verified'] is True and
            commit['commit']['verification']['reason'] == 'valid',
            'bot branch contains an unverified or foreign commit; preserving branch')
    require(commit['commit']['message'].startswith('chore(deps): update ') and
            1 <= len(commit['parents']) <= 2, 'unexpected bot commit shape; preserving branch')
    base = commit['parents'][0]['sha']
    comparison = gh_api(f'repos/{repo}/compare/{base}...{main}')
    require(comparison['status'] in {'ahead', 'identical'}, 'bot base is not trusted main history')
    def tree(revision):
        result = gh_api(f'repos/{repo}/git/trees/{revision}?recursive=1')
        require(result.get('truncated') is False, 'bot tree inventory truncated')
        return {item['path']: (item['mode'], item['type'], item['sha'])
                for item in result['tree'] if item['type'] != 'tree'}
    before, after = tree(base), tree(head)
    changed = {name for name in before.keys() | after.keys() if before.get(name) != after.get(name)}
    require(changed and changed <= {'Cargo.toml', 'Cargo.lock'} and
            all(after.get(name, (None, None))[:2] == ('100644', 'blob') for name in changed),
            'bot branch contains unexpected edits; preserving branch')


def verify_candidate(root, output, policy, candidate):
    require(set(candidate) == {'repository', 'base', 'records', 'title', 'files'}, 'unexpected candidate schema')
    require(isinstance(candidate['base'], str) and re.fullmatch('[0-9a-f]{40}', candidate['base']), 'invalid candidate base')
    require(isinstance(candidate['records'], dict) and isinstance(candidate['files'], dict), 'invalid candidate object')
    pins = exact_dependencies(root, policy)
    require(set(candidate['records']) == set(pins), 'candidate dependency scope changed')
    records = {}
    text = (root / 'Cargo.toml').read_text()
    for dep in policy['dependencies']:
        record = candidate['records'][dep['name']]
        require(re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', record['version']), 'invalid candidate version')
        authenticated = source(dep, record['version'])
        require(record == authenticated, 'candidate source provenance changed')
        require(tuple(map(int, record['version'].split('.'))) >= tuple(map(int, pins[dep['name']].split('.'))), 'candidate downgrade prohibited')
        name = dep['name']
        text, count = re.subn(r'(?m)^' + re.escape(name) + r' = [^\n]+$',
                             f'{name} = "={record["version"]}"', text)
        require(count == 1, 'unsupported trusted manifest declaration')
        records[name] = authenticated
    require(bounded_file(output / 'Cargo.toml', 131072).decode() == text, 'candidate changes exceed exact dependency pins')
    expected_title = 'chore(deps): update ' + ', '.join(f'{name} to {record["version"]}' for name, record in records.items())
    require(candidate['title'] == expected_title and set(candidate['files']) == {'Cargo.toml', 'Cargo.lock'}, 'candidate metadata scope changed')
    lock = tomllib.loads(bounded_file(output / 'Cargo.lock').decode())
    require(set(lock) == {'version', 'package'} and lock['version'] == 4, 'unexpected lock schema')
    packages = lock['package']
    trusted_workspace_version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
    internal = {tomllib.loads(path.read_text())['package']['name']: tomllib.loads(path.read_text())['package'].get('version', trusted_workspace_version) for path in root.glob('crates/*/Cargo.toml')}
    internal = {name: trusted_workspace_version if isinstance(version, dict) else version for name, version in internal.items()}
    validate_graph(packages, internal)
    for package in packages:
        if package['name'] in internal:
            require('source' not in package and 'checksum' not in package and package['version'] == internal[package['name']], 'internal lock package identity changed')
        else:
            require(package.get('source') == REGISTRY and re.fullmatch('[0-9a-f]{64}', package.get('checksum', '')), 'candidate lock contains non-registry source')
            registry({'name': package['name']}, package['version'], package['checksum'])
    lock_check(output, records)
    for name in ('Cargo.toml', 'Cargo.lock'):
        require(hashlib.sha256(bounded_file(output / name)).hexdigest() == candidate['files'][name], 'candidate artifact drift')


def publish(root, output, policy):
    candidate = json.loads(bounded_file(output / 'candidate.json', 131072))
    require(candidate['repository'] == policy['repository'], 'candidate repository drift')
    repo = policy['repository']
    branch = 'automation/released-crate-dependencies'
    require(os.environ.get('CRATE_DEPENDENCY_UPDATE_APP_SLUG') == 'atrinik-crate-dependency-update', 'unexpected GitHub App identity')
    bot = 'atrinik-crate-dependency-update[bot]'
    installation = gh_api('installation/repositories?per_page=100')
    require(installation['total_count'] == 1 and any(item['full_name'] == repo and item['id'] == policy['repository_id'] for item in installation['repositories']), 'unexpected App installation scope')
    main = gh_api(f'repos/{repo}/git/ref/heads/main')['object']['sha']
    require(main == candidate['base'] and run('git', 'rev-parse', 'HEAD', cwd=root) == main, 'main advanced; rerun credential-free preparation')
    require(not run('git', 'status', '--porcelain', '--untracked-files=all', cwd=root), 'trusted publisher checkout is dirty')
    verify_candidate(root, output, policy, candidate)
    pulls = gh_api(f'repos/{repo}/pulls?state=open&per_page=100')
    require(len(pulls) < 100, 'PR inventory truncated; refusing mutation')
    for existing in pulls:
        head_repo = existing['head']['repo']['full_name']
        content = gh_api(f'repos/{head_repo}/contents/Cargo.toml?ref={existing["head"]["sha"]}')
        require(content['encoding'] == 'base64' and content['size'] <= 131072, 'PR manifest exceeds bound')
        manifest = tomllib.loads(base64.b64decode(content['content']).decode())
        dependencies = manifest.get('workspace', {}).get('dependencies', {})
        if all(dependencies.get(name) == '=' + record['version'] or
               (isinstance(dependencies.get(name), dict) and dependencies[name].get('version') == '=' + record['version'] and
                not any(key in dependencies[name] for key in ('git', 'path', 'registry')))
               for name, record in candidate['records'].items()):
            print('Existing PR waits for these registry versions; preserving contributor branch and text.')
            return
    mode, pr = choose_pr(pulls, candidate['title'], branch, bot, repo)
    if mode == 'waiting':
        print('Existing PR waits for this version; preserving contributor branch and text.')
        return
    if mode == 'owned':
        verify_owned_head(repo, pr, main, bot)
    # Credentials never execute build scripts or candidate code. Commit only validated data files.
    for name in ('Cargo.toml', 'Cargo.lock'):
        data = (output / name).read_bytes()
        require(hashlib.sha256(data).hexdigest() == candidate['files'][name], 'candidate artifact drift')
        (root / name).write_bytes(data)
    require(run('git', 'diff', '--name-only', cwd=root).splitlines() == ['Cargo.lock', 'Cargo.toml'], 'unexpected candidate diff')
    lock_check(root, candidate['records'])
    commit = gh_api(f'repos/{repo}/git/commits/{main}')
    entries = []
    for name in ('Cargo.toml', 'Cargo.lock'):
        blob = gh_api(f'repos/{repo}/git/blobs', 'POST', {
            'content': base64.b64encode((output / name).read_bytes()).decode(), 'encoding': 'base64'})
        entries.append({'path': name, 'mode': '100644', 'type': 'blob', 'sha': blob['sha']})
    tree = gh_api(f'repos/{repo}/git/trees', 'POST', {'base_tree': commit['tree']['sha'], 'tree': entries})
    parents = [main] if mode == 'create' else [main, pr['head']['sha']]
    update = gh_api(f'repos/{repo}/git/commits', 'POST', {'message': candidate['title'], 'tree': tree['sha'], 'parents': parents})
    require(gh_api(f'repos/{repo}/git/ref/heads/main')['object']['sha'] == main, 'main advanced before branch mutation')
    if mode == 'create':
        gh_api(f'repos/{repo}/git/refs', 'POST', {'ref': 'refs/heads/' + branch, 'sha': update['sha']})
    else:
        fresh = gh_api(f'repos/{repo}/pulls/{pr["number"]}')
        require(fresh['user']['login'] == bot and fresh['head']['sha'] == pr['head']['sha'] and
                fresh['head']['ref'] == branch and fresh['head']['repo']['full_name'] == repo,
                'owned bot PR changed before mutation')
        require(gh_api(f'repos/{repo}/git/ref/heads/{branch}')['object']['sha'] == pr['head']['sha'], 'bot branch advanced')
        # Previous bot commit is a parent, so a concurrent replacement cannot fast-forward.
        gh_api(f'repos/{repo}/git/refs/heads/{branch}', 'PATCH', {'sha': update['sha'], 'force': False})
    if mode == 'create':
        gh_api(f'repos/{repo}/pulls', 'POST', {'title': candidate['title'], 'head': branch,
               'base': 'main', 'body': 'Updates exact released registry dependencies and Cargo.lock.\n\nSource release provenance, crates.io API/index/archive checksums, and credential-free Cargo resolution passed. Required application CI and review must pass before merge. No automatic merge.'})
    # Preserve the existing PR title/body as contributor text.


def verify_lock(root, output, policy):
    """Run only in the credential-free resolver container with read-only inputs."""
    candidate = json.loads(bounded_file(output / 'candidate.json', 131072))
    require(run('git', 'rev-parse', 'HEAD', cwd=root) == candidate['base'] and
            not run('git', 'status', '--porcelain', '--untracked-files=all', cwd=root),
            'resolver requires clean trusted main')
    verify_candidate(root, output, policy, candidate)
    archive = subprocess.check_output(['git', 'archive', candidate['base']], cwd=root, timeout=60)
    with tempfile.TemporaryDirectory(prefix='atrinik-authoritative-resolve-') as directory:
        resolved = Path(directory)
        with tarfile.open(fileobj=io.BytesIO(archive)) as source_tree:
            source_tree.extractall(resolved, filter='data')
        (resolved / 'Cargo.toml').write_bytes(bounded_file(output / 'Cargo.toml'))
        run('cargo', '+1.97.1', 'generate-lockfile', cwd=resolved)
        require((resolved / 'Cargo.lock').read_bytes() == bounded_file(output / 'Cargo.lock'),
                'candidate lock differs from authoritative Cargo resolution')


def consumer(records):
    with tempfile.TemporaryDirectory(prefix='atrinik-external-resolve-') as directory:
        external = Path(directory)
        (external / 'src').mkdir()
        (external / 'src/lib.rs').write_text('')
        require(not any(record.get('legacy_git') for record in records.values()), 'external consumer requires released registry packages')
        deps = '\n'.join(f'{name} = "={record["version"]}"' for name, record in records.items())
        (external / 'Cargo.toml').write_text('[package]\nname="registry-consumer-proof"\nversion="0.0.0"\nedition="2024"\n[dependencies]\n' + deps + '\n')
        run('cargo', '+1.97.1', 'generate-lockfile', '--manifest-path', str(external / 'Cargo.toml'))
        run('cargo', '+1.97.1', 'check', '--locked', '--manifest-path', str(external / 'Cargo.toml'))
        lock_check(external, records)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=['ready', 'prepare', 'consumer', 'verify-lock', 'publish'])
    parser.add_argument('--root', type=Path, default=Path('.'))
    parser.add_argument('--output', '--candidate', type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    policy = json.loads((root / 'policy/released-dependencies.json').read_text())
    require(policy['schema_version'] == 1, 'unsupported dependency policy')
    if args.command == 'publish':
        publish(root, args.output, policy)
        return
    require(not any(os.environ.get(key) for key in ('GH_TOKEN', 'GITHUB_TOKEN', 'CARGO_REGISTRY_TOKEN', 'CARGO_REGISTRIES_CRATES_IO_TOKEN')), 'verification/build job must be credential-free')
    if args.command == 'verify-lock':
        verify_lock(root, args.output, policy)
        return
    pins = exact_dependencies(root, policy)
    records = {}
    for dep in policy['dependencies']:
        declaration = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['dependencies'][dep['name']]
        if args.command in {'ready', 'consumer'} and declaration == dep.get('legacy_git'):
            records[dep['name']] = {'version': pins[dep['name']], 'legacy_git': dep['legacy_git']}
        else:
            records[dep['name']] = source(dep, pins[dep['name']] if args.command in {'ready', 'consumer'} else None)
    if args.command == 'consumer':
        consumer(records)
        return
    if args.command == 'ready':
        lock_check(root, records)
        print('Dependency readiness policy verified.')
        return
    require(args.output is not None, 'prepare requires --output')
    declarations = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['dependencies']
    if all(declarations[name] == '=' + record['version'] for name, record in records.items()):
        print('No released dependency update.')
        return
    args.output.mkdir(parents=True, exist_ok=False)
    text = (root / 'Cargo.toml').read_text()
    for name, record in records.items():
        require(tuple(map(int, record['version'].split('.'))) >= tuple(map(int, pins[name].split('.'))), 'dependency downgrade prohibited')
        text, count = re.subn(r'(?m)^' + re.escape(name) + r' = [^\n]+$',
                             f'{name} = "={record["version"]}"', text)
        require(count == 1, 'unsupported dependency declaration for update')
    (root / 'Cargo.toml').write_text(text)
    run('cargo', '+1.97.1', 'generate-lockfile', cwd=root)
    lock_check(root, records)
    title = 'chore(deps): update ' + ', '.join(f'{name} to {record["version"]}' for name, record in records.items())
    candidate = {'repository': policy['repository'], 'base': run('git', 'rev-parse', 'HEAD', cwd=root),
                 'records': records, 'title': title, 'files': {}}
    for name in ('Cargo.toml', 'Cargo.lock'):
        data = (root / name).read_bytes()
        (args.output / name).write_bytes(data)
        candidate['files'][name] = hashlib.sha256(data).hexdigest()
    (args.output / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, urllib.error.URLError, subprocess.SubprocessError) as error:
        raise SystemExit(f'dependency readiness/update failed: {error}') from error
