#!/usr/bin/env python3
"""GPU-free tests for ab_table.py.  Run from this directory: python3 -I test_ab_table.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ab_table as ab  # noqa: E402


class Table(unittest.TestCase):
    def test_arm_of(self):
        self.assertEqual(ab.arm_of("ab1_base"), "base")
        self.assertEqual(ab.arm_of("ab2_c_readback"), "c_readback")
        self.assertEqual(ab.arm_of("x1"), "x1")

    def test_summarize(self):
        runs = [{"max_gap_s": 0.01}, {"max_gap_s": 1.5}, {"max_gap_s": 4.2}, {"max_gap_s": 9.0}]
        s = ab.summarize(runs)
        self.assertEqual(s["n"], 4)
        self.assertEqual(s["ge1"], 3)
        self.assertEqual(s["ge2"], 2)
        self.assertEqual(s["ge4"], 2)
        self.assertAlmostEqual(s["max"], 9.0)

    def test_empty(self):
        self.assertEqual(ab.summarize([])["n"], 0)

    def test_pool(self):
        d = {"base": [{"max_gap_s": 5}], "a": [{"max_gap_s": 0}], "c": [{"max_gap_s": 7}]}
        self.assertEqual(ab.summarize(ab.pool(d, ["base", "a"]))["n"], 2)


if __name__ == "__main__":
    unittest.main()
