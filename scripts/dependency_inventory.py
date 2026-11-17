#!/usr/bin/env python3
"""Record declared licenses of the resolved graph; declarations are not legal review."""
import argparse
import json
import hashlib
import os
import shlex
import shutil
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent.parent
host = next(line.removeprefix('host: ') for line in subprocess.check_output(['rustc', '-vV'], text=True).splitlines() if line.startswith('host: '))
target = os.environ.get('CARGO_BUILD_TARGET', host)
feature_flags = []
flags = iter(shlex.split(os.environ.get('CARGO_BUILD_FLAGS', '')))
for flag in flags:
    if flag in ('--no-default-features', '--all-features') or flag.startswith('--features='):
        feature_flags.append(flag)
    elif flag == '--features':
        feature_flags.extend([flag, next(flags)])
metadata = json.loads(subprocess.check_output(shlex.split(os.environ.get('CARGO', 'cargo')) + ['metadata','--offline','--format-version=1','--filter-platform',target] + feature_flags, cwd=root, text=True))
parser = argparse.ArgumentParser()
parser.add_argument('output', nargs='?', type=Path, default=root / 'build/dependency-licenses.json')
parser.add_argument('--artifacts', type=Path, help='package IDs emitted by the actual Cargo build')
parser.add_argument('--require-notices', action='store_true')
args = parser.parse_args()
output = args.output
if args.artifacts:
    built = set(json.loads(args.artifacts.read_text()))
    metadata['packages'] = [p for p in metadata['packages'] if p['id'] in built]
output.parent.mkdir(parents=True, exist_ok=True)
notices = output.parent / 'dependency-notices'
if notices.exists():
    shutil.rmtree(notices)
notices.mkdir()
supplements = json.loads((root / 'LICENSES/dependency-supplements.json').read_text())
packages = []
for p in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
    copied = []
    # Registry crates carry their own notices. Do not scan the workspace/build tree.
    if p['source'] is not None:
        crate = Path(p['manifest_path']).parent
        candidates = [f for f in crate.rglob('*') if f.is_file() and
                      f.name.lower().startswith(('license', 'licence', 'copying', 'notice', 'copyright', 'unlicense', 'ofl', 'ufl'))]
        if p['license_file']:
            candidates.append(crate / p['license_file'])
        for source in sorted(set(candidates)):
            if not source.resolve().is_relative_to(crate.resolve()):
                raise SystemExit('license file escapes crate: ' + str(source))
            relative = Path(p['name'] + '-' + p['version']) / source.relative_to(crate)
            target_file = notices / relative
            target_file.parent.mkdir(parents=True, exist_ok=True)
            target_file.write_bytes(source.read_bytes())
            copied.append(relative.as_posix())
    for supplement in supplements.values():
        if supplement['packages'].get(p['name']) != p['version']:
            continue
        for evidence in supplement['evidence']:
            source = root / evidence['local_path']
            body = source.read_bytes()
            if hashlib.sha256(body).hexdigest() != evidence['sha256']:
                raise SystemExit('supplemental notice changed: ' + str(source))
            relative = Path(p['name'] + '-' + p['version']) / ('upstream-' + source.name)
            target_file = notices / relative
            target_file.parent.mkdir(parents=True, exist_ok=True)
            target_file.write_bytes(body)
            copied.append(relative.as_posix())
    packages.append({'name': p['name'], 'version': p['version'], 'source': p['source'],
                     'declared_license': p['license'], 'license_file': p['license_file'],
                     'notice_files': copied})
output.write_text(json.dumps({'target': target, 'feature_flags': feature_flags,
    'inventory_kind': 'compiled-artifacts' if args.artifacts else 'metadata-graph',
    'note': 'Declared licenses and verbatim notices. Compiled-artifact inventories include host/build dependencies; select the actual linked-license expression separately.',
    'packages': packages}, indent=2) + '\n')
missing = [p['name'] for p in packages if p['source'] is not None and not p['notice_files']]
print(f'{len(packages)} packages: {output}; {len(missing)} without notice files')
if missing:
    print('Review crates without notice files: ' + ', '.join(missing))
    if args.require_notices:
        raise SystemExit('Missing dependency notices; supply reviewed notices before staging a release build')
