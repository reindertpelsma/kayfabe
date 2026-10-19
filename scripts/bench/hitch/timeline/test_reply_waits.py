# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""S8: observe request phases; assign a guest cause only with verified linkage."""
import unittest

from test_analyze import analyze, event, MS


def phase(kind, at, rpc_id=1, generation=1, device="gpu0", **kw):
    return dict(kind=kind, ts_ns=at*MS, rpc_id=rpc_id, generation=generation, device=device, **kw)


def result(events, **kw):
    return analyze.analyze(events, [dict(ts_ns=0, seq=1, new=True),
                                   dict(ts_ns=120*MS, seq=2, new=True)], **kw)


def rows(events, **kw):
    return result(events, **kw)["gaps"][0]["context"]["reply_waits"]["requests"]


def verified_wait(**kw):
    e = event("rpc_reply_wait", stage=0)
    e.update(guest_thread_wait_verified=True, guest_waiter_pid=100, guest_waiter_tid=200)
    e.update(kw)
    return e


class ReplyPhases(unittest.TestCase):
    def test_arrival_hold_rows_and_post_are_distinct_durations(self):
        e = [phase("rpc_arrive", 10, function="RmControl", guest_client="0x123", guest_object="0x456"),
             phase("reply_held", 20, hold_reason="map_refresh", row_count=9, page_count=64),
             phase("rows_applied", 80, row_count=9, page_count=64), phase("reply_posted", 110)]
        r = rows(e)[0]
        self.assertEqual(r["arrive_to_post_ns"], 100*MS)
        self.assertEqual(r["held_to_post_ns"], 90*MS)
        self.assertEqual(r["held_to_rows_ns"], 60*MS)
        self.assertEqual(r["rows_to_post_ns"], 30*MS)
        self.assertEqual((r["row_count"], r["page_count"]), (9, 64))
        self.assertEqual(r["guest_client"], "0x123")

    def test_deferred_act_hold_does_not_invent_a_rows_applied_time(self):
        e = [phase("rpc_arrive", 10), phase("reply_held", 20, hold_reason="deferred_act"),
             phase("act_resolved", 100), phase("reply_posted", 110)]
        r = rows(e)[0]
        self.assertEqual(r["hold_reason"], "deferred_act")
        self.assertIsNone(r["held_to_rows_ns"])
        self.assertEqual(r["act_resolved_ns"], 100*MS)

    def test_forwarded_alloc_without_hold_still_has_reply_latency(self):
        r = rows([phase("rpc_arrive", 10, function="RmAlloc"), phase("reply_posted", 90)])[0]
        self.assertEqual(r["arrive_to_post_ns"], 80*MS)
        self.assertIsNone(r["held_to_post_ns"])
        self.assertIsNone(r["rows_applied_ns"])

    def test_pending_reply_has_no_fabricated_post_or_completed_duration(self):
        r = rows([phase("rpc_arrive", 10), phase("reply_held", 20)])[0]
        self.assertTrue(r["right_censored"])
        self.assertIsNone(r["reply_posted_ns"])
        self.assertIsNone(r["arrive_to_post_ns"])
        self.assertIsNone(r["overlap_ns"])

    def test_complete_coverage_can_bound_an_unclosed_wait(self):
        r = rows([phase("rpc_arrive", 10), phase("reply_held", 20)],
                 context_coverage={"replies": True})[0]
        self.assertEqual(r["overlap_lower_bound_ns"], 110*MS)
        self.assertIsNone(r["arrive_to_post_ns"])

    def test_hold_time_is_not_relabelled_arrival_when_arrival_is_missing(self):
        r = rows([phase("reply_held", 20), phase("reply_posted", 110)])[0]
        self.assertIsNone(r["arrive_to_post_ns"])
        self.assertEqual(r["held_to_post_ns"], 90*MS)

    def test_failed_post_attempt_does_not_complete_held_reply(self):
        r = rows([phase("rpc_arrive", 10), phase("reply_held", 20),
                  phase("reply_post_attempt", 100, status="queue_full")])[0]
        self.assertIsNone(r["reply_posted_ns"])
        self.assertTrue(r["right_censored"])

    def test_failed_post_attempt_does_not_complete_forwarded_free(self):
        r = rows([phase("rpc_arrive", 10, function="Free"),
                  phase("reply_post_attempt", 100, status="queue_full")])[0]
        self.assertIsNone(r["arrive_to_post_ns"])
        self.assertTrue(r["right_censored"])

    def test_rpc_sequence_reuse_across_generations_is_not_joined(self):
        r = rows([phase("rpc_arrive", 10, generation=1), phase("reply_posted", 100, generation=2)])
        self.assertEqual(len(r), 2)
        self.assertTrue(all(x["arrive_to_post_ns"] is None for x in r))

    def test_rpc_sequence_reuse_across_devices_is_not_joined(self):
        r = rows([phase("rpc_arrive", 10, device="A"), phase("reply_posted", 100, device="B")])
        self.assertEqual(len(r), 2)
        self.assertTrue(all(x["arrive_to_post_ns"] is None for x in r))

    def test_missing_row_counts_are_unknown(self):
        r = rows([phase("rpc_arrive", 10), phase("reply_posted", 100)])[0]
        self.assertIsNone(r["row_count"])
        self.assertIsNone(r["page_count"])

    def test_long_boot_wait_is_preserved_but_does_not_overlap_later_gap(self):
        e = [phase("rpc_arrive", -2000, rpc_id=1), phase("reply_posted", -1000, rpc_id=1),
             phase("rpc_arrive", 20, rpc_id=2), phase("reply_posted", 50, rpc_id=2)]
        r = result(e)
        longest = r["longest_reply_waits"][0]
        self.assertEqual(longest["arrive_to_post_ns"], 1000*MS)
        self.assertEqual(longest["stall_overlaps"][0]["overlap_ns"], 0)
        self.assertEqual(rows(e)[0]["rpc_id"], 2)

    def test_longest_table_also_reports_a_positive_stall_overlap(self):
        e = [phase("rpc_arrive", 10), phase("reply_posted", 110)]
        r = result(e)["longest_reply_waits"][0]
        self.assertEqual(r["stall_overlaps"][0]["overlap_ns"], 100*MS)

    def test_host_call_duration_is_separate_from_reply_latency(self):
        e = [phase("rpc_arrive", 10), phase("rm_call", 25, end_ns=30*MS),
             phase("reply_posted", 110)]
        r = rows(e)[0]
        self.assertEqual(r["host_call_ns"], 5*MS)
        self.assertEqual(r["arrive_to_post_ns"], 100*MS)

    def test_observed_host_wait_alone_never_proves_guest_s8(self):
        r = result([phase("rpc_arrive", 10), phase("reply_posted", 110)], coverage=True)
        self.assertEqual(r["gaps"][0]["class"], "UNKNOWN")

    def test_unidentified_hold_does_not_pair_by_depth(self):
        e = [dict(kind="reply_held", ts_ns=10*MS, depth=1),
             dict(kind="reply_posted", ts_ns=100*MS, count=1)]
        c = result(e)["gaps"][0]["context"]["reply_waits"]
        self.assertEqual(c["requests"], [])
        self.assertEqual(c["unidentified_events"], 2)


