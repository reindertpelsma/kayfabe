# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Coordinator additions 2/3: uptime exposure and measured per-gap context."""
import copy
import unittest

from test_analyze import analyze, MS


def analyze_gap(events=(), **kwargs):
    broker = [dict(ts_ns=0, seq=1, new=True), dict(ts_ns=120*MS, seq=2, new=True)]
    return analyze.analyze(list(events), broker, **kwargs)


def inval(kind, at, seq=7, generation=1, **fields):
    return dict(kind=kind, ts_ns=at*MS, inval_seq=seq, generation=generation, **fields)


class Uptime(unittest.TestCase):
    def test_missing_vm_start_does_not_invent_uptime(self):
        r = analyze_gap([dict(kind="marker", ts_ns=10*MS)])
        self.assertIsNone(r["gaps"][0]["vm_uptime_ns"])
        self.assertIsNone(r["gaps"][0]["events"][0]["vm_uptime_ns"])
        self.assertEqual(r["uptime_bins"], [])

    def test_zero_anchor_is_valid_and_events_have_uptime(self):
        r = analyze_gap([dict(kind="marker", ts_ns=10*MS)], vm_start_ns=0)
        self.assertEqual(r["gaps"][0]["events"][0]["vm_uptime_ns"], 10*MS)
        self.assertEqual(r["gaps"][0]["end_uptime_ns"], 120*MS)

    def test_bin_rates_use_exposure_not_assumed_ten_minutes(self):
        r = analyze_gap(vm_start_ns=0)
        row = r["uptime_bins"][0]
        self.assertEqual(row["observed_ns"], 120*MS)
        self.assertEqual(row["classes"]["UNKNOWN"]["count"], 1)
        self.assertEqual(row["classes"]["UNKNOWN"]["per_min"], 500)

    def test_cross_bin_gap_belongs_to_preceding_content_bin(self):
        b = [dict(ts_ns=599900*MS, seq=1, new=True), dict(ts_ns=600120*MS, seq=2, new=True)]
        r = analyze.analyze([], b, vm_start_ns=0)
        self.assertEqual([x["observed_ns"] for x in r["uptime_bins"]], [100*MS, 120*MS])
        self.assertEqual(r["uptime_bins"][0]["classes"]["UNKNOWN"]["count"], 1)
        self.assertEqual(r["uptime_bins"][1]["classes"]["UNKNOWN"]["count"], 0)

    def test_three_hour_observation_retains_absolute_uptime(self):
        start = 10_800_000*MS
        b = [dict(ts_ns=start, seq=1, new=True), dict(ts_ns=start+120*MS, seq=2, new=True)]
        r = analyze.analyze([], b, vm_start_ns=0)
        self.assertEqual(r["uptime_bins"][0]["start_uptime_ns"], start)
        self.assertEqual(r["gaps"][0]["vm_uptime_ns"], start)

    def test_nonzero_anchor_uses_same_monotonic_domain(self):
        anchor = 10**18
        b = [dict(ts_ns=anchor+5*MS, seq=1, new=True), dict(ts_ns=anchor+125*MS, seq=2, new=True)]
        r = analyze.analyze([], b, vm_start_ns=anchor)
        self.assertEqual(r["gaps"][0]["vm_uptime_ns"], 5*MS)

    def test_invalid_zero_bin_size_is_refused(self):
        with self.assertRaises(ValueError):
            analyze_gap(vm_start_ns=0, uptime_bin_minutes=0)

    def test_negative_bin_size_is_refused(self):
        with self.assertRaises(ValueError):
            analyze_gap(vm_start_ns=0, uptime_bin_minutes=-1)

    def test_events_are_not_mutated_by_uptime_annotation(self):
        events = [dict(kind="marker", ts_ns=10*MS)]
        original = copy.deepcopy(events)
        analyze_gap(events, vm_start_ns=0)
        self.assertEqual(events, original)


