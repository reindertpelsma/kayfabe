"""Campaign guard: source controls, immutable history, exact post-init QEMU exception."""
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import phase_xid_guard as G
import xid_policy as P

ROOT = Path(__file__).resolve().parents[3]
OLD = ROOT / 'traces/sweep_windows_20261021/lease_57064037/native'
BW = ROOT / 'traces/blackwell_resume_cont_20261022/lease_metadata/final_kernel_xids.json'
XID = '2026-10-21T23:07:37.411873+0000 ubuntu kernel: NVRM: Xid (PCI:0000:00:08): 32, pid=96783, name=qemu-system-x86, channel 0x0000000c intr 00200000'
HIGH = XID.replace('0x0000000c', '0x0e000007')
INIT = '2026-10-21T23:07:30+00:00'


class NativeControls(unittest.TestCase):
    def setUp(self):
        self.d = (OLD / 'sweep1ce11cf4_ad106_native595_defer-liveness.log').read_text().replace('chid=5', 'chid=2').replace('chid=6', 'chid=3')
        self.m = (OLD / 'sweep1ce11cf4_ad106_native595_missing-page-fault.log').read_text().replace('chid=5', 'chid=2').replace('chid=6', 'chid=3')
        self.j = '\n'.join(json.loads(BW.read_text())['xid_lines'][:5])

    def test_blackwell_source_channel_address_type_controls_are_accepted(self):
        self.assertEqual(G.native_baseline(self.j, self.d, self.m), G.N.xid_lines(self.j))

    def test_client_route_names_are_metadata_not_a_neighbour_die_row(self):
        j = self.j.replace('HUBCLIENT_CE2', 'HUBCLIENT_CE4').replace('HUBCLIENT_CE3', 'HUBCLIENT_CE5')
        self.assertEqual(G.native_baseline(j, self.d, self.m), G.N.xid_lines(j))

    def test_native_hce_xid32_is_never_a_negative_control(self):
        j = '\n'.join(json.loads(BW.read_text())['xid_lines'][:6])
        with self.assertRaises(G.N.GuardRefused): G.native_baseline(j, self.d, self.m)

    def test_wrong_source_channel_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.native_baseline(self.j.replace('0x00000002', '0x00000009', 1), self.d, self.m)

    def test_wrong_marked_address_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.native_baseline(self.j.replace('0x60_00010000', '0x60_00020000'), self.d, self.m)

    def test_missing_positive_recovery_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.native_baseline(self.j, self.d.replace('DEFER_RIGCTL=PASS', 'DEFER_RIGCTL=FAIL'), self.m)


class PhaseGuard(unittest.TestCase):
    def test_exact_pair_after_init_on_is_accepted(self):
        self.assertEqual(G.evaluate_snapshot(XID+'\n'+HIGH, (), (), 'on_p1', 96783, INIT), (XID, HIGH))

    def test_exact_pair_after_init_off_is_accepted(self):
        self.assertEqual(G.evaluate_snapshot(XID+'\n'+HIGH, (), (), 'off_c', 96783, INIT), (XID, HIGH))

    def test_early_on_fault_is_fatal(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(XID, (), (), 'on_p1', 96783, '2026-10-21T23:07:38+00:00')

    def test_early_off_fault_is_fatal(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(XID, (), (), 'off_c', 96783, '2026-10-21T23:07:38+00:00')

    def test_unproven_initialization_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(XID, (), (), 'on_p1', 96783, None)

    def test_native_phase_cannot_use_qemu_exception(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(XID, (), (), None, None, None)

    def test_later_duplicate_is_not_a_new_two_event_budget(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(XID+'\n'+XID, (XID,), (XID,), 'on_p1', 96783, INIT)

    def test_third_event_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(XID+'\n'+HIGH+'\n'+XID, (XID,HIGH), (XID,HIGH), 'on_p1', 96783, INIT)

    def test_replaced_historical_record_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(HIGH, (XID,), (), 'on_p1', 96783, INIT)

    def test_missing_historical_record_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot('', (XID,), (), None, None, None)

    def test_no_new_events_keeps_history(self):
        self.assertEqual(G.evaluate_snapshot(XID, (XID,), (), None, None, None), (XID,))

    def test_wrong_qemu_pid_stops(self):
        with self.assertRaises(G.N.GuardRefused): G.evaluate_snapshot(XID, (), (), 'on_p1', 1, INIT)


class ParserBoundary(unittest.TestCase):
    def test_trailing_garbage_is_fatal(self):
        self.assertEqual(P.judge([XID+' unexpected'], 'on_p1', 96783, True)['verdict'], 'FATAL')

    def test_trailing_second_payload_is_fatal(self):
        self.assertEqual(P.judge([XID+'; channel 0x00000001'], 'off_c', 96783, True)['verdict'], 'FATAL')

    def test_zero_pid_cannot_be_an_owned_qemu(self):
        self.assertEqual(P.judge([XID.replace('96783','0')], 'on_p1', 0, True)['verdict'], 'FATAL')

    def test_pid_one_cannot_be_the_owned_vm_on_this_box(self):
        self.assertEqual(P.judge([XID.replace('96783','1')], 'off_c', 1, True)['verdict'], 'FATAL')


if __name__ == '__main__': unittest.main()
