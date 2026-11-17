#!/usr/bin/env python3
"""Exercise a real IBus daemon on private buses and disposable user directories."""
import argparse
import os
from pathlib import Path
import subprocess
import signal
import shlex
import xml.etree.ElementTree as ET
import tempfile
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument('binary', type=Path)
parser.add_argument('lexicon', type=Path)
args = parser.parse_args()
binary, lexicon = args.binary.resolve(), args.lexicon.resolve()
assert binary.is_file() and lexicon.is_file()

with tempfile.TemporaryDirectory(prefix='waypinyin-ibus-') as temporary:
    work = Path(temporary)
    env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / 'config'),
               XDG_DATA_HOME=str(work / 'data'), XDG_CACHE_HOME=str(work / 'cache'),
               XDG_RUNTIME_DIR=str(work), WAYPINYIN_LEXICON=str(lexicon),
               IBUS_ADDRESS='unix:path=' + str(work / 'ibus.sock'))
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_DISPLAY', None)
    env['GIO_USE_VFS'] = 'local'
    env['GIO_USE_VOLUME_MONITOR'] = 'unix'
    components = work / 'components'
    components.mkdir()
    component = ET.parse(Path(__file__).resolve().parent.parent / 'share/waypinyin.xml')
    component.find('exec').text = shlex.quote(str(binary))
    component.find('engines/engine/setup').text = ''
    component.write(components / 'waypinyin.xml')
    env['IBUS_COMPONENT_PATH'] = str(components)
    config = work / 'bus.conf'
    config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=' + str(work) +
                      '</listen><auth>EXTERNAL</auth><policy context="default">'
                      '<allow send_destination="*"/><allow eavesdrop="true"/>'
                      '<allow own="*"/></policy></busconfig>')
    children = []
    try:
        with (work / 'session.log').open('w+') as session_log, (work / 'ibus.log').open('w+') as ibus_log:
            session = subprocess.Popen(['dbus-daemon', '--config-file=' + str(config), '--nofork', '--print-address=1'],
                                       stdout=subprocess.PIPE, stderr=session_log, text=True, env=env, start_new_session=True)
            children.append(session)
            env['DBUS_SESSION_BUS_ADDRESS'] = session.stdout.readline().strip()
            assert env['DBUS_SESSION_BUS_ADDRESS']
            daemon = subprocess.Popen(['ibus-daemon', '--verbose', '--single', '--panel=disable', '--config=disable',
                                       '--emoji-extension=disable', '--cache=none', '--address=' + env['IBUS_ADDRESS']],
                                      stdout=ibus_log, stderr=ibus_log, env=env, start_new_session=True)
            children.append(daemon)
            for _ in range(100):
                if (work / 'ibus.sock').exists():
                    break
                if daemon.poll() is not None:
                    raise RuntimeError('private ibus-daemon exited')
                time.sleep(0.05)
            else:
                raise RuntimeError('private IBus socket did not appear')
            # A separate client sees only the isolated environment, never the user's bus.
            client = Path(__file__).with_name('ibus_client.py')
            result = subprocess.run([sys.executable, str(client)], env=env,
                                    text=True, capture_output=True, timeout=30)
            if result.returncode:
                ibus_log.seek(0)
                raise RuntimeError(result.stdout + result.stderr + ibus_log.read())
            print(result.stdout.strip())
    finally:
        for child in reversed(children):
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        for child in reversed(children):
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
