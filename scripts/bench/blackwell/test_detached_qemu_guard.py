import signal
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
