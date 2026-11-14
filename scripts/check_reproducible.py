#!/usr/bin/env python3
"""Generate the dictionary twice in fresh processes and compare exact bytes."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
host = next(s.removeprefix('host: ') for s in subprocess.check_output(['rustc','-vV'],text=True).splitlines() if s.startswith('host: '))
with tempfile.TemporaryDirectory(prefix='waypinyin-repro-') as temporary:
    work = Path(temporary)
    compiler = work / 'compiler'
    subprocess.run(['rustc','--edition=2021','-O',str(root / 'build.rs'),'-o',str(compiler)],check=True)
    hashes = []
    for name in ['one','two']:
        out = work / name
        out.mkdir()
        env = dict(os.environ, CARGO_MANIFEST_DIR=str(root), OUT_DIR=str(out), HOST=host, TARGET=host)
        subprocess.run([str(compiler)],env=env,stdout=subprocess.DEVNULL,check=True)
        hashes.append(hashlib.sha256((out / 'lexicon.bin').read_bytes()).hexdigest())
    assert hashes[0] == hashes[1], f'dictionary images differ: {hashes}'
    print('dictionary reproducible:', hashes[0])
