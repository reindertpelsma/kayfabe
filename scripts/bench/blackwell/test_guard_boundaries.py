"""Failing-first proof of runtime ownership and fail-closed observer boundaries."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import phase_xid_guard as G
import xid_policy as P
from test_phase_xid_guard import NativeControls, XID


class ParseBoundaries(unittest.TestCase):
    def test_hidden_first_marker_stops_on(self):
        self.assertEqual(P.judge(['NVRM: Xid unexplained '+XID], 'on_precision', 96783, True)['verdict'], 'FATAL')

    def test_hidden_first_marker_stops_off(self):
        self.assertEqual(P.judge(['NVRM: Xid 31 unknown '+XID], 'off_c', 96783, True)['verdict'], 'FATAL')

    def test_non_qemu_name_is_not_our_vm(self):
        self.assertEqual(P.judge([XID.replace('name=qemu-system-x86', 'name=kf-gr-tier')], 'on_precision', 96783, True)['verdict'], 'FATAL')

    def test_other_process_name_is_not_our_vm(self):
        self.assertEqual(P.judge([XID.replace('name=qemu-system-x86', 'name=raw-client')], 'off_c', 96783, True)['verdict'], 'FATAL')


class NativeOwnership(unittest.TestCase):
    def setUp(self):
        f=NativeControls(); f.setUp()
        self.d, self.m, self.j=f.d, f.m, f.j
        self.proofs=[{'kind':'defer-liveness','pid':90288},{'kind':'missing-page-fault','pid':90327}]

    def test_actual_source_pids_and_device_are_required(self):
        self.assertEqual(G.verify_native_run(self.j,self.d,self.m,self.proofs,'0000:00:08'), G.N.xid_lines(self.j))

    def test_wrong_defer_pid_stops(self):
        self.proofs[0]['pid']=90289
        with self.assertRaises(G.N.GuardRefused): G.verify_native_run(self.j,self.d,self.m,self.proofs,'0000:00:08')

    def test_wrong_missing_page_pid_stops(self):
        self.proofs[1]['pid']=90328
        with self.assertRaises(G.N.GuardRefused): G.verify_native_run(self.j,self.d,self.m,self.proofs,'0000:00:08')

    def test_other_pci_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.verify_native_run(self.j,self.d,self.m,self.proofs,'0000:00:09')

    def test_defer_cannot_be_reported_as_graphics_write_route(self):
        with self.assertRaises(G.N.GuardRefused): G.verify_native_run(self.j.replace('ENGINE CE0','ENGINE GR_HOST0'),self.d,self.m,self.proofs,'0000:00:08')

    def test_missing_page_cannot_be_reported_as_copy_read_route(self):
        with self.assertRaises(G.N.GuardRefused): G.verify_native_run(self.j.replace('ENGINE GR_HOST0','ENGINE CE0'),self.d,self.m,self.proofs,'0000:00:08')

    def test_extra_native_marker_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.verify_native_run('NVRM: Xid unexplained '+self.j,self.d,self.m,self.proofs,'0000:00:08')


class OwnedPhase(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name); proc=self.root/'42'; proc.mkdir()
        binary=self.root/'qemu-system-x86_64'; binary.write_text('owned test fixture')
        (proc/'exe').symlink_to(binary)
        fields=['0']*22; fields[0]='S'; fields[1]='10'; fields[19]='123'
        (proc/'stat').write_text('42 (qemu-system-x86) '+' '.join(fields))
        self.phase={'pid':42,'starttime':123,'exe':str(binary),'pci':'0000:00:08','expected_pci':'0000:00:08'}

    def test_matching_owned_process_is_accepted(self):
        self.assertTrue(G.validate_phase_owner(self.phase,self.root))

    def test_reused_pid_stops(self):
        self.phase['starttime']=124
        with self.assertRaises(G.N.GuardRefused): G.validate_phase_owner(self.phase,self.root)

    def test_wrong_executable_stops(self):
        self.phase['exe']=str(self.root/'other-qemu')
        with self.assertRaises(G.N.GuardRefused): G.validate_phase_owner(self.phase,self.root)

    def test_other_gpu_stops(self):
        self.phase['pci']='0000:00:09'
        with self.assertRaises(G.N.GuardRefused): G.validate_phase_owner(self.phase,self.root)

    def test_missing_process_stops(self):
        self.phase['pid']=43
        with self.assertRaises(G.N.GuardRefused): G.validate_phase_owner(self.phase,self.root)

    def test_pid_one_stops(self):
        self.phase['pid']=1
        with self.assertRaises(G.N.GuardRefused): G.validate_phase_owner(self.phase,self.root)


class ObserverErrors(unittest.TestCase):
    def check_error(self,error):
        with tempfile.TemporaryDirectory() as d:
            argv=['guard','watch',d+'/receipt','100',d+'/phase',d+'/fatal']
            with patch.object(sys,'argv',argv), patch.object(G.os,'getpgid',return_value=100), patch.object(G.os,'getpgrp',return_value=100), patch.object(G.N,'bounded',return_value=json.dumps({'approved_native_xid_lines':[]})), patch.object(G.N,'kernel_snapshot',side_effect=error), patch.object(G.N,'terminate_owned_tree') as stop:
                self.assertEqual(G.main(),7)
                stop.assert_called_once_with(100)
                self.assertIn('STOP',json.loads(Path(d+'/fatal').read_text()))

    def test_timeout_aborts_owned_tree(self):
        self.check_error(subprocess.TimeoutExpired('journalctl',10))

    def test_failed_subprocess_aborts_owned_tree(self):
        self.check_error(subprocess.CalledProcessError(1,'journalctl'))


if __name__=='__main__': unittest.main()
