#!/usr/bin/env python3
"""Verify the data inventory; --release also rejects unresolved provenance."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--release', action='store_true')
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
inventory = json.loads((root / 'data/sources.json').read_text())
sources = inventory['files']
demo = inventory.get('demo_files', [])
expected = {str(p.relative_to(root)) for p in (root / 'data').glob('*.txt')}
assert {s['path'] for s in sources} == expected, 'data inventory is incomplete or duplicated'
assert len(sources) == len(expected), 'duplicate data inventory entries'
assert {s['path'] for s in demo} == {str(p.relative_to(root)) for p in (root / 'tests/fixtures/demo').glob('*.txt')}, 'demo inventory incomplete'
for source in inventory.get('excluded_files', []):
    assert not (root / source['path']).exists(), f"excluded data reappeared: {source['path']}"
blocked = []
for source in sources + demo:
    actual = hashlib.sha256((root / source['path']).read_bytes()).hexdigest()
    assert actual == source['sha256'], f"{source['path']}: checksum changed; review and update provenance"
    assert source['license_status'] in ('verified-local', 'verified-upstream', 'unresolved')
    if source['license_status'] == 'verified-upstream':
        assert source['upstream_revision'] and source['evidence'] and not source['required_evidence']
        for evidence in source['evidence']:
            assert evidence['url'].startswith('https://')
            if evidence.get('local_path'):
                assert hashlib.sha256((root / evidence['local_path']).read_bytes()).hexdigest() == evidence['sha256'], 'local upstream evidence changed'
    if source['license_status'] == 'unresolved':
        blocked.append(source['path'])
supplements = json.loads((root / 'LICENSES/dependency-supplements.json').read_text())
for supplement in supplements.values():
    assert supplement['commit'] and supplement['packages'] and supplement['evidence']
    for evidence in supplement['evidence']:
        assert hashlib.sha256((root / evidence['local_path']).read_bytes()).hexdigest() == evidence['sha256'], 'dependency license supplement changed'
print(f'{len(sources)} data files and {len(demo)} demo files verified; {len(blocked)} have unresolved provenance')
if args.release and blocked:
    raise SystemExit('Release blocked: ' + ', '.join(blocked))
