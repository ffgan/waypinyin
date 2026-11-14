#!/usr/bin/env python3
"""Create a deterministic source archive, refusing unresolved data by default."""
import argparse
import gzip
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tomllib

parser = argparse.ArgumentParser()
parser.add_argument('--allow-unresolved', action='store_true', help='local review archive only; not a release')
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
subprocess.run(['python3', str(root / 'scripts/check_sources.py')] + ([] if args.allow_unresolved else ['--release']), check=True)
version = tomllib.loads((root / 'Cargo.toml').read_text())['package']['version']
name = f'waypinyin-{version}' + ('-review' if args.allow_unresolved else '')
# Never walk HOME, target, git metadata or imported user dictionaries.
directories = ['src','data','share','scripts','docs','packaging','tests','examples','.github','LICENSES']
files = [p for p in root.iterdir() if p.is_file() and p.name in {
    'Cargo.toml','Cargo.lock','build.rs','Makefile','LICENSE','README.md','README.en.md',
    'CHANGELOG.md','CONTRIBUTING.md','SECURITY.md','.gitignore'
}]
for directory in directories:
    files += [p for p in (root / directory).rglob('*') if p.is_file() and '__pycache__' not in p.parts and p.suffix != '.pyc']
stamp = int(os.environ.get('SOURCE_DATE_EPOCH', '0'))
(root / 'dist').mkdir(exist_ok=True)
output = root / 'dist' / (name + '.tar.gz')
with output.open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=stamp) as compressed, tarfile.open(fileobj=compressed, mode='w') as tar:
    for p in sorted(files):
        if p.is_symlink(): raise SystemExit(f'refusing source symlink: {p}')
        info = tarfile.TarInfo(str(Path(name) / p.relative_to(root)))
        body = p.read_bytes()
        info.size = len(body)
        info.mode = 0o755 if p.name == 'rules' else 0o644
        info.mtime = stamp
        tar.addfile(info, io.BytesIO(body))
checksum = hashlib.sha256(output.read_bytes()).hexdigest()
output.with_suffix(output.suffix + '.sha256').write_text(f'{checksum}  {output.name}\n')
print(output)