class StallContext(unittest.TestCase):
    def test_arrival_clear_and_apply_durations_are_per_request(self):
        events = [inval("mmu_arrive", 10), inval("mmu_apply", 20, end_ns=30*MS),
                  inval("mmu_apply", 40, end_ns=55*MS), inval("mmu_clear", 100)]
        row = analyze_gap(events)["gaps"][0]["context"]["invalidates"]["requests"][0]
        self.assertEqual(row["arrive_to_clear_ns"], 90*MS)
        self.assertEqual(row["apply_ns"], 25*MS)

    def test_pending_invalidate_has_unknown_not_zero_duration(self):
        c = analyze_gap([inval("mmu_arrive", 10)])["gaps"][0]["context"]["invalidates"]
        self.assertEqual(c["observed_count"], 1)
        self.assertIsNone(c["requests"][0]["arrive_to_clear_ns"])
        self.assertEqual(c["requests"][0]["state"], "pending")
        self.assertIsNone(c["requests"][0]["apply_ns"])

    def test_same_sequence_in_two_generations_is_not_joined(self):
        events = [inval("mmu_arrive", 5, generation=1), inval("mmu_clear", 10, generation=1),
                  inval("mmu_arrive", 20, generation=2), inval("mmu_clear", 30, generation=2)]
        rows = analyze_gap(events)["gaps"][0]["context"]["invalidates"]["requests"]
        self.assertEqual([r["arrive_to_clear_ns"] for r in rows], [5*MS, 10*MS])

    def test_same_sequence_in_two_devices_is_not_joined(self):
        events = [inval("mmu_arrive", 5, device="A"), inval("mmu_clear", 10, device="A"),
                  inval("mmu_arrive", 20, device="B"), inval("mmu_clear", 30, device="B")]
        rows = analyze_gap(events)["gaps"][0]["context"]["invalidates"]["requests"]
        self.assertEqual([r["arrive_to_clear_ns"] for r in rows], [5*MS, 10*MS])

    def test_completed_request_before_gap_is_excluded(self):
        events = [inval("mmu_arrive", -20), inval("mmu_clear", -10)]
        self.assertEqual(analyze_gap(events)["gaps"][0]["context"]["invalidates"]["observed_count"], 0)

    def test_context_is_observed_only_without_complete_mmu_coverage(self):
        c = analyze_gap([inval("mmu_arrive", 10)])["gaps"][0]["context"]["invalidates"]
        self.assertFalse(c["complete"])
        self.assertIsNone(c["exact_arrivals_in_gap"])

    def test_mmu_loss_invalidates_exact_count_even_with_coverage_assertion(self):
        r = analyze_gap([inval("mmu_arrive", 10)], context_coverage={"mmu": True}, lost=1)
        self.assertIsNone(r["gaps"][0]["context"]["invalidates"]["exact_arrivals_in_gap"])

    def test_completion_rates_use_exact_gap_duration(self):
        events = [dict(kind="completion", ts_ns=10*MS, engine="CE2", count=3),
                  dict(kind="completion", ts_ns=20*MS, engine="GR0", count=6)]
        c = analyze_gap(events, context_coverage={"completions": True})["gaps"][0]["context"]["completions"]
        self.assertEqual(c["CE2"]["per_s"], 25)
        self.assertEqual(c["GR0"]["per_s"], 50)
        self.assertEqual(c["CE3"]["per_s"], 0)

    def test_missing_completion_coverage_does_not_report_zero_rate(self):
        c = analyze_gap()["gaps"][0]["context"]["completions"]
        self.assertIsNone(c["CE2"]["per_s"])
        self.assertIsNone(c["GR0"]["per_s"])

    def test_completion_loss_does_not_report_exact_rate(self):
        c = analyze_gap(context_coverage={"completions": True}, lost=1)["gaps"][0]["context"]["completions"]
        self.assertIsNone(c["CE3"]["per_s"])

    def test_vram_samples_bracket_gap_without_interpolation(self):
        events = [dict(kind="guest_vram", ts_ns=t*MS, dedicated_used_bytes=n)
                  for t, n in [(-20, 10), (-10, 20), (20, 30), (130, 40), (140, 50)]]
        samples = analyze_gap(events)["gaps"][0]["context"]["guest_vram"]
        self.assertEqual([s["dedicated_used_bytes"] for s in samples], [20, 30, 40])
        self.assertEqual([s["position"] for s in samples], ["before", "inside", "after"])

    def test_post_exit_vram_sample_stays_marked_after_gap(self):
        samples = analyze_gap([dict(kind="guest_vram", ts_ns=900*MS, app_running=False,
                                    dedicated_used_bytes=1969*1024**2)])["gaps"][0]["context"]["guest_vram"]
        self.assertEqual(samples[0]["position"], "after")
        self.assertFalse(samples[0]["app_running"])

    def test_guest_present_count_requires_etw_coverage(self):
        c = analyze_gap()["gaps"][0]["context"]
        self.assertIsNone(c["guest_present_count"])
        c = analyze_gap(context_coverage={"guest": True})["gaps"][0]["context"]
        self.assertEqual(c["guest_present_count"], 0)

    def test_guest_presents_remain_observations_not_counter_inferences(self):
        e = dict(kind="guest_present", ts_ns=20*MS, pid=123)
        c = analyze_gap([e], context_coverage={"guest": True})["gaps"][0]["context"]
        self.assertEqual(c["guest_present_count"], 1)

    def test_static_desktop_counters_do_not_manufacture_stalls(self):
        e = dict(kind="display_counter", ts_ns=10*MS, kms=0, copies=0)
        self.assertEqual(analyze.analyze([e], [], vm_start_ns=0)["gaps"], [])


if __name__ == "__main__":
    unittest.main()
