#!/usr/bin/env python3
"""Stage an installation; never starts services or modifies user settings."""
import html
import json
import os
from pathlib import Path
import shutil
import shlex

root = Path(__file__).resolve().parent.parent
prefix = os.environ.get('PREFIX', '/usr/local')
bindir = os.environ.get('BINDIR', prefix + '/bin')
datadir = os.environ.get('DATADIR', prefix + '/share')
mandir = os.environ.get('MANDIR', datadir + '/man')
destdir = os.environ.get('DESTDIR', '')
for path in (prefix, bindir, datadir, mandir):
    if not Path(path).is_absolute() or '..' in Path(path).parts:
        raise SystemExit('installation directories must be absolute and contain no ..')
if json.loads((root / 'build/paths.json').read_text()) != {'prefix': prefix, 'datadir': datadir}:
    raise SystemExit('installation paths differ from build paths; rebuild with the intended PREFIX/DATADIR')

def install(source, directory, name=None, executable=False):
    target = Path(destdir + directory) / (name or Path(source).name)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(root / source, target)
    target.chmod(0o755 if executable else 0o644)
    return target

install('build/waypinyin', bindir, executable=True)
install('build/lexicon.bin', datadir + '/waypinyin')
xml = (root / 'share/waypinyin.xml').read_text().replace('@BINDIR@', html.escape(shlex.quote(bindir)))
component = Path(destdir + datadir + '/ibus/component/waypinyin.xml')
component.parent.mkdir(parents=True, exist_ok=True)
component.write_text(xml)
component.chmod(0o644)
install('share/waypinyin-settings.desktop', datadir + '/applications')
install('share/waypinyin.svg', datadir + '/icons/hicolor/scalable/apps')
install('share/waypinyin.1', mandir + '/man1')
for source in ['README.md', 'README.en.md', 'CHANGELOG.md', 'docs/RELEASE.md', 'docs/PACKAGING.md', 'docs/VALIDATION.md', 'docs/PUBLIC_SOURCE.md']:
    install(source, datadir + '/doc/waypinyin')
install('LICENSE', datadir + '/licenses/waypinyin')
install('data/NOTICE', datadir + '/licenses/waypinyin', 'DATA-NOTICE')
install('data/sources.json', datadir + '/licenses/waypinyin')
install('build/dependency-licenses.json', datadir + '/licenses/waypinyin')
for source in sorted((root / 'LICENSES').glob('*.txt')):
    install(source.relative_to(root), datadir + '/licenses/waypinyin')
