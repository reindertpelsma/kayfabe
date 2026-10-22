#!/usr/bin/env python3
"""Tier-3 evidence collector; never a guest-baseline admission."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "native_route_measurement", ROOT / "scripts/bench/diesweep/native_route_measurement.py")


class NativeRouteMeasurement(unittest.TestCase):
    def setUp(self):
        self.m = importlib.util.module_from_spec(SPEC)
        SPEC.loader.exec_module(self.m)
        base = ROOT / "traces/sweep_dies_20261018/lease_57209908"
        self.journal = (base / "safety_stopped/native_kernel_snapshot.log").read_text()
        self.defer = (base / "native_controls/defer-liveness.log").read_text()
        self.missing = (base / "native_controls/missing-page-fault.log").read_text()
        self.pids = {"defer": 89601, "missing": 89637}

    def measure(self, journal=None, before="", pids=None):
        return self.m.measure(before, self.journal if journal is None else journal,
                              self.defer, self.missing, self.pids if pids is None else pids)

    def test_authentic_ga102_records_are_observations_only(self):
        r = self.measure()
        self.assertEqual([x["client"] for x in r["copy_faults"]], ["HUBCLIENT_CE0"] * 4)
        self.assertFalse(r["approved_for_guest"])

    def test_other_observed_client_is_recorded_without_selecting_a_die_row(self):
        r = self.measure(self.journal.replace("HUBCLIENT_CE0", "HUBCLIENT_CE1"))
        self.assertEqual([x["client"] for x in r["copy_faults"]], ["HUBCLIENT_CE1"] * 4)
        self.assertFalse(r["approved_for_guest"])

    def test_wrong_defer_process_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(pids={"defer": 123, "missing": 89637})

    def test_wrong_missing_page_process_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(pids={"defer": 89601, "missing": 123})

    def test_duplicate_record_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(self.journal + self.m.G.xid_lines(self.journal)[0] + "\n")

    def test_missing_record_is_refused(self):
        first = self.m.G.xid_lines(self.journal)[0]
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(self.journal.replace(first, ""))

    def test_preexisting_fault_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(before=self.m.G.xid_lines(self.journal)[0])

    def test_wrong_va_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(self.journal.replace("0x60_00010000", "0x60_00020000"))

    def test_wrong_engine_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(self.journal.replace("ENGINE CE0", "ENGINE CE1", 1))

    def test_non_ce_client_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(self.journal.replace("HUBCLIENT_CE0", "HUBCLIENT_PMU", 1))

    def test_nonzero_interrupt_is_refused(self):
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure(self.journal.replace("intr 00000000", "intr 00000001", 1))

    def test_source_recovery_failure_is_refused(self):
        self.defer = self.defer.replace("DEFER_RIGCTL=PASS", "DEFER_RIGCTL=FAIL")
        with self.assertRaises(self.m.MeasurementRefused):
            self.measure()


if __name__ == "__main__":
    unittest.main()
