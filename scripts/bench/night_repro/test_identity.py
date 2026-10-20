# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import importlib.util
from pathlib import Path
import unittest
s=importlib.util.spec_from_file_location('identity',Path(__file__).with_name('identity.py'))
identity=importlib.util.module_from_spec(s)
if Path(s.origin).exists(): s.loader.exec_module(identity)


class Identity(unittest.TestCase):
    def test_own_qemu(self):
        self.assertTrue(identity.is_qemu(['/bin/qemu-system-x86_64','-name','boundary-kayfabe-1'],1))
    def test_different_run_is_not_owned(self):
        self.assertFalse(identity.is_qemu(['/bin/qemu-system-x86_64','-name','boundary-kayfabe-2'],1))
    def test_reused_shell_pid_is_not_owned(self):
        self.assertFalse(identity.is_qemu(['/bin/bash','-c','boundary-kayfabe-1'],1))
    def test_argument_cannot_impersonate_executable(self):
        self.assertFalse(identity.is_qemu(['python3','qemu-system-x86_64','-name','boundary-kayfabe-1'],1))
    def test_own_collector(self):
        self.assertTrue(identity.is_collector(['nvidia-smi','-l','1'],{'N1_ROOT':'/lane'},'/lane'))
    def test_foreign_collector_same_command(self):
        self.assertFalse(identity.is_collector(['nvidia-smi','-l','1'],{'N1_ROOT':'/other'},'/lane'))
    def test_unmarked_collector_same_command(self):
        self.assertFalse(identity.is_collector(['nvidia-smi','-l','1'],{},'/lane'))


if __name__=='__main__': unittest.main()
