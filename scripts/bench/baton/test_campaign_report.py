#!/usr/bin/env python3
"""GPU-free tests for campaign_report.py.  Run from this directory: python3 -I test_campaign_report.py"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import campaign_report as cr  # noqa: E402


def log(label, t0):
    return ("# baton v1\n# label=%s pid=1 k_ms=5.000 cycles=1 grid=128x256 duration_s=20\n# t0_epoch=%.6f\n"
            "sec iterations max_iter_ms\nSUMMARY iterations=1 busy_ms=1 work_cycles=1 longest_gap_ms=1 "
            "longest_gap_end_s=1 first_done_ms=1 zero_sec=0 mean_iter_ms=1 iters_per_s=1 aborted=0\n" % (label, t0))


def dump(done_times):
    return "idx,start_s,dur_ms,done_s\n" + "".join("%d,%.3f,5,%.3f\n" % (i, d - .005, d) for i, d in enumerate(done_times))


class Report(unittest.TestCase):
    def mk(self, d, name, a_done, b_done):
        for lab, dn in (("A", a_done), ("B", b_done)):
            open(os.path.join(d, "%s.%s.log" % (name, lab)), "w").write(log(lab, 1000.0))
            open(os.path.join(d, "%s.%s.csv" % (name, lab)), "w").write(dump(dn))

    def test_group_runs(self):
        with tempfile.TemporaryDirectory() as d:
            self.mk(d, "pair1", [1, 2], [1, 2])
            self.mk(d, "tri2", [1], [1])
            self.assertEqual(sorted(cr.group_runs(d)), ["pair1", "tri2"])

    def test_clean_run_is_not_a_hit(self):
        with tempfile.TemporaryDirectory() as d:
            t = [i * 0.01 for i in range(1, 2000)]
            self.mk(d, "pair1", t, t)
            r = cr.analyze_run(d, "pair1")
            self.assertFalse(r["hit"])
            self.assertEqual(r["max_starved_s"], 0.0)

    def test_starved_client_is_a_hit(self):
        with tempfile.TemporaryDirectory() as d:
            a = [i * 0.01 for i in range(1, 2000)]
            b = [i * 0.01 for i in range(1, 300)] + [i * 0.01 for i in range(900, 2000)]
            self.mk(d, "pair1", a, b)
            r = cr.analyze_run(d, "pair1")
            self.assertTrue(r["hit"])
            self.assertAlmostEqual(r["max_starved_s"], 6.01, places=1)
            self.assertEqual(r["starved_label"], "B")

    def test_hit_threshold_and_rate(self):
        runs = [{"hit": True, "max_starved_s": 6}, {"hit": False, "max_starved_s": 0}, {"hit": True, "max_starved_s": 12}]
        self.assertEqual(cr.rate(runs), (2, 3))
        self.assertEqual(cr.rate([]), (0, 0))


if __name__ == "__main__":
    unittest.main()