class S8Attribution(unittest.TestCase):
    def test_verified_guest_reply_wait_at_first_stage_is_s8(self):
        r = result([verified_wait()], coverage=True, clock_error_ns=1000)
        self.assertEqual(r["gaps"][0]["class"], "S8")

    def test_unverified_guest_wait_is_unknown(self):
        r = result([verified_wait(guest_thread_wait_verified=False)], coverage=True, clock_error_ns=1000)
        self.assertEqual(r["gaps"][0]["class"], "UNKNOWN")

    def test_missing_guest_waiter_pid_prevents_s8(self):
        r = result([verified_wait(guest_waiter_pid=None)], coverage=True, clock_error_ns=1000)
        self.assertEqual(r["gaps"][0]["class"], "UNKNOWN")

    def test_missing_guest_waiter_tid_prevents_s8(self):
        r = result([verified_wait(guest_waiter_tid=None)], coverage=True, clock_error_ns=1000)
        self.assertEqual(r["gaps"][0]["class"], "UNKNOWN")

    def test_missing_guest_clock_prevents_s8(self):
        self.assertEqual(result([verified_wait()], coverage=True)["gaps"][0]["class"], "UNKNOWN")

    def test_uncertain_guest_clock_prevents_s8(self):
        r = result([verified_wait()], coverage=True, clock_error_ns=90*MS)
        self.assertEqual(r["gaps"][0]["class"], "UNKNOWN")

    def test_s8_first_guest_stage_keeps_nested_mmu_context(self):
        r = result([event(), verified_wait()], coverage=True, clock_error_ns=1000)
        self.assertEqual(r["gaps"][0]["class"], "S8")

    def test_unrelated_s8_does_not_replace_correlated_s1(self):
        r = result([event(), verified_wait(frame_seq=99)], coverage=True, clock_error_ns=1000)
        self.assertEqual(r["gaps"][0]["class"], "S1")

    def test_s8_appears_in_global_summary(self):
        r = result([verified_wait()], coverage=True, clock_error_ns=1000)
        self.assertEqual(r["summary"]["S8"]["count"], 1)

    def test_s8_appears_in_uptime_bins(self):
        r = result([verified_wait()], coverage=True, clock_error_ns=1000, vm_start_ns=0)
        self.assertEqual(r["uptime_bins"][0]["classes"]["S8"]["count"], 1)


if __name__ == "__main__":
    unittest.main()
