"""Installation must preserve executable and mmap inodes held by live processes."""
import json
import mmap
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class InstallTests(unittest.TestCase):
    def test_update_preserves_running_executable_and_mapped_dictionary(self):
        root = Path(__file__).resolve().parent.parent
        with tempfile.TemporaryDirectory(prefix='waypinyin-install-') as temporary:
            work = Path(temporary)
            source, stage = work / 'source', work / 'stage'
            for directory in ('share', 'docs', 'LICENSES'):
                shutil.copytree(root / directory, source / directory)
            for name in ('LICENSE', 'README.md', 'README.en.md', 'CHANGELOG.md',
                         'data/NOTICE', 'data/sources.json', 'scripts/install.py'):
                target = source / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(root / name, target)
            build = source / 'build'
            (build / 'dependency-notices').mkdir(parents=True)
            (build / 'dependency-licenses.json').write_text(json.dumps({'packages': []}))
            (build / 'paths.json').write_text(json.dumps({'prefix': '/usr', 'datadir': '/usr/share'}))
            shutil.copyfile(shutil.which('sleep'), build / 'waypinyin')
            original = b'old mapped dictionary\n' * 4096
            replacement = b'new dictionary\n' * 8192
            (build / 'lexicon.bin').write_bytes(original)
            env = dict(os.environ, PREFIX='/usr', BINDIR='/usr/bin',
                       DATADIR='/usr/share', MANDIR='/usr/share/man', DESTDIR=str(stage),
                       CARGO='false')

            def install():
                subprocess.run([sys.executable, str(source / 'scripts/install.py')],
                               env=env, check=True, capture_output=True)

            install()
            binary = stage / 'usr/bin/waypinyin'
            image = stage / 'usr/share/waypinyin/lexicon.bin'
            executable_inode, image_inode = binary.stat().st_ino, image.stat().st_ino
            process = subprocess.Popen([str(binary), '30'])
            try:
                with image.open('rb') as incoming:
                    with mmap.mmap(incoming.fileno(), 0, access=mmap.ACCESS_READ) as mapped:
                        (build / 'lexicon.bin').write_bytes(replacement)
                        install()
                        self.assertIsNone(process.poll())
                        self.assertEqual(mapped[:], original)
                        self.assertEqual(image.read_bytes(), replacement)
                        self.assertNotEqual(binary.stat().st_ino, executable_inode)
                        self.assertNotEqual(image.stat().st_ino, image_inode)
                        self.assertEqual(binary.stat().st_mode & 0o777, 0o755)
                        self.assertEqual(image.stat().st_mode & 0o777, 0o644)
                        self.assertEqual(list(stage.rglob('.*')), [])
            finally:
                process.terminate()
                process.wait(timeout=5)


if __name__ == '__main__':
    unittest.main()
