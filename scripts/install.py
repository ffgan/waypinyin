#!/usr/bin/env python3
"""Stage an installation; never starts services or modifies user settings."""
import html
import json
import os
from pathlib import Path
import shutil
import shlex
import tempfile

root = Path(__file__).resolve().parent.parent
prefix = os.environ.get('PREFIX', '/usr/local')
bindir = os.environ.get('BINDIR', prefix + '/bin')
datadir = os.environ.get('DATADIR', prefix + '/share')
mandir = os.environ.get('MANDIR', datadir + '/man')
destdir = os.environ.get('DESTDIR', '')
for path in (prefix, bindir, datadir, mandir):
    if not Path(path).is_absolute() or '..' in Path(path).parts or any(c in path for c in '\n\r\t'):
        raise SystemExit('installation directories must be absolute and contain no .. or control whitespace')
for artifact in ('paths.json', 'waypinyin', 'lexicon.bin', 'dependency-licenses.json'):
    if not (root / 'build' / artifact).is_file():
        raise SystemExit('missing build/' + artifact + '; run make build with the intended installation paths first')
if not (root / 'build/dependency-notices').is_dir():
    raise SystemExit('missing dependency notices; run make build first')
if json.loads((root / 'build/paths.json').read_text()) != {'prefix': prefix, 'datadir': datadir}:
    raise SystemExit('installation paths differ from build paths; rebuild with the intended PREFIX/DATADIR')

def write_atomic(target, source=None, text=None, executable=False):
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.' + target.name + '.', dir=target.parent)
    try:
        with os.fdopen(fd, 'wb') as output:
            if text is not None:
                output.write(text.encode('utf-8'))
            else:
                with source.open('rb') as incoming:
                    shutil.copyfileobj(incoming, output)
            os.fchmod(output.fileno(), 0o755 if executable else 0o644)
        os.replace(temporary, target)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return target


def install(source, directory, name=None, executable=False):
    target = Path(destdir + directory) / (name or Path(source).name)
    return write_atomic(target, source=root / source, executable=executable)

install('build/waypinyin', bindir, executable=True)
install('build/lexicon.bin', datadir + '/waypinyin')
xml = (root / 'share/waypinyin.xml').read_text().replace('@BINDIR@', html.escape(shlex.quote(bindir)))
component = Path(destdir + datadir + '/ibus/component/waypinyin.xml')
write_atomic(component, text=xml)
desktop = Path(destdir + datadir + '/applications/waypinyin-settings.desktop')
# Desktop Exec uses its own quoting rules, including doubled percent literals.
executable = (bindir + '/waypinyin').replace('%', '%%')
executable = ''.join(('\\' * 4 if c == '\\' else '\\' * 2 + c)
                     if c in ('\\', '"', '`', '$') else c for c in executable)
desktop_text = (root / 'share/waypinyin-settings.desktop').read_text()
write_atomic(desktop, text=desktop_text.replace('Exec=waypinyin --settings', 'Exec="' + executable + '" --settings'))
install('share/waypinyin.svg', datadir + '/icons/hicolor/scalable/apps')
install('share/waypinyin.1', mandir + '/man1')
for source in ['README.md', 'README.en.md', 'CHANGELOG.md', 'docs/RELEASE.md', 'docs/PACKAGING.md', 'docs/VALIDATION.md', 'docs/PUBLIC_SOURCE.md']:
    install(source, datadir + '/doc/waypinyin')
install('LICENSE', datadir + '/licenses/waypinyin')
install('data/NOTICE', datadir + '/licenses/waypinyin', 'DATA-NOTICE')
install('data/sources.json', datadir + '/licenses/waypinyin')
install('build/dependency-licenses.json', datadir + '/licenses/waypinyin')
for source in sorted((root / 'build/dependency-notices').rglob('*')):
    if source.is_file():
        relative = source.relative_to(root / 'build')
        install(source.relative_to(root), datadir + '/licenses/waypinyin/' + str(relative.parent))
for source in sorted((root / 'LICENSES').glob('*.txt')):
    install(source.relative_to(root), datadir + '/licenses/waypinyin')
