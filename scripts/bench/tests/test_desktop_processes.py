# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Use owned real QEMU processes to distinguish exited children from live VMs."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import time
import unittest

MODULE = Path(__file__).resolve().parents[1] / 'diesweep/desktop/live_qemu.py'
spec=importlib.util.spec_from_file_location('s4_live',MODULE)
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)


@unittest.skipUnless(shutil.which('qemu-system-x86_64'), 'needs CPU-only QEMU')
class ProcessTests(unittest.TestCase):
    def zombie(self):
        child=subprocess.Popen(['qemu-system-x86_64','--version'],stdout=subprocess.DEVNULL)
        for _ in range(200):
            stat=Path('/proc',str(child.pid),'stat').read_text().rpartition(')')[2].split()
            if stat[0]=='Z':return child
            time.sleep(.01)
        child.wait(timeout=5);self.fail('fixture did not reach zombie state')

    def test_exited_qemu_is_not_a_live_vm(self):
        child=self.zombie()
        try:self.assertNotIn(child.pid,m.live_qemu())
        finally:child.wait()

    def test_shell_census_does_not_refuse_the_exited_child(self):
        child=self.zombie()
        try:
            p=subprocess.run(['python3',str(MODULE)],capture_output=True,text=True,check=True)
            self.assertNotIn(str(child.pid),p.stdout.split())
        finally:child.wait()

    def live(self):
        child=subprocess.Popen(['nice','-n','19','qemu-system-x86_64','-machine','q35,accel=tcg','-m','64','-nodefaults','-display','none','-S','-qmp','stdio'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        json.loads(child.stdout.readline())
        return child

    def test_live_qemu_still_blocks_the_next_phase(self):
        child=self.live()
        try:self.assertIn(child.pid,m.live_qemu())
        finally:child.terminate();child.communicate(timeout=5)

    def test_shell_census_preserves_a_live_vm(self):
        child=self.live()
        try:
            p=subprocess.run(['python3',str(MODULE)],capture_output=True,text=True,check=True)
            self.assertIn(str(child.pid),p.stdout.split())
        finally:child.terminate();child.communicate(timeout=5)

    def test_direct_shell_guard_blocks_a_live_vm(self):
        child=self.live()
        try:
            p=subprocess.run(['python3',str(MODULE),'--assert-none'],capture_output=True,text=True)
            self.assertNotEqual(p.returncode,0)
        finally:child.terminate();child.communicate(timeout=5)

    def test_direct_guard_names_the_conflicting_live_pid(self):
        child=self.live()
        try:
            p=subprocess.run(['python3',str(MODULE),'--assert-none'],capture_output=True,text=True)
            self.assertIn('live QEMU',p.stderr)
            self.assertIn(str(child.pid),p.stderr)
        finally:child.terminate();child.communicate(timeout=5)
