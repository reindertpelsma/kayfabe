#!/usr/bin/env python3
"""Owner decision 1: exact source-marked native faults, never a global whitelist."""
import importlib.util
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("native_xid_guard", ROOT / "scripts/bench/native_xid_guard.py")
G = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(G)
FIX = ROOT / "traces/sweep_windows_20261021/lease_57064037/native"


class NativeXidGuardTests(unittest.TestCase):
    def setUp(self):
        self.journal = (FIX / "native_expected_xids.log").read_text()
        self.defer = (FIX / "sweep1ce11cf4_ad106_native595_defer-liveness.log").read_text()
        self.missing = (FIX / "sweep1ce11cf4_ad106_native595_missing-page-fault.log").read_text()
        self.baseline = G.xid_lines(self.journal)

    def approve(self, journal=None, defer=None, missing=None):
        return G.approve_native(self.journal if journal is None else journal,
                                self.defer if defer is None else defer,
                                self.missing if missing is None else missing)

    def test_source_matched_five_faults_are_accepted(self):
        self.assertEqual(self.approve(), self.baseline)

    def test_channels_are_derived_from_this_runs_doorbell_records(self):
        j = self.journal.replace("channel 0x00000005", "channel 0x00000025").replace("channel 0x00000006", "channel 0x00000026")
        d = self.defer.replace("chid=5", "chid=37").replace("chid=6", "chid=38")
        m = self.missing.replace("chid=5", "chid=37").replace("chid=6", "chid=38")
        self.assertEqual(self.approve(j, d, m), G.xid_lines(j))

    def test_unmatched_xid32_in_defer_still_stops(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(self.journal.replace(": 31,", ": 32,", 1))

    def test_unmatched_xid32_in_missing_page_still_stops(self):
        a = self.journal.rsplit(": 31,", 1)
        with self.assertRaises(G.GuardRefused):
            self.approve(": 32,".join(a))

    def test_changed_defer_channel_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(self.journal.replace("channel 0x00000005", "channel 0x00000009", 1))

    def test_changed_missing_page_channel_is_fatal(self):
        a = self.journal.rsplit("channel 0x00000006", 1)
        with self.assertRaises(G.GuardRefused):
            self.approve("channel 0x00000009".join(a))

    def test_changed_defer_address_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(self.journal.replace("0x60_00010000", "0x60_00020000"))

    def test_changed_missing_page_address_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(self.journal.replace("0xa_11000000", "0xa_12000000"))

    def test_changed_fault_type_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(self.journal.replace("FAULT_PTE", "FAULT_PDE", 1))

    def test_qemu_fault_cannot_use_native_exemption(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(self.journal.replace("name=kayfabe-rm-ladd", "name=qemu-system-x86", 1))

    def test_duplicate_matching_native_fault_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(self.journal + self.baseline[0] + "\n")

    def test_missing_fault_marker_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(defer=self.defer.replace("FAULT_MARK=", "NO_MARK="))

    def test_missing_positive_recovery_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(defer=self.defer.replace("DEFER_RIGCTL=PASS", "DEFER_RIGCTL=FAIL"))

    def test_missing_kernel_fault_records_cannot_look_clean(self):
        with self.assertRaises(G.GuardRefused):
            self.approve(journal="kernel booted\nno fault records captured\n")

    def test_identical_frozen_snapshot_is_accepted(self):
        G.check_unchanged(self.journal, self.baseline)

    def test_same_count_replacement_with_xid32_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            G.check_unchanged(self.journal.replace(": 31,", ": 32,", 1), self.baseline)

    def test_same_count_replacement_with_other_address_is_fatal(self):
        with self.assertRaises(G.GuardRefused):
            G.check_unchanged(self.journal.replace("0xa_11000000", "0xa_12000000"), self.baseline)

    def test_later_matching_native_fault_is_still_new_and_fatal(self):
        with self.assertRaises(G.GuardRefused):
            G.check_unchanged(self.journal + self.baseline[0] + "\n", self.baseline)


if __name__ == "__main__":
    unittest.main()
