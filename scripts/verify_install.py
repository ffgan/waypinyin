#!/usr/bin/env python3
"""Verify staged files and registration without executing a desktop session."""
import argparse
import json
from pathlib import Path
import subprocess
import shlex
import xml.etree.ElementTree as ET

parser = argparse.ArgumentParser()
parser.add_argument('stage', type=Path)
parser.add_argument('--prefix', default='/usr/local')
parser.add_argument('--bindir')
parser.add_argument('--datadir')
parser.add_argument('--mandir')
args = parser.parse_args()
bindir = args.bindir or args.prefix + '/bin'
datadir = args.datadir or args.prefix + '/share'
mandir = args.mandir or datadir + '/man'
def staged(path):
    return args.stage / path.lstrip('/')
for path in [bindir + '/waypinyin', datadir + '/waypinyin/lexicon.bin',
             datadir + '/applications/waypinyin-settings.desktop',
             datadir + '/icons/hicolor/scalable/apps/waypinyin.svg',
             mandir + '/man1/waypinyin.1', datadir + '/licenses/waypinyin/LICENSE',
             datadir + '/licenses/waypinyin/sources.json',
             datadir + '/doc/waypinyin/README.en.md']:
    assert staged(path).is_file(), f'missing {path}'
component = ET.parse(staged(datadir + '/ibus/component/waypinyin.xml')).getroot()
assert shlex.split(component.findtext('exec')) == [bindir + '/waypinyin']
assert shlex.split(component.findtext('engines/engine/setup')) == [bindir + '/waypinyin', '--settings']
assert '@BINDIR@' not in ET.tostring(component).decode()
subprocess.run(['desktop-file-validate',str(staged(datadir + '/applications/waypinyin-settings.desktop'))],check=True)
inventory = json.loads(staged(datadir + '/licenses/waypinyin/dependency-licenses.json').read_text())
for package in inventory['packages']:
    for notice in package['notice_files']:
        assert staged(datadir + '/licenses/waypinyin/dependency-notices/' + notice).is_file(), notice
print('staged installation and desktop metadata verified')
