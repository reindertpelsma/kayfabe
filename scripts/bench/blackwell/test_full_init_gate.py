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
            with patch.object(sys,'argv',argv),patch.object(G.os,'getpgid',return_value=100),patch.object(G.os,'getpgrp',return_value=100),patch.object(G.N,'bounded',side_effect=bounded),patch.object(G,'validate_phase_owner',return_value=True),patch.object(G.N,'kernel_snapshot',side_effect=[XID,G.N.GuardRefused('test end')]),patch.object(G,'abort_owned_qemu'),patch.object(G.N,'terminate_owned_tree'),patch.object(G.time,'sleep'):
                self.assertEqual(G.main(),7)
            accepted=root/'receipt.accepted.json'
            self.assertFalse(accepted.exists(),'fault was accepted before actual Code0 evidence')
    def test_progress_without_any_windows_code0_stops(self):self.check_no_code0(None)
    def test_progress_while_windows_reports_code43_stops(self):self.check_no_code0({'code':43,'driver':'32.0.15.8088','observed_utc':INIT})

if __name__=='__main__':unittest.main()
