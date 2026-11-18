#!/usr/bin/env python3
"""Build with Cargo and explicitly export the generated dictionary."""
import json
import hashlib
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
artifacts = set()
with subprocess.Popen(command, env=env, stdout=subprocess.PIPE, text=True) as proc:
    for line in proc.stdout:
        event = json.loads(line)
        if event.get('reason') == 'compiler-artifact':
            artifacts.add(event['package_id'])
        if event.get('reason') == 'build-script-executed' and event['package_id'] == package_id:
            image = Path(event['out_dir']) / 'lexicon.bin'
        if event.get('reason') == 'compiler-artifact' and event['target']['name'] == 'waypinyin' and event.get('executable'):
            binary = Path(event['executable'])
    if proc.wait():
        raise SystemExit(proc.returncode)
if image is None or binary is None:
    raise SystemExit('Cargo did not report the waypinyin binary and dictionary')
(root / 'build').mkdir(exist_ok=True)
# A failed export or notice collection must not leave an installable mixed build.
(root / 'build/paths.json').unlink(missing_ok=True)
shutil.copyfile(image, root / 'build/lexicon.bin')
model = image.with_suffix('.lm')
if model.is_file():
    shutil.copyfile(model, root / 'build/lexicon.lm')
    model_source = os.environ.get('WAYPINYIN_MODEL_BODY')
    metadata = Path(model_source).with_suffix('.json') if model_source else None
    attribution = Path(model_source).with_suffix('.attribution.tsv') if model_source else None
    for name in ('language-model.json', 'language-model-attribution.tsv'):
        (root / 'build' / name).unlink(missing_ok=True)
    if metadata and metadata.is_file():
        details = json.loads(metadata.read_text())
        assert details['model_body_sha256'] == hashlib.sha256(Path(model_source).read_bytes()).hexdigest(), 'model provenance does not match trained payload'
        details['dictionary_sha256'] = hashlib.sha256(image.read_bytes()).hexdigest()
        details['model_sha256'] = hashlib.sha256(model.read_bytes()).hexdigest()
        (root / 'build/language-model.json').write_text(json.dumps(details, ensure_ascii=False, indent=2) + '\n')
    if attribution and attribution.is_file():
        shutil.copyfile(attribution, root / 'build/language-model-attribution.tsv')
else:
    (root / 'build/lexicon.lm').unlink(missing_ok=True)
    for name in ('language-model.json', 'language-model-attribution.tsv'):
        (root / 'build' / name).unlink(missing_ok=True)
shutil.copy2(binary, root / 'build/waypinyin')

artifact_file = root / 'build/dependency-packages.json'
artifact_file.write_text(json.dumps(sorted(artifacts)) + '\n')
subprocess.run(["python3", str(root / "scripts/dependency_inventory.py"),
                "--require-notices", "--artifacts", str(artifact_file)], env=env, check=True)
(root / 'build/paths.json').write_text(json.dumps({'prefix': prefix, 'datadir': datadir}))
