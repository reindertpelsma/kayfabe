# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Requirement tests: run before implementing analyze.py (EXPECTED RED)."""
import importlib.util
import pathlib
import unittest

P = pathlib.Path(__file__).with_name("analyze.py")
spec = importlib.util.spec_from_file_location("h1_analyze", P)
analyze = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analyze)
MS = 1_000_000


def event(kind="mmu_apply", start=20, end=110, seq=2, tid=41, stage=1):
    return dict(kind=kind, ts_ns=start * MS, end_ns=end * MS,
                frame_seq=seq, tid=tid, stage=stage)


def run(events=(), **kw):
    return analyze.analyze(list(events), [dict(ts_ns=0, seq=1, new=True),
                                        dict(ts_ns=120 * MS, seq=2, new=True)],
                           coverage=True, **kw)


class Attribution(unittest.TestCase):
    def test_reports_every_gap_even_without_evidence(self):
        self.assertEqual(run()["gaps"][0]["class"], "UNKNOWN")

    def test_mmu_interval_at_the_first_stage_is_s1(self):
        self.assertEqual(run([event()])["gaps"][0]["class"], "S1")

    def test_unrelated_frame_is_not_causal(self):
        self.assertEqual(run([event(seq=99)])["gaps"][0]["class"], "UNKNOWN")

    def test_unlinked_global_act_is_not_causal(self):
        e = event(kind="rm_call")
        e.pop("frame_seq")
        self.assertEqual(run([e])["gaps"][0]["class"], "UNKNOWN")

    def test_first_stage_wins_over_a_later_longer_stage(self):
        a, b = event(end=105), event("broker_wait", 25, 119, stage=5)
        self.assertEqual(run([b, a])["gaps"][0]["class"], "S1")

    def test_dropped_events_force_unknown(self):
        self.assertEqual(run([event()], lost=1)["gaps"][0]["class"], "UNKNOWN")

    def test_missing_coverage_forces_unknown(self):
        result = analyze.analyze([event()], [dict(ts_ns=0, seq=1, new=True),
                                dict(ts_ns=120 * MS, seq=2, new=True)], coverage=False)
        self.assertEqual(result["gaps"][0]["class"], "UNKNOWN")

    def test_s7_requires_the_same_thread_as_first_stage(self):
        self.assertEqual(run([event(), event("reclaim", tid=41)])["gaps"][0]["class"], "S7")

    def test_compaction_is_s7(self):
        self.assertEqual(run([event(), event("compaction")])["gaps"][0]["class"], "S7")

    def test_runqueue_stall_is_s7_per_coordinator(self):
        self.assertEqual(run([event(), event("runqueue")])["gaps"][0]["class"], "S7")

    def test_other_threads_reclaim_is_not_s7(self):
        self.assertEqual(run([event(), event("reclaim", tid=9)])["gaps"][0]["class"], "S1")

    def test_global_pressure_is_not_s7(self):
        self.assertEqual(run([event(), event("memory_pressure")])["gaps"][0]["class"], "S1")

    def test_repeated_present_does_not_end_content_gap(self):
        b = [dict(ts_ns=0, seq=1, new=True), dict(ts_ns=60*MS, seq=1, new=False),
             dict(ts_ns=120*MS, seq=2, new=True)]
        r = analyze.analyze([], b, coverage=True)
        self.assertEqual(r["gaps"][0]["duration_ns"], 120*MS)
        self.assertAlmostEqual(r["metrics"]["repeat_per_s"], 1/.12)

    def test_threshold_is_strict(self):
        self.assertEqual(run(threshold_ms=120)["gaps"], [])

    def test_summary_uses_observation_duration(self):
        r = run([event()])
        self.assertEqual(r["summary"]["S1"]["count"], 1)
        self.assertEqual(r["summary"]["S1"]["per_min"], 500)

    def test_uncertain_guest_clock_cannot_assign_s6(self):
        self.assertEqual(run([event("guest_wait")], clock_error_ns=90*MS)["gaps"][0]["class"], "UNKNOWN")

    def test_precise_guest_clock_can_assign_s6(self):
        self.assertEqual(run([event("guest_wait")], clock_error_ns=1000)["gaps"][0]["class"], "S6")

    def test_clock_conversion_uses_integer_qpc_frequency(self):
        c = dict(qpc0=10**16, host_ns0=10**18, frequency=10_000_000, error_ns=1000)
        self.assertEqual(analyze.qpc_to_host(10**16+12345, c), 10**18+1234500)

    def test_invalid_qpc_frequency_is_refused(self):
        with self.assertRaises(ValueError):
            analyze.qpc_to_host(1, dict(qpc0=0, host_ns0=0, frequency=0, error_ns=0))

    def test_regressing_broker_clock_is_refused(self):
        with self.assertRaises(ValueError):
            analyze.analyze([], [dict(ts_ns=2, seq=1, new=True), dict(ts_ns=1, seq=2, new=True)])


if __name__ == "__main__":
    unittest.main()
