"""Kernel ring-buffer rollover must not erase the new-Xid safety condition."""
import json
import unittest
from unittest.mock import patch
from vfio_session import xid_events_after


class XidCursorTests(unittest.TestCase):
    def test_empty_interval_has_no_new_xid(self):
        with patch('vfio_session.subprocess.check_output', return_value='') as query:
            self.assertEqual(xid_events_after('s=before-gpu-detach'), [])
        args=query.call_args.args[0]
        self.assertIn('--after-cursor',args)
        self.assertIn('s=before-gpu-detach',args)

    def test_new_xid_survives_empty_dmesg_history(self):
        text=json.dumps({'MESSAGE':'NVRM: Xid (PCI:0000:01:00): 31, new fault'})+'\n'
        with patch('vfio_session.subprocess.check_output',return_value=text):
            self.assertEqual(len(xid_events_after('s=before')),1)

    def test_multiple_new_xids_are_retained(self):
        text='\n'.join(json.dumps({'MESSAGE':f'NVRM: Xid fault {i}'}) for i in [1,2])
        with patch('vfio_session.subprocess.check_output',return_value=text):
            self.assertEqual(len(xid_events_after('s=before')),2)

    def test_module_probe_warning_is_not_an_xid(self):
        text=json.dumps({'MESSAGE':'NVRM: No NVIDIA devices probed.'})
        with patch('vfio_session.subprocess.check_output',return_value=text):
            self.assertEqual(xid_events_after('s=before'),[])


if __name__=='__main__':unittest.main()
