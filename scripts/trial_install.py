#!/usr/bin/env python3
"""Install a reviewed stage with an exact file manifest and reversible backup.

Only files in the installation stage are touched. Learning logs, imports and
preferences are outside that manifest. Reload only this prefix's engine.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import shlex
import signal
import subprocess
import tempfile
import time

def require(condition,message):
    if not condition:raise SystemExit(message)

def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''): h.update(block)
    return h.hexdigest()

def atomic_copy(source,target):
    target.parent.mkdir(parents=True,exist_ok=True)
    fd,name=tempfile.mkstemp(prefix='.'+target.name+'.',dir=target.parent)
    os.close(fd)
    try:
        shutil.copy2(source,name)
        os.replace(name,target)
    finally: Path(name).unlink(missing_ok=True)

def select_engine(name):
    # IBus can return failure while an on-demand component is still starting.
    # Observe the actual selection and retry before reporting a reload failure.
    for attempt in range(10):
        subprocess.run(['ibus','engine',name],check=False,timeout=10)
        current=subprocess.check_output(['ibus','engine'],text=True,timeout=10).strip()
        if current==name:return attempt
        time.sleep(.2)
    raise SystemExit('installed files are ready, but IBus did not select '+name)

def reload_engine(prefix):
    current=subprocess.check_output(['ibus','engine'],text=True,timeout=10).strip()
    binary=str(prefix/'bin/waypinyin')
    if current=='waypinyin':
        subprocess.run(['ibus','engine','xkb:us::eng'],check=True,timeout=10)
    targets=[]
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit(): continue
        try:
            if proc.stat().st_uid!=os.getuid():continue
            executable=os.readlink(proc/'exe').removesuffix(' (deleted)')
            args=(proc/'cmdline').read_bytes().split(b'\0')
            if executable==binary and len([a for a in args if a])==1:
                targets.append(int(proc.name))
        except (OSError,PermissionError):continue
    for pid in targets:
        try:os.kill(pid,signal.SIGTERM)
        except ProcessLookupError:pass
    for _ in range(100):
        if not any(Path(f'/proc/{pid}').exists() for pid in targets):break
        time.sleep(.05)
    retries=select_engine('waypinyin') if current=='waypinyin' else 0
    return {'previous_engine':current,'terminated_engine_pids':targets,
            'current_engine':subprocess.check_output(['ibus','engine'],text=True,timeout=10).strip(),
            'selection_retries':retries}

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--stage',type=Path)
    p.add_argument('--prefix',type=Path)
    p.add_argument('--backup',type=Path)
    p.add_argument('--rollback',type=Path)
    p.add_argument('--no-reload',action='store_true')
    a=p.parse_args()
    if a.rollback:
        backup=a.rollback.resolve();manifest=json.loads((backup/'manifest.json').read_text())
        prefix=Path(manifest['prefix'])
        # Validate every backup and installed generation before any writes.
        for row in manifest['files']:
            target=Path(row['target'])
            require(target.resolve().is_relative_to(prefix) and not target.is_symlink(),'unsafe rollback target: '+str(target))
            if target.exists():require(sha(target) in {row['new_sha256'],row.get('old_sha256')},'installed file changed: '+str(target))
            if row['existed']:require(sha(backup/'files'/row['relative'])==row['old_sha256'],'backup checksum mismatch: '+str(target))
        for row in manifest['files']:
            target=Path(row['target'])
            if row['existed']:atomic_copy(backup/'files'/row['relative'],target)
            else:target.unlink(missing_ok=True)
        result={'rolled_back':str(backup)}
    else:
        require(a.stage and a.prefix and a.backup,'--stage, --prefix and --backup are required')
        stage=a.stage.resolve();prefix=a.prefix.resolve();backup=a.backup.resolve()
        require(prefix!=Path('/') and not backup.exists(),'use a non-root prefix and a new backup directory')
        rows=[]
        for source in sorted(stage.rglob('*')):
            if source.is_dir():continue
            require(source.is_file() and not source.is_symlink(),'stage must contain regular files')
            relative=source.relative_to(stage);target=Path('/')/relative
            require(target.resolve().is_relative_to(prefix) and not target.is_symlink(),'unsafe install target: '+str(target))
            row={'relative':str(relative),'target':str(target),'new_sha256':sha(source),'existed':target.exists()}
            if target.exists():
                row['old_sha256']=sha(target)
                destination=backup/'files'/relative
                destination.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(target,destination)
            rows.append(row)
        require(any(row['target']==str(prefix/'bin/waypinyin') for row in rows),'stage has no Waypinyin executable')
        backup.mkdir(parents=True,exist_ok=True)
        backup.chmod(0o700)
        private_log=prefix/'share/waypinyin/user.txt'
        if private_log.is_file():
            private=backup/'private';private.mkdir(mode=0o700)
            shutil.copy2(private_log,private/'user.txt')
            (private/'user.txt').chmod(0o600)
        (backup/'manifest.json').write_text(json.dumps({'prefix':str(prefix),'files':rows},indent=2)+'\n')
        shutil.copy2(__file__,backup/'trial_install.py')
        for row in rows:atomic_copy(stage/row['relative'],Path(row['target']))
        result={'installed_files':len(rows),'backup':str(backup),
                'rollback_command':shlex.join(['python3',str(backup/'trial_install.py'),'--rollback',str(backup)])}
    if not a.no_reload:result['reload']=reload_engine(prefix)
    print(json.dumps(result,indent=2))

if __name__=='__main__':main()
