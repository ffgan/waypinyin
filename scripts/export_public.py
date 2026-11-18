#!/usr/bin/env python3
"""Export cleared source into an empty directory, never copying Git history."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
output = args.output.resolve()
if output == root or root in output.parents and output.parts[len(root.parts)] != 'build':
    raise SystemExit('use a separate directory or a directory under build/')
if output.exists() and any(output.iterdir()):
    raise SystemExit('export destination must be empty; use a new directory')
subprocess.run(['python3',str(root / 'scripts/check_sources.py')],check=True)
inventory = json.loads((root / 'data/sources.json').read_text())
cleared = {s['path'] for s in inventory['files'] if s['license_status'] in ('verified-local', 'verified-upstream')}
excluded = [s for s in inventory['files'] if s['path'] not in cleared]
# This frequency sample includes readings selected from the local complete lexicon.
local_only = {root / 'tests/quality_words.tsv'}
files = [root / name for name in ['Cargo.toml','Cargo.lock','build.rs','Makefile','LICENSE',
         'README.md','README.en.md','CHANGELOG.md','CONTRIBUTING.md','SECURITY.md','.gitignore','.gitattributes']]
for directory in ['src','share','scripts','docs','packaging','tests','examples','.github','LICENSES']:
    files += [p for p in (root / directory).rglob('*') if p.is_file()
              and '__pycache__' not in p.parts and p.suffix != '.pyc']
files += [root / p for p in cleared] + [root / 'data/NOTICE']
output.mkdir(parents=True,exist_ok=True)
for source in sorted(files):
    if source in local_only:
        continue
    if source.is_symlink(): raise SystemExit(f'refusing symlink: {source}')
    target = output / source.relative_to(root)
    target.parent.mkdir(parents=True,exist_ok=True)
    shutil.copy2(source,target)
inventory['files'] = [s for s in inventory['files'] if s['path'] in cleared]
inventory['excluded_files'] = inventory.get('excluded_files', []) + excluded
inventory['distribution'] = 'public-source-with-reviewed-data-and-original-demo'
(output / 'data/sources.json').write_text(json.dumps(inventory,ensure_ascii=False,indent=2)+'\n')
# A fresh index can never retain the original corpus objects or old commits.
assert not (output / '.git').exists()
assert all(not (output / s['path']).exists() for s in inventory['excluded_files'])
subprocess.run(['python3',str(output / 'scripts/check_sources.py'),'--release'],check=True)
print(f'Public source exported to {output}; excluded {len(inventory["excluded_files"])} data files and all Git history')
