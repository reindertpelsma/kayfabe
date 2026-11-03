#!/usr/bin/env python3
"""The real sweep waits behind timeout groups; abort must reach owned descendants."""
import json
import importlib.util
from types import SimpleNamespace
from unittest import mock
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[3]
GUARD = ROOT / 'scripts/bench/native_xid_guard.py'
FIX = ROOT / 'traces/sweep_windows_20261021/lease_57064037/native/native_expected_xids.log'

CONTROLLER = r'''
import importlib.util,json,os,signal,subprocess,sys,time
from pathlib import Path
folder=Path(sys.argv[1]);mode=sys.argv[2];guard=sys.argv[3]
signal.signal(signal.SIGTERM,lambda *args:None)
leaf="from pathlib import Path;import os,sys,time;Path(sys.argv[1]).write_text(str(os.getpid()));time.sleep(60)"
args=[sys.executable,'-c',leaf,str(folder/'leaf.pid')]
if mode=='timeout':args=['timeout','60',*args]
q=subprocess.Popen(args,start_new_session=(mode=='setsid'))
(folder/'wrapper.pid').write_text(str(q.pid))
while not (folder/'leaf.pid').exists():time.sleep(.01)
watch=r"""import importlib.util,sys,os,json
from pathlib import Path
s=importlib.util.spec_from_file_location('g',sys.argv[1]);g=importlib.util.module_from_spec(s);s.loader.exec_module(g)
g.kernel_snapshot=lambda:Path(sys.argv[3]).read_text()
sys.argv=[sys.argv[0],'watch',sys.argv[2],sys.argv[4],sys.argv[5]]
sys.exit(g.main())
"""
w=subprocess.Popen([sys.executable,'-B','-c',watch,guard,str(folder/'baseline.json'),str(folder/'changed.log'),str(os.getpid()),str(folder/'fatal.json')])
(folder/'watcher.pid').write_text(str(w.pid))
q.wait()
'''


def live(pid):
    try:
        s=Path('/proc',str(pid),'stat').read_text().rpartition(')')[2].split()
        return s[0]!='Z'
    except FileNotFoundError:
        return False


class AbortTreeTests(unittest.TestCase):
    def run_case(self, mode):
        with tempfile.TemporaryDirectory(prefix='sweep-xid-abort-') as folder:
            d=Path(folder);journal=FIX.read_text()
            (d/'baseline.json').write_text(json.dumps({'approved_native_xid_lines':journal.splitlines()}))
            (d/'changed.log').write_text(journal.replace(': 31,',': 32,',1))
            root=subprocess.Popen([sys.executable,'-B','-c',CONTROLLER,folder,mode,str(GUARD)],start_new_session=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            try:
                deadline=time.monotonic()+5
                while not (d/'fatal.json').exists() and time.monotonic()<deadline:time.sleep(.01)
                self.assertTrue((d/'fatal.json').exists(),'watcher failed to observe the new Xid')
                pids=[int((d/n).read_text()) for n in ['wrapper.pid','leaf.pid']]
                deadline=time.monotonic()+2
                while any(live(p) for p in [root.pid,*pids]) and time.monotonic()<deadline:time.sleep(.01)
                self.assertFalse(any(live(p) for p in [root.pid,*pids]),f'owned {mode} descendants survived abort: {[p for p in [root.pid,*pids] if live(p)]}')
            finally:
                for name in ['wrapper.pid','leaf.pid','watcher.pid']:
                    p=d/name
                    if p.exists():
                        try:os.kill(int(p.read_text()),signal.SIGKILL)
                        except ProcessLookupError:pass
                try:os.killpg(root.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                root.wait(timeout=5)

    def test_new_xid_terminates_nested_timeout_group(self):
        self.run_case('timeout')

    def test_new_xid_terminates_owned_setsid_descendant(self):
        self.run_case('setsid')


class AbortIdentityTests(unittest.TestCase):
    def run_identity_case(self, reuse):
        spec=importlib.util.spec_from_file_location('abort_guard_identity',GUARD)
        guard=importlib.util.module_from_spec(spec);spec.loader.exec_module(guard)
        identities={800:(1,100),801:(800,200)}
        signals=[]
        def kill(pid,sig):
            signals.append((pid,sig))
            if reuse is True and pid==800 and sig==signal.SIGTERM:
                identities[801]=(999,201)
            elif reuse=='reparent_before' and pid==800 and sig==signal.SIGTERM:
                identities[801]=(1,200)
            elif reuse is False and pid==801 and sig==signal.SIGTERM:
                identities[801]=(1,200)
        directory=SimpleNamespace(iterdir=lambda:iter([SimpleNamespace(name='800'),SimpleNamespace(name='801')]))
        with mock.patch.object(guard,'Path',return_value=directory), mock.patch.object(guard,'process_identity',side_effect=lambda pid:identities.get(pid)), mock.patch.object(guard.os,'getpid',return_value=900), mock.patch.object(guard.os,'getpgrp',return_value=800), mock.patch.object(guard.os,'getpgid',return_value=800), mock.patch.object(guard.os,'kill',side_effect=kill), mock.patch.object(guard.os,'killpg'):
            guard.terminate_owned_tree(800)
        return signals

    def test_reparented_owned_child_is_resumed_for_queued_term(self):
        self.assertIn((801,signal.SIGCONT),self.run_identity_case(False))

    def test_reparented_owned_child_still_receives_term(self):
        signals=self.run_identity_case('reparent_before')
        self.assertIn((801,signal.SIGTERM),signals)
        self.assertIn((801,signal.SIGCONT),signals)

    def test_reused_pid_does_not_inherit_abort_ownership(self):
        signals=self.run_identity_case(True)
        self.assertNotIn((801,signal.SIGTERM),signals)
        self.assertNotIn((801,signal.SIGCONT),signals)


if __name__=='__main__':unittest.main()
