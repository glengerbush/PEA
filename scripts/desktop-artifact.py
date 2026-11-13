#!/usr/bin/env python3
"""Carry exact-revision build outputs from main to a release, never a stale cache."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

BINARY = 'target/release/pea'
RESOURCES = 'apps/desktop/src-tauri/resources/'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def identity(root, platform):
    return {
        'schema': 1,
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'version': json.loads((root / 'package.json').read_text())['version'],
        'platform': platform,
    }


def allowed(name):
    parts = name.split('/')
    return (name == BINARY or name.startswith(RESOURCES)) and all(p not in ('', '.', '..') for p in parts) and '\\' not in name


def create(root, archive, expected):
    paths = [root / BINARY, *sorted((root / RESOURCES).rglob('*'))]
    files = {}
    with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
        for path in paths:
            if path.is_symlink():
                raise ValueError(f'Symlink not allowed: {path}')
            if path.is_dir():
                continue
            name = path.relative_to(root).as_posix()
            data = path.read_bytes()
            files[name] = digest(data)
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            entry.mode = 0o755 if name == BINARY else 0o644
            tar.addfile(entry, io.BytesIO(data))
        if RESOURCES + 'frontend/index.html' not in files:
            raise ValueError('Frontend resources missing')
        data = json.dumps({**expected, 'files': files}, sort_keys=True).encode()
        entry = tarfile.TarInfo('manifest.json')
        entry.size = len(data)
        tar.addfile(entry, io.BytesIO(data))


def restore(root, archive, expected):
    # Validate every member before writing anything into the checkout.
    with tarfile.open(archive, 'r:gz') as tar:
        members = tar.getmembers()
        names = [m.name for m in members]
        if len(names) != len(set(names)) or any(not m.isfile() for m in members):
            raise ValueError('Duplicate or non-file artifact member')
        if any(n != 'manifest.json' and not allowed(n) for n in names):
            raise ValueError('Unexpected artifact path')
        manifest = json.load(tar.extractfile('manifest.json'))
        if any(manifest.get(k) != v for k, v in expected.items()):
            raise ValueError('Artifact revision, version or platform differs')
        files = manifest['files']
        if set(files) != set(names) - {'manifest.json'} or not {BINARY, RESOURCES + 'frontend/index.html'} <= set(files):
            raise ValueError('Incomplete artifact')
        for name, checksum in files.items():
            if digest(tar.extractfile(name).read()) != checksum:
                raise ValueError(f'Artifact checksum mismatch: {name}')
        # A clean runner normally has neither destination; clear stale resources
        # so an obsolete file cannot leak into the new package.
        shutil.rmtree(root / RESOURCES, ignore_errors=True)
        for name in files:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(tar.extractfile(name).read())
            path.chmod(0o755 if name == BINARY else 0o644)
    print(f"Verified desktop inputs for {expected['revision']} ({expected['platform']})")


def api(endpoint):
    return json.loads(subprocess.check_output(['gh', 'api', endpoint], text=True))


def find_artifact(repo, expected, job_name):
    revision = expected['revision']
    name = f"desktop-inputs-{revision}-{expected['platform']}"
    runs = api(f'repos/{repo}/actions/workflows/warm-desktop-cache.yml/runs?branch=main&head_sha={revision}&per_page=100')['workflow_runs']
    for run in runs:
        if run['head_sha'] != revision or run['head_branch'] != 'main' or run['event'] not in ('push', 'workflow_dispatch'):
            continue
        if run['head_repository']['full_name'] != repo:
            continue
        jobs = api(f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100")['jobs']
        if not any(j['name'] == job_name and j['conclusion'] == 'success' for j in jobs):
            continue
        artifacts = api(f"repos/{repo}/actions/runs/{run['id']}/artifacts?per_page=100")['artifacts']
        if any(a['name'] == name and not a['expired'] for a in artifacts):
            return run['id'], name
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['create', 'restore', 'fetch'])
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--platform', required=True)
    parser.add_argument('--job-name')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    expected = identity(root, args.platform)
    if args.command == 'create':
        create(root, args.archive, expected)
    elif args.command == 'restore':
        restore(root, args.archive, expected)
    else:
        repo = os.environ['GITHUB_REPOSITORY']
        try:
            found = find_artifact(repo, expected, args.job_name)
        except subprocess.CalledProcessError:
            found = None
            print('::warning::Artifact lookup unavailable; using a normal build.')
        if not found:
            print('No completed exact-revision artifact; using a normal build.')
            return
        run, name = found
        with tempfile.TemporaryDirectory() as directory:
            # A missing/expired download fails before touching build inputs.
            result = subprocess.run(['gh', 'run', 'download', str(run), '--repo', repo, '--name', name, '--dir', directory])
            if result.returncode:
                print('::warning::Artifact download unavailable; using a normal build.')
                return
            restore(root, Path(directory) / 'desktop-inputs.tar.gz', expected)
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write('restored=true\n')


if __name__ == '__main__':
    main()
