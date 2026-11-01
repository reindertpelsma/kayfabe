import signal
import json
import tempfile
import subprocess
import sys
from pathlib import Path
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).parent))
import phase_xid_guard as G

class DetachedQemu(unittest.TestCase):
    def check_owned(self,pid):
        with patch.object(G.os,'pidfd_open',return_value=55) as opened, patch.object(G,'validate_phase_owner',return_value=True) as validated, patch.object(G.signal,'pidfd_send_signal') as sent, patch.object(G.os,'close') as closed:
            phase={'pid':pid,'starttime':123,'exe':'/owned/qemu','pci':'0000:00:08','expected_pci':'0000:00:08'}
            self.assertTrue(G.abort_owned_qemu(phase))
            opened.assert_called_once_with(pid,0);validated.assert_called_once_with(phase)
            sent.assert_called_once_with(55,signal.SIGKILL);closed.assert_called_once_with(55)
    def test_first_detached_owned_vm_is_stopped(self):self.check_owned(42)
    def test_next_detached_owned_vm_is_stopped(self):self.check_owned(43)
    def test_reused_pid_is_never_signalled(self):
        with patch.object(G.os,'pidfd_open',return_value=55),patch.object(G,'validate_phase_owner',side_effect=G.N.GuardRefused('reused')),patch.object(G.signal,'pidfd_send_signal') as sent,patch.object(G.os,'close'):
            self.assertFalse(G.abort_owned_qemu({'pid':42}));sent.assert_not_called()
    def test_absent_phase_signals_nobody(self):
        with patch.object(G.os,'pidfd_open') as opened:
            self.assertFalse(G.abort_owned_qemu(None));opened.assert_not_called()
    def test_exited_vm_does_not_signal_another_pid(self):
        with patch.object(G.os,'pidfd_open',side_effect=ProcessLookupError),patch.object(G.signal,'pidfd_send_signal') as sent:
            self.assertFalse(G.abort_owned_qemu({'pid':42}));sent.assert_not_called()

class DetachedWatchWiring(unittest.TestCase):
    def check_observer_error(self,error):
        with tempfile.TemporaryDirectory() as d:
            phase=Path(d)/'phase';phase.write_text(json.dumps({'arm':'on_precision','pid':42,'starttime':123,'exe':'/owned/qemu','pci':'0000:00:08','rpc_path':d+'/rpc'}))
            receipt=json.dumps({'approved_native_xid_lines':[],'pci':'0000:00:08'})
            def bounded(path):return phase.read_text() if str(path)==str(phase) else receipt
            argv=['guard','watch',d+'/receipt','100',str(phase),d+'/fatal']
            with patch.object(sys,'argv',argv),patch.object(G.os,'getpgid',return_value=100),patch.object(G.os,'getpgrp',return_value=100),patch.object(G.N,'bounded',side_effect=bounded),patch.object(G,'validate_phase_owner',return_value=True),patch.object(G.N,'kernel_snapshot',side_effect=error),patch.object(G,'abort_owned_qemu',return_value=True) as stop_vm,patch.object(G.N,'terminate_owned_tree') as stop_tree:
                self.assertEqual(G.main(),7)
                self.assertEqual(stop_vm.call_args.args[0]['pid'],42)
                stop_tree.assert_called_once_with(100)
    def test_timeout_stops_detached_vm_before_controller_abort(self):self.check_observer_error(subprocess.TimeoutExpired('journalctl',10))
    def test_failed_journal_stops_detached_vm_before_controller_abort(self):self.check_observer_error(subprocess.CalledProcessError(1,'journalctl'))
