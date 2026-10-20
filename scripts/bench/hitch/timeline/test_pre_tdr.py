# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""S9 requirements, committed before implementation (EXPECTED RED)."""
import copy
import unittest

from test_analyze import analyze, event

SEC = 1_000_000_000
PID = 123
COUNTS = [60, 60, 50, 45, 31, 20, 10, 3, 0, 0]


def evidence(counts=COUNTS, *, tdr_at=20*SEC, pid=PID):
    lo = tdr_at - 10*SEC
    rows = [dict(kind="guest_tdr", ts_ns=tdr_at, pid=pid, tdr_verified=True),
            dict(kind="workload_active", ts_ns=lo, end_ns=tdr_at, pid=pid)]
    for i, count in enumerate(counts):
        rows.extend(dict(kind="guest_present", ts_ns=lo+i*SEC+SEC//2, pid=pid)
                    for _ in range(count))
    return rows


def run(rows=None, *, counts=COUNTS, lost=0, error=0, coverage=None, broker=None, **kw):
    if coverage is None:
        coverage = dict(guest=True, guest_start_ns=0, guest_end_ns=30*SEC, broker=True)
    if broker is None:
        broker = [dict(ts_ns=10*SEC, seq=1, new=True),
                  dict(ts_ns=10*SEC+16_000_000, seq=2, new=False),
                  dict(ts_ns=20*SEC, seq=3, new=True)]
    return analyze.analyze(evidence(counts) if rows is None else rows, broker,
                           clock_error_ns=error, lost=lost, context_coverage=coverage, **kw)


class PreTdr(unittest.TestCase):
    def test_reports_full_ten_second_guest_series(self):
        row = run()["pre_tdr_windows"][0]
        self.assertEqual([b["guest_present_count"] for b in row["bins"]], COUNTS)
        self.assertEqual(row["start_ns"], 10*SEC)
        self.assertEqual(row["end_ns"], 20*SEC)
        self.assertEqual(row["phenomenon"], "S9")
        self.assertEqual(row["status"], "DECLINE_OBSERVED")

    def test_decline_compares_counts_with_same_exposure(self):
        row = run()["pre_tdr_windows"][0]
        self.assertEqual(row["early"]["observed_count"], 120)
        self.assertEqual(row["late"]["observed_count"], 0)
        self.assertEqual(row["early"]["per_s"], 60)
        self.assertEqual(row["late"]["per_s"], 0)

    def test_steady_presents_falsify_decline_for_this_window(self):
        self.assertEqual(run(counts=[60]*10)["pre_tdr_windows"][0]["status"],
                         "NO_DECLINE_OBSERVED")

    def test_increasing_rate_does_not_claim_degradation(self):
        self.assertEqual(run(counts=list(reversed(COUNTS)))["pre_tdr_windows"][0]["status"],
                         "UNKNOWN")  # no positive early baseline
        self.assertEqual(run(counts=list(range(1, 11)))["pre_tdr_windows"][0]["status"],
                         "NO_DECLINE_OBSERVED")

    def test_zero_throughout_does_not_prove_a_preceding_decline(self):
        self.assertEqual(run(counts=[0]*10)["pre_tdr_windows"][0]["status"], "UNKNOWN")

    def test_broker_bins_count_actual_new_content_only(self):
        bins = run()["pre_tdr_windows"][0]["bins"]
        self.assertEqual([b["broker_new_count"] for b in bins], [1]+[0]*9)
        self.assertEqual(bins[0]["broker_new_per_s"], 1)

    def test_missing_broker_exposure_keeps_exact_rates_unknown(self):
        row = run(broker=[])["pre_tdr_windows"][0]
        self.assertIsNone(row["bins"][0]["broker_new_per_s"])
        self.assertEqual(row["status"], "DECLINE_OBSERVED")

    def test_missing_guest_coverage_keeps_zero_unknown(self):
        row = run(coverage={})["pre_tdr_windows"][0]
        self.assertEqual(row["status"], "UNKNOWN")
        self.assertIsNone(row["bins"][-1]["guest_present_per_s"])
        self.assertIsNone(row["early"]["per_s"])

    def test_loss_prevents_exact_rates_and_s9_conclusion(self):
        row = run(lost=1)["pre_tdr_windows"][0]
        self.assertEqual(row["status"], "UNKNOWN")
        self.assertIsNone(row["bins"][0]["guest_present_per_s"])
        self.assertIsNone(row["bins"][0]["broker_new_per_s"])

    def test_guest_capture_must_cover_the_entire_window(self):
        row = run(coverage=dict(guest=True, guest_start_ns=11*SEC,
                                guest_end_ns=30*SEC))["pre_tdr_windows"][0]
        self.assertEqual(row["status"], "UNKNOWN")
        self.assertIsNone(row["bins"][0]["guest_present_per_s"])

    def test_missing_capture_end_is_not_complete_exposure(self):
        row = run(coverage=dict(guest=True, guest_start_ns=0))["pre_tdr_windows"][0]
        self.assertEqual(row["status"], "UNKNOWN")

    def test_unverified_tdr_is_visible_but_cannot_conclude_s9(self):
        rows = evidence()
        rows[0]["tdr_verified"] = False
        self.assertEqual(run(rows)["pre_tdr_windows"][0]["status"], "UNKNOWN")

    def test_generic_153_event_is_not_automatically_a_tdr(self):
        r = run([dict(kind="guest_event", ts_ns=20*SEC, event_id=153, pid=PID)])
        self.assertEqual(r["pre_tdr_windows"], [])

    def test_tdr_pid_is_required_and_must_not_be_an_opaque_handle(self):
        for pid in (None, True, "123", 0):
            with self.subTest(pid=pid):
                rows = evidence()
                rows[0]["pid"] = pid
                self.assertEqual(run(rows)["pre_tdr_windows"][0]["status"], "UNKNOWN")

    def test_other_process_presents_do_not_fill_the_game_timeline(self):
        rows = evidence()
        rows.extend(dict(kind="guest_present", ts_ns=19*SEC, pid=999) for _ in range(200))
        row = run(rows)["pre_tdr_windows"][0]
        self.assertEqual(row["late"]["observed_count"], 0)
        self.assertEqual(row["status"], "DECLINE_OBSERVED")

    def test_activity_must_cover_the_entire_window(self):
        rows = evidence()
        rows[1]["end_ns"] = 19*SEC
        self.assertEqual(run(rows)["pre_tdr_windows"][0]["status"], "UNKNOWN")

    def test_other_process_activity_is_insufficient(self):
        rows = evidence()
        rows[1]["pid"] = 999
        self.assertEqual(run(rows)["pre_tdr_windows"][0]["status"], "UNKNOWN")

    def test_contiguous_active_segments_cover_but_a_hole_does_not(self):
        rows = evidence()
        rows[1]["end_ns"] = 15*SEC
        rows.append(dict(kind="workload_active", ts_ns=15*SEC, end_ns=20*SEC, pid=PID))
        self.assertEqual(run(rows)["pre_tdr_windows"][0]["status"], "DECLINE_OBSERVED")
        rows[-1]["ts_ns"] += 1
        self.assertEqual(run(rows)["pre_tdr_windows"][0]["status"], "UNKNOWN")

    def test_missing_or_uncertain_guest_clock_prevents_ordering(self):
        for error in (None, -1, 40_000_000):
            with self.subTest(error=error):
                self.assertEqual(run(error=error)["pre_tdr_windows"][0]["status"], "UNKNOWN")

    def test_boundary_uncertainty_retains_count_bounds(self):
        rows = evidence([0]*10)
        rows.append(dict(kind="guest_present", ts_ns=12*SEC-100, pid=PID))
        row = run(rows, error=1000)["pre_tdr_windows"][0]
        self.assertEqual(row["early"]["count_lower"], 0)
        self.assertEqual(row["early"]["count_upper"], 1)
        self.assertEqual(row["status"], "UNKNOWN")

    def test_boundary_ambiguity_cannot_forge_decline(self):
        rows = evidence([0]*10)
        rows.extend([dict(kind="guest_present", ts_ns=12*SEC-100, pid=PID),
                     dict(kind="guest_present", ts_ns=12*SEC-200, pid=PID),
                     dict(kind="guest_present", ts_ns=19*SEC, pid=PID)])
        row = run(rows, error=1000)["pre_tdr_windows"][0]
        self.assertNotEqual(row["status"], "DECLINE_OBSERVED")

    def test_s9_context_preserves_first_stage_causal_class(self):
        rows = evidence()
        e = event(start=10020, end=19900, seq=3)
        r = analyze.analyze(rows+[e], [dict(ts_ns=10*SEC, seq=1, new=True),
                                      dict(ts_ns=20*SEC, seq=3, new=True)],
                            coverage=True, clock_error_ns=0,
                            context_coverage=dict(guest=True, guest_start_ns=0,
                                                  guest_end_ns=30*SEC))
        self.assertEqual(r["gaps"][0]["class"], "S1")
        self.assertEqual(r["gaps"][0]["pre_tdr_window_indices"], [0])
        self.assertEqual(r["pre_tdr_windows"][0]["status"], "DECLINE_OBSERVED")

    def test_all_tdr_windows_and_overlaps_are_retained(self):
        r = run(evidence()+evidence([60]*10, tdr_at=40*SEC),
                coverage=dict(guest=True, guest_start_ns=0, guest_end_ns=50*SEC))
        self.assertEqual(len(r["pre_tdr_windows"]), 2)
        self.assertEqual(r["pre_tdr_summary"], dict(DECLINE_OBSERVED=1,
                         NO_DECLINE_OBSERVED=1, UNKNOWN=0))

    def test_tdr_windows_have_vm_uptime_without_resetting_anchor(self):
        row = run(vm_start_ns=5*SEC)["pre_tdr_windows"][0]
        self.assertEqual(row["vm_uptime_ns"], 15*SEC)
        self.assertEqual(row["start_uptime_ns"], 5*SEC)

    def test_static_counters_do_not_manufacture_tdr_or_content_gap(self):
        r = run([dict(kind="display_counter", ts_ns=20*SEC, kms=0, copies=0)], broker=[])
        self.assertEqual(r["gaps"], [])
        self.assertEqual(r["pre_tdr_windows"], [])

    def test_input_events_are_not_mutated(self):
        rows = evidence()
        saved = copy.deepcopy(rows)
        run(rows)
        self.assertEqual(rows, saved)


if __name__ == "__main__":
    unittest.main()
