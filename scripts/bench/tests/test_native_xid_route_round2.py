#!/usr/bin/env python3
"""EXPECTED RED: Round 2 route replay, pending an independent public derivation.

These expectations describe the requested endpoint. They do not license deriving
a HUB client from the journal being admitted. A future implementation must also
test the independently sourced topology proof before enabling GA102 admission.
"""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "native_xid_guard", ROOT / "scripts/bench/native_xid_guard.py")
G = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(G)


def fixtures(die):
    # MEASURED: authentic source controls and journals, leases below; no client
    # replacement in the positive replay and no synthetic positive fixture.
    if die == "GA102":
        base = ROOT / "traces/sweep_dies_20261018/lease_57209908"
        return ((base / "safety_stopped/native_kernel_snapshot.log").read_text(),
                (base / "native_controls/defer-liveness.log").read_text(),
                (base / "native_controls/missing-page-fault.log").read_text())
    base = ROOT / "traces/sweep_ada_20261018/lease_57144505/native_complete"
    return ((base / "kernel_xids.log").read_text(),
            (base / "sweepf98ceae6_ad102_native595_defer-liveness.log").read_text(),
            (base / "sweepf98ceae6_ad102_native595_missing-page-fault.log").read_text())


class Round2NativeRouteReplay(unittest.TestCase):
    def test_ad102_historical_exact_five_still_admit(self):
        journal, defer, missing = fixtures("AD102")
        self.assertEqual(len(G.xid_lines(journal)), 5)
        self.assertEqual(G.approve_native(journal, defer, missing), G.xid_lines(journal))

    def test_ga102_exact_four_ce_and_one_gr_should_admit_after_derivation(self):
        journal, defer, missing = fixtures("GA102")
        self.assertEqual(len(G.xid_lines(journal)), 5)
        self.assertEqual(journal.count("HUBCLIENT_CE0"), 4)
        self.assertEqual(G.approve_native(journal, defer, missing), G.xid_lines(journal))

    def test_ga102_substituting_ce1_must_not_admit(self):
        journal, defer, missing = fixtures("GA102")
        with self.assertRaises(G.GuardRefused):
            G.approve_native(journal.replace("HUBCLIENT_CE0", "HUBCLIENT_CE1"), defer, missing)

    def test_ad102_substituting_ce0_stays_fatal(self):
        journal, defer, missing = fixtures("AD102")
        with self.assertRaises(G.GuardRefused):
            G.approve_native(journal.replace("HUBCLIENT_CE1", "HUBCLIENT_CE0"), defer, missing)

    def test_ga102_one_extra_known_record_stays_fatal(self):
        journal, defer, missing = fixtures("GA102")
        with self.assertRaises(G.GuardRefused):
            G.approve_native(journal + G.xid_lines(journal)[0] + "\n", defer, missing)

    def test_ga102_wrong_va_stays_fatal(self):
        journal, defer, missing = fixtures("GA102")
        with self.assertRaises(G.GuardRefused):
            G.approve_native(journal.replace("0x60_00010000", "0x60_00020000"), defer, missing)

    def test_ga102_wrong_channel_stays_fatal(self):
        journal, defer, missing = fixtures("GA102")
        with self.assertRaises(G.GuardRefused):
            G.approve_native(journal.replace("channel 0x00000004", "channel 0x00000024", 1), defer, missing)

    def test_ga102_unrelated_xid_stays_fatal(self):
        journal, defer, missing = fixtures("GA102")
        with self.assertRaises(G.GuardRefused):
            G.approve_native(journal.replace(": 31,", ": 32,", 1), defer, missing)


if __name__ == "__main__":
    unittest.main()
