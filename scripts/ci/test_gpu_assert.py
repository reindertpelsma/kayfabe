#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/bench/windows/gpu_assert.py (2026-11-16): parsers and the verdict rule, including an idle GPU, an
unreadable GPU, a memory-only rise, and an operation that raises. A fake guest stands in for the VM."""
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench/windows"))
import gpu_assert as ga  # noqa: E402


def q(gpu, mem, sess=0, fps=0):
    return "2026/11/16 00:00:00.000, %s, 0, %s, %s, %s\n" % (gpu, mem, sess, fps)


DMON = """# gpu    sm   mem   enc   dec   jpg   ofa  fb  bar1
# Idx     %     %     %     %     %     %  MB    MB
    0     0     0     0     0     0     0  500   1
    0    55    20     0     0     0     0  900   1
    0    60    22    30    12     0     0 1200   1
"""


class FakeGuest:
    def __init__(self, query, dmon):
        self.query, self.dmon, self.calls = query, dmon, []

    def ps(self, script, timeout=120):
        self.calls.append(script)
        if "Start-Process" in script:
            return SimpleNamespace(returncode=0, stdout="started\n", stderr="")
        if "gpu_query.csv'" in script and "Test-Path" in script:
            return SimpleNamespace(returncode=0, stdout=self.query, stderr="")
        if "gpu_dmon.txt'" in script and "Test-Path" in script:
            return SimpleNamespace(returncode=0, stdout=self.dmon, stderr="")
        return SimpleNamespace(returncode=0, stdout="stopped\n", stderr="")


class T(unittest.TestCase):
    def test_parsers(self):
        rows = ga.parse_query_csv(q(5, 700) + q("[N/A]", "[Not Supported]") + "garbage\n")
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["gpu"], 5.0)
        self.assertIsNone(rows[1]["gpu"])
        d = ga.parse_dmon(DMON)
        self.assertEqual(len(d), 3)
        self.assertEqual(d[2]["enc"], 30.0)

    def test_busy_passes(self):
        rows = ga.parse_query_csv(q(0, 500) + q(0, 510) + q(40, 900) + q(70, 1200))
        r = ga.evaluate(rows, ga.parse_dmon(DMON), ("gpu", "mem", "enc", "dec"))
        self.assertEqual(r["verdict"], "PASS", r)

    def test_idle_gpu_fails(self):
        rows = ga.parse_query_csv("".join(q(0, 500) for _ in range(8)))
        r = ga.evaluate(rows, [], ("gpu",))
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("idle", r["reasons"][0])

    def test_memory_rise_alone_does_not_satisfy_gpu(self):
        rows = ga.parse_query_csv(q(0, 500) + q(0, 500) + q(1, 900) + q(2, 1400))
        self.assertEqual(ga.evaluate(rows, [], ("mem",))["verdict"], "PASS")
        self.assertEqual(ga.evaluate(rows, [], ("gpu",))["verdict"], "FAIL")

    def test_unreadable_is_fail_with_reason(self):
        r = ga.evaluate([], [], ("gpu",))
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("no samples", r["reasons"][0])

    def test_encoder_needs_use(self):
        rows = ga.parse_query_csv(q(50, 500) + q(50, 500) + q(50, 600))
        self.assertEqual(ga.evaluate(rows, [], ("enc",))["verdict"], "FAIL")
        rows = ga.parse_query_csv(q(50, 500) + q(50, 500) + q(50, 600, sess=1))
        self.assertEqual(ga.evaluate(rows, [], ("enc",))["verdict"], "PASS")

    def test_unknown_engine_refused(self):
        with self.assertRaises(ValueError):
            ga.evaluate([], [], ("tensor",))

    def test_assert_gpu_used_end_to_end_and_operation_error(self):
        g = FakeGuest(q(0, 500) + q(0, 500) + q(60, 1200) + q(70, 1300), DMON)
        with tempfile.TemporaryDirectory() as td:
            res, rec = ga.assert_gpu_used(g, lambda: 42, ("gpu", "mem"), out_dir=td, name="demo", sleep=lambda s: None)
            self.assertEqual((res, rec["verdict"]), (42, "PASS"))
            self.assertTrue((Path(td) / "demo_gpu.json").is_file())

            def boom():
                raise RuntimeError("x")
            with self.assertRaises(RuntimeError):
                ga.assert_gpu_used(g, boom, ("gpu",), out_dir=td, name="bad", sleep=lambda s: None)
            self.assertIn("operation raised", (Path(td) / "bad_gpu.json").read_text())
        with self.assertRaises(ValueError):
            ga.assert_gpu_used(g, lambda: 1, name="../x")


if __name__ == "__main__":
    unittest.main()
