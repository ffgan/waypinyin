#!/usr/bin/env python3
"""Build with Cargo and explicitly export the generated dictionary."""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
os.chdir(root)
prefix = os.environ.get('PREFIX', '/usr/local')
datadir = os.environ.get('DATADIR', prefix + '/share')
env = dict(os.environ, WAYPINYIN_DATADIR=datadir + '/waypinyin')
command = shlex.split(os.environ.get('CARGO', 'cargo')) + [
    'build', '--release', '--message-format=json-render-diagnostics'
] + shlex.split(os.environ.get('CARGO_BUILD_FLAGS', '--locked'))
metadata = json.loads(subprocess.check_output(shlex.split(os.environ.get('CARGO', 'cargo')) + ['metadata', '--no-deps', '--format-version=1', '--offline'], text=True))
package_id = next(p['id'] for p in metadata['packages'] if p['name'] == 'waypinyin')
image = binary = None
with subprocess.Popen(command, env=env, stdout=subprocess.PIPE, text=True) as proc:
    for line in proc.stdout:
        event = json.loads(line)
        if event.get('reason') == 'build-script-executed' and event['package_id'] == package_id:
            image = Path(event['out_dir']) / 'lexicon.bin'
        if event.get('reason') == 'compiler-artifact' and event['target']['name'] == 'waypinyin' and event.get('executable'):
            binary = Path(event['executable'])
    if proc.wait():
        raise SystemExit(proc.returncode)
if image is None or binary is None:
    raise SystemExit('Cargo did not report the waypinyin binary and dictionary')
(root / 'build').mkdir(exist_ok=True)
shutil.copyfile(image, root / 'build/lexicon.bin')
shutil.copy2(binary, root / 'build/waypinyin')
(root / 'build/paths.json').write_text(json.dumps({'prefix': prefix, 'datadir': datadir}))

subprocess.run(["python3", str(root / "scripts/dependency_inventory.py")], env=env, check=True)
