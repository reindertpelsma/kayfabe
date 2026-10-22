"""Stopping login management is justified only by a remaining DRM holder."""
import unittest
from unittest.mock import patch
import vfio_session


class LogindScopeTests(unittest.TestCase):
    def exercise(self, snapshots):
        calls=[]
        index=[0]
        def next_snapshot(nodes):
            value=snapshots[min(index[0],len(snapshots)-1)]
            index[0]+=1
            return value
        with patch.object(vfio_session,'run',side_effect=lambda *a,**k:calls.append(a)),patch.object(vfio_session,'gpu_nodes',return_value=[]),patch.object(vfio_session,'open_holders',side_effect=next_snapshot),patch.object(vfio_session.os,'pidfd_open',side_effect=ProcessLookupError):
            vfio_session.close_desktop_users()
        return calls

    def test_no_holders_does_not_stop_logind(self):
        calls=self.exercise([{},{}])
        self.assertNotIn(('systemctl','stop','systemd-logind'),calls)

    def test_exited_monitor_does_not_stop_logind(self):
        calls=self.exercise([{101:'nvidia-smi'},{}])
        self.assertNotIn(('systemctl','stop','systemd-logind'),calls)

    def test_remaining_logind_descriptor_uses_service_cleanup(self):
        holders={1:'systemd',2135:'systemd-logind'}
        calls=self.exercise([holders,holders,{}])
        self.assertIn(('systemctl','stop','systemd-logind'),calls)
        self.assertIn(('systemctl','clean','--what=fdstore','systemd-logind'),calls)
        self.assertFalse(any(c[0]=='pkill' for c in calls))


if __name__=='__main__':unittest.main()
