#!/usr/bin/env python3
"""Re-fetch pinned upstream evidence and compare bytes; not run during a build."""
import hashlib
import json
from pathlib import Path
import urllib.request

root = Path(__file__).resolve().parent.parent
inventory = json.loads((root / 'data/sources.json').read_text())
supplements = json.loads((root / 'LICENSES/dependency-supplements.json').read_text())
records = inventory['files'] + [dict(s, license_status='verified-upstream') for s in supplements.values()]
for source in records:
    if source['license_status'] != 'verified-upstream':
        continue
    for evidence in source['evidence']:
        with urllib.request.urlopen(evidence['url'], timeout=30) as response:
            body = response.read()
        actual = hashlib.sha256(body).hexdigest()
        if actual != evidence['sha256']:
            raise SystemExit('upstream evidence mismatch: ' + evidence['url'])
        if evidence.get('local_path'):
            local = root / evidence['local_path']
            if local.read_bytes() != body:
                raise SystemExit('local evidence mismatch: ' + str(local))
        print('verified:', evidence['url'])
