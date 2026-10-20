#!/usr/bin/env python3
"""GPU-free tests for baton_parse.py (log parser, stats, baton detector).

Run: python3 -m unittest scripts/bench/baton/test_baton_parse.py   (from the repo root)
 or: python3 test_baton_parse.py                                    (from this directory)
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import baton_parse as bp  # noqa: E402

SAMPLE = """\
# baton v1
# label=A pid=4242 k_ms=5.000 cycles=8000000 grid=128x256 duration_s=3
# t0_epoch=1760000000.500000
sec iterations max_iter_ms
0 190 5.312
1 0 0.000
2 5 160.250
SUMMARY iterations=195 busy_ms=2990.5 work_cycles=1560000000 longest_gap_ms=1100.000 longest_gap_end_s=2.100 first_done_ms=5.100 zero_sec=1 mean_iter_ms=15.336 iters_per_s=65.000
SLOW_CSV_BEGIN idx,start_s,dur_ms,done_s
190,1.000,1100.000,2.100
191,2.100,160.250,2.260
SLOW_CSV_END
"""


def it(idx, start, dur):
    """(idx, start_s, dur_ms, done_s) tuple helper."""
    return (idx, start, dur, start + dur / 1000.0)


class ParseLog(unittest.TestCase):
    def test_header_fields(self):
        log = bp.parse_log(SAMPLE)
        self.assertEqual(log["label"], "A")
        self.assertEqual(log["pid"], 4242)
        self.assertAlmostEqual(log["k_ms"], 5.0)
        self.assertEqual(log["cycles"], 8000000)
        self.assertEqual(log["duration_s"], 3)
        self.assertAlmostEqual(log["t0_epoch"], 1760000000.5)

    def test_per_second(self):
        log = bp.parse_log(SAMPLE)
        self.assertEqual(log["per_sec"], [(0, 190, 5.312), (1, 0, 0.0), (2, 5, 160.25)])

    def test_summary(self):
        s = bp.parse_log(SAMPLE)["summary"]
        self.assertEqual(s["iterations"], 195)
        self.assertAlmostEqual(s["longest_gap_ms"], 1100.0)
        self.assertEqual(s["zero_sec"], 1)
        self.assertEqual(s["work_cycles"], 1560000000)

    def test_slow_csv(self):
        slow = bp.parse_log(SAMPLE)["slow"]
        self.assertEqual(len(slow), 2)
        self.assertEqual(slow[0], (190, 1.0, 1100.0, 2.1))

    def test_noise_lines_ignored(self):
        noisy = "garbage\n" + SAMPLE + "\nmore garbage\n"
        self.assertEqual(bp.parse_log(noisy)["summary"]["iterations"], 195)

    def test_missing_summary_is_none(self):
        trunc = "\n".join(SAMPLE.splitlines()[:7]) + "\n"
        log = bp.parse_log(trunc)
        self.assertIsNone(log["summary"])  # a killed run has no summary
        self.assertEqual(len(log["per_sec"]), 3)

    def test_empty_input_raises(self):
        with self.assertRaises(ValueError):
            bp.parse_log("")


class DumpCsv(unittest.TestCase):
    def test_parse_dump(self):
        txt = "idx,start_s,dur_ms,done_s\n0,0.000,5.000,0.005\n1,0.005,6.000,0.011\n"
        self.assertEqual(bp.parse_dump(txt), [(0, 0.0, 5.0, 0.005), (1, 0.005, 6.0, 0.011)])

    def test_parse_dump_skips_blank_and_header(self):
        self.assertEqual(bp.parse_dump("idx,start_s,dur_ms,done_s\n\n"), [])


class Stats(unittest.TestCase):
    def test_basic(self):
        its = [it(0, 0.0, 5.0), it(1, 0.005, 5.0), it(2, 0.010, 5.0)]
        st = bp.compute_stats(its, duration_s=1)
        self.assertEqual(st["iterations"], 3)
        self.assertAlmostEqual(st["busy_ms"], 15.0)
        self.assertAlmostEqual(st["longest_gap_ms"], 5.0, places=3)
        self.assertEqual(st["zero_sec"], 0)  # bucket 0 has completions
        self.assertAlmostEqual(st["first_done_ms"], 5.0, places=3)

    def test_zero_seconds_counted_in_window_only(self):
        # completions in sec 0 and sec 3 only; window 5 s -> sec 1,2,4 are empty
        its = [it(0, 0.1, 5.0), it(1, 3.2, 5.0)]
        st = bp.compute_stats(its, duration_s=5)
        self.assertEqual(st["zero_sec"], 3)

    def test_gap_includes_leading_gap_and_reports_end(self):
        its = [it(0, 0.0, 9000.0), it(1, 9.0, 5.0)]  # starved start: first completion at 9 s
        st = bp.compute_stats(its, duration_s=10)
        self.assertAlmostEqual(st["longest_gap_ms"], 9000.0, places=2)
        self.assertAlmostEqual(st["longest_gap_end_s"], 9.0, places=3)

    def test_empty(self):
        st = bp.compute_stats([], duration_s=4)
        self.assertEqual(st["iterations"], 0)
        self.assertEqual(st["zero_sec"], 4)
        self.assertEqual(st["longest_gap_ms"], 0.0)

    def test_matches_c_summary_for_sample(self):
        # Recomputing from the slow list plus the per-second counts must agree on the gap.
        log = bp.parse_log(SAMPLE)
        slow = log["slow"]
        gap = max(d for (_, _, d, _) in slow)
        self.assertAlmostEqual(gap, log["summary"]["longest_gap_ms"])


class Batons(unittest.TestCase):
    def _run(self, a, b, **kw):
        return bp.detect_batons({"A": (0.0, a), "B": (0.0, b)}, **kw)

    def test_no_baton_when_interleaved(self):
        a = [it(i, i * 0.010, 9.0) for i in range(500)]
        b = [it(i, i * 0.010 + 0.005, 9.0) for i in range(500)]
        self.assertEqual(self._run(a, b), [])

    def test_baton_detected(self):
        # B is starved 2.0 s -> 8.0 s (6 s gap) while A keeps completing.
        a = [it(i, i * 0.010, 9.0) for i in range(1000)]  # 0..10 s
        b = [it(i, i * 0.010, 9.0) for i in range(200)] + [it(200, 2.0, 6000.0)] \
            + [it(201 + i, 8.0 + i * 0.010, 9.0) for i in range(100)]
        ev = self._run(a, b)
        self.assertEqual(len(ev), 1)
        self.assertEqual(ev[0]["starved"], "B")
        self.assertAlmostEqual(ev[0]["gap_s"], 6.0, places=1)
        self.assertGreater(ev[0]["other_completions"], 100)

    def test_gap_below_threshold_ignored(self):
        a = [it(i, i * 0.010, 9.0) for i in range(1000)]
        b = [it(0, 0.0, 3000.0)] + [it(1 + i, 3.0 + i * 0.010, 9.0) for i in range(100)]
        self.assertEqual(self._run(a, b, min_gap_s=4.0), [])

    def test_not_a_baton_if_both_stalled(self):
        # Both stop for 6 s together (e.g. a TDR / whole-GPU stall): not a baton.
        a = [it(0, 0.0, 6000.0), it(1, 6.0, 5.0)]
        b = [it(0, 0.0, 6000.0), it(1, 6.0, 5.0)]
        self.assertEqual(self._run(a, b), [])

    def test_clock_alignment_uses_t0_epoch(self):
        # B's t0 is 10 s after A's: B's local 2..8 s stall is A-time 12..18 s.
        a = [it(i, i * 0.010, 9.0) for i in range(3000)]  # 0..30 s
        b = [it(0, 1.995, 5.0), it(1, 2.0, 6000.0), it(2, 8.0, 5.0)]
        ev = bp.detect_batons({"A": (100.0, a), "B": (110.0, b)})
        self.assertEqual(len(ev), 1)
        self.assertAlmostEqual(ev[0]["from_s"], 12.0, places=2)  # relative to the earliest t0


class Share(unittest.TestCase):
    def test_share_of_work(self):
        sh = bp.share_of_work({"A": (1000, 8_000_000), "B": (100, 80_000_000)})
        self.assertAlmostEqual(sh["A"], 0.5)
        self.assertAlmostEqual(sh["B"], 0.5)

    def test_share_zero_total(self):
        sh = bp.share_of_work({"A": (0, 1), "B": (0, 1)})
        self.assertEqual(sh, {"A": 0.0, "B": 0.0})


class Table(unittest.TestCase):
    def test_table_has_rows(self):
        rows = [{"label": "A", "iterations": 10, "iters_per_s": 1.0, "share": 0.5,
                 "longest_gap_ms": 12.0, "zero_sec": 0}]
        t = bp.format_table(rows)
        self.assertIn("A", t)
        self.assertIn("longest_gap_ms", t)


if __name__ == "__main__":
    unittest.main()
