"""Trial installation and rollback preserve non-installation user state."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import importlib.util

class TrialInstallTests(unittest.TestCase):
    def test_delayed_ibus_selection_recovers_after_failed_launch(self):
        script=Path(__file__).resolve().parent.parent/'scripts/trial_install.py'
        spec=importlib.util.spec_from_file_location('trial_install',script)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        with patch.object(module.subprocess,'run',return_value=subprocess.CompletedProcess([],1)) as run, \
             patch.object(module.subprocess,'check_output',side_effect=['xkb:us::eng\n','waypinyin\n']), \
             patch.object(module.time,'sleep'):
            self.assertEqual(module.select_engine('waypinyin'),1)
            self.assertEqual(run.call_count,2)

    def test_backup_restore_and_generation_check(self):
        script=Path(__file__).resolve().parent.parent/'scripts/trial_install.py'
        with tempfile.TemporaryDirectory(prefix='waypinyin-trial-test-') as temporary:
            root=Path(temporary);prefix=root/'prefix';stage=root/'stage';backup=root/'backup'
            binary=prefix/'bin/waypinyin';binary.parent.mkdir(parents=True)
            binary.write_bytes(b'original executable');binary.chmod(0o755)
            user=prefix/'share/waypinyin/user.txt';user.parent.mkdir(parents=True)
            user.write_bytes(b'personal state remains unchanged')
            staged_binary=stage/binary.relative_to('/');staged_binary.parent.mkdir(parents=True)
            staged_binary.write_bytes(b'new executable');staged_binary.chmod(0o755)
            model=prefix/'share/waypinyin/lexicon.lm'
            staged_model=stage/model.relative_to('/');staged_model.parent.mkdir(parents=True)
            staged_model.write_bytes(b'new optional model')
            result=subprocess.run([sys.executable,str(script),'--stage',str(stage),'--prefix',str(prefix),'--backup',str(backup),'--no-reload'],capture_output=True,text=True,check=True)
            self.assertEqual(json.loads(result.stdout)['installed_files'],2)
            self.assertEqual(binary.read_bytes(),b'new executable')
            self.assertEqual(model.read_bytes(),b'new optional model')
            self.assertEqual(user.read_bytes(),b'personal state remains unchanged')
            rollback=[sys.executable,str(backup/'trial_install.py'),'--rollback',str(backup),'--no-reload']
            model.write_bytes(b'later independent update')
            rejected=subprocess.run(rollback,capture_output=True,text=True)
            self.assertNotEqual(rejected.returncode,0)
            self.assertEqual(binary.read_bytes(),b'new executable','rollback changed files before validating the generation')
            model.write_bytes(b'new optional model')
            subprocess.run(rollback,capture_output=True,text=True,check=True)
            self.assertEqual(binary.read_bytes(),b'original executable')
            self.assertEqual(binary.stat().st_mode&0o777,0o755)
            self.assertFalse(model.exists())
            self.assertEqual(user.read_bytes(),b'personal state remains unchanged')

if __name__=='__main__':unittest.main()
