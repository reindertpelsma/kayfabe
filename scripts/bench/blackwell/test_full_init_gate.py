"""RPC progress alone must never authorize the post-init fault exception."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).parent))
import phase_xid_guard as G

INIT='2026-10-22T01:00:00+00:00'
XID='2026-10-22T01:00:02+0000 ubuntu kernel: NVRM: Xid (PCI:0000:00:08): 32, pid=42, name=qemu-system-x86, channel 0x0000000c intr 00200000'

class FullInitWatch(unittest.TestCase):
    def check_no_code0(self, observation):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);phase=root/'phase';rpc=root/'rpc'
            rpc.write_text(''.join(json.dumps({'ordinal':i,'observed_utc':INIT})+'\n' for i in range(1000)))
            data={'arm':'on_precision','pid':42,'starttime':123,'exe':'/owned/qemu','pci':'0000:00:08','rpc_path':str(rpc)}
            if observation is not None:data['windows_init']=observation
            phase.write_text(json.dumps(data))
            receipt=json.dumps({'approved_native_xid_lines':[],'pci':'0000:00:08'})
            def bounded(path):return phase.read_text() if str(path)==str(phase) else receipt
            argv=['guard','watch',str(root/'receipt'),'100',str(phase),str(root/'fatal')]
            with patch.object(sys,'argv',argv),patch.object(G.os,'getpgid',return_value=100),patch.object(G.os,'getpgrp',return_value=100),patch.object(G.N,'bounded',side_effect=bounded),patch.object(G,'validate_phase_owner',return_value=True),patch.object(G.N,'kernel_snapshot',side_effect=[XID,G.N.GuardRefused('test end')]),patch.object(G,'abort_owned_qemu',return_value=True),patch.object(G.N,'terminate_owned_tree'),patch.object(G.time,'sleep'):
                self.assertEqual(G.main(),7)
            accepted=root/'receipt.accepted.json'
            self.assertFalse(accepted.exists(),'fault was accepted before actual Code0 evidence')
    def test_progress_without_any_windows_code0_stops(self):self.check_no_code0(None)
    def test_progress_while_windows_reports_code43_stops(self):self.check_no_code0({'code':43,'driver':'32.0.15.8088','observed_utc':INIT})


class Code0Proof(unittest.TestCase):
    def setUp(self):
        self.phase={'arm':'on_precision','pid':42,'starttime':123,'rpc_path':'unused'}
        self.phase['windows_init']=dict(arm='on_precision',pid=42,starttime=123,code=0,driver='32.0.15.8088',observed_utc=INIT)
    def test_actual_code0_and_prefix_prove_observed_init(self):
        with patch.object(G,'init_time',return_value=INIT):self.assertEqual(G.phase_init_time(self.phase),INIT)
    def test_another_pid_cannot_supply_code0(self):
        self.phase['windows_init']['pid']=43
        with patch.object(G,'init_time',return_value=INIT):self.assertIsNone(G.phase_init_time(self.phase))
    def test_another_arm_cannot_supply_code0(self):
        self.phase['windows_init']['arm']='off_c'
        with patch.object(G,'init_time',return_value=INIT):self.assertIsNone(G.phase_init_time(self.phase))
    def test_late_code0_cannot_retroactively_excuse_an_earlier_fault(self):
        self.phase['windows_init']['observed_utc']='2026-10-22T01:00:03+00:00'
        with patch.object(G,'init_time',return_value=INIT):proof=G.phase_init_time(self.phase)
        with self.assertRaises(G.N.GuardRefused):G.evaluate_snapshot(XID,(),(),'on_precision',42,proof)
    def test_code0_without_rpc_prefix_is_insufficient(self):
        with patch.object(G,'init_time',return_value=None):self.assertIsNone(G.phase_init_time(self.phase))
    def test_missing_timezone_is_not_observation_proof(self):
        self.phase['windows_init']['observed_utc']='2026-10-22T01:00:00'
        with patch.object(G,'init_time',return_value=INIT):self.assertIsNone(G.phase_init_time(self.phase))

class ActualDiagWriter(unittest.TestCase):
    def record(self,code,driver='32.0.15.8088',tdr=0):
        with tempfile.TemporaryDirectory() as d:
            phase=Path(d)/'phase';diag=Path(d)/'diag'
            phase.write_text(json.dumps({'arm':'on_precision','pid':42,'starttime':123,'exe':'/owned/qemu','pci':'0000:00:08','windows_init':{'stale':True}}))
            diag.write_text('BW580_RESULT '+json.dumps({'code':code,'driver':driver,'tdr4101':tdr})+'\n')
            with patch.object(G,'validate_phase_owner',return_value=True):G.record_windows_init(phase,diag,'on_precision')
            return json.loads(phase.read_text())
    def test_actual_diag_code0_records_host_observation_and_vm_life(self):
        p=self.record(0);r=p['windows_init'];self.assertEqual((r['code'],r['pid'],r['starttime'],r['arm']),(0,42,123,'on_precision'));self.assertIsNotNone(G.utc(r['observed_utc']).tzinfo);self.assertEqual(len(r['diagnostic_sha256']),64)
    def test_code43_cannot_record_or_retain_an_old_code0(self):self.assertNotIn('windows_init',self.record(43))
    def test_disabled_code22_cannot_record_or_retain_an_old_code0(self):self.assertNotIn('windows_init',self.record(22))
    def test_wrong_driver_cannot_record_code0(self):self.assertNotIn('windows_init',self.record(0,'32.0.15.8199'))
    def test_observed_tdr_cannot_record_clean_init(self):self.assertNotIn('windows_init',self.record(0,tdr=1))

class PrefixTiming(unittest.TestCase):
    def test_code0_does_not_excuse_fault_before_prefix_was_observed(self):
        phase={'arm':'on_precision','pid':42,'starttime':123,'rpc_path':'unused',
               'windows_init':dict(arm='on_precision',pid=42,starttime=123,code=0,driver='32.0.15.8088',observed_utc=INIT)}
        with patch.object(G,'init_time',return_value='2026-10-22T01:00:03+00:00'):proof=G.phase_init_time(phase)
        with self.assertRaises(G.N.GuardRefused):G.evaluate_snapshot(XID,(),(),'on_precision',42,proof)

if __name__=='__main__':unittest.main()
