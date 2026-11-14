#!/usr/bin/env python3
"""Verify staged files and registration without executing a desktop session."""
import argparse
from pathlib import Path
import subprocess
import shlex
import xml.etree.ElementTree as ET

parser = argparse.ArgumentParser()
parser.add_argument('stage', type=Path)
parser.add_argument('--prefix', default='/usr/local')
args = parser.parse_args()
base = args.stage / args.prefix.lstrip('/')
for name in ['bin/waypinyin','share/waypinyin/lexicon.bin',
             'share/applications/waypinyin-settings.desktop',
             'share/icons/hicolor/scalable/apps/waypinyin.svg',
             'share/man/man1/waypinyin.1','share/licenses/waypinyin/LICENSE',
             'share/licenses/waypinyin/sources.json','share/doc/waypinyin/README.en.md']:
    assert (base / name).is_file(), f'missing {name}'
component = ET.parse(base / 'share/ibus/component/waypinyin.xml').getroot()
assert shlex.split(component.findtext('exec')) == [args.prefix + '/bin/waypinyin']
assert shlex.split(component.findtext('engines/engine/setup')) == [args.prefix + '/bin/waypinyin', '--settings']
assert '@BINDIR@' not in ET.tostring(component).decode()
subprocess.run(['desktop-file-validate',str(base / 'share/applications/waypinyin-settings.desktop')],check=True)
print('staged installation and desktop metadata verified')
