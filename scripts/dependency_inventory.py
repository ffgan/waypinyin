#!/usr/bin/env python3
"""Record declared licenses of the resolved graph; declarations are not legal review."""
import json
import os
import shlex
from pathlib import Path
import subprocess
import sys

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
packages = []
for p in sorted(metadata['packages'], key=lambda p: (p['name'],p['version'])):
    packages.append({'name':p['name'], 'version':p['version'], 'source':p['source'],
                     'declared_license':p['license'], 'license_file':p['license_file']})
output = Path(sys.argv[1]) if len(sys.argv) > 1 else root / 'build/dependency-licenses.json'
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps({'target':target, 'feature_flags':feature_flags, 'note':'Cargo metadata declarations, for the selected build target. Verify actual linked crates, notices and bundled assets before release.','packages':packages}, indent=2)+'\n')
print(f'{len(packages)} packages: {output}')
