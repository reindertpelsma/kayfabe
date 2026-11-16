#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of kf3_census.py and apps_b_compare.py (2026-11-16)."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench/windows"))
import apps_b_compare as cmp  # noqa: E402
import kf3_census as kc  # noqa: E402

LOG = """kf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: 0 }
kf3: GSP rpc UNSERVICED Fn(0x1f) uuid 12345678-1234-1234-1234-123456789abc
kf3: refused write 0xdeadbeef
kf3: GR channel 0x5 ForeignClass subch 5 class 0xa140
channel 0x7 DEAD
unrelated line
NVRM: Xid (PCI:0000:00:08): 31 mac 52:54:00:12:34:56
"""


class T(unittest.TestCase):
    def test_census(self):
        c = kc.census(LOG.splitlines())
        self.assertEqual(c["lines"], 7)
        self.assertEqual(c["counts"]["unserviced"], 2)
        self.assertIn("code: 76", c["ids"]["unserviced"])
        self.assertEqual(c["counts"]["foreign_class"], 1)
        self.assertEqual(c["counts"]["dead_channel"], 1)
        self.assertEqual(c["xid"], 1)
        self.assertIn("0xa140", c["ids"]["foreign_class"])
        self.assertNotIn("12345678", str(c))
        self.assertNotIn("52:54", str(c))

    def test_window(self):
        with tempfile.TemporaryDirectory() as td:
            p = Path(td) / "q.log"
            p.write_text(LOG)
            self.assertEqual(kc.count_lines(p), 7)
            self.assertEqual(len(kc.window(p, 2, 3)), 2)
            self.assertEqual(len(kc.window(p, 6, None)), 2)

    def test_classes(self):
        kf3 = {"a": {"app": "a", "verdict": "PASS"}, "b": {"app": "b", "verdict": "FAIL", "census": {"counts": {"xid": 2}}},
               "c": {"app": "c", "verdict": "PASS_LIMITED"}, "d": {"app": "d", "verdict": "FAIL"}, "e": {"app": "e", "verdict": "PASS"},
               "f": {"app": "f", "verdict": "FAIL"}}
        vfio = {"a": {"app": "a", "verdict": "PASS"}, "b": {"app": "b", "verdict": "PASS"}, "c": {"app": "c", "verdict": "FAIL"},
                "d": {"app": "d", "verdict": "FAIL"}, "e": {"app": "e", "verdict": "PASS_LIMITED"}}
        got = {r["app"]: r["class"] for r in cmp.classify(kf3, vfio)}
        self.assertEqual(got, {"a": "PASS/PASS", "b": "KAYFABE_DEFECT", "c": "KF3_ONLY_PASS", "d": "FAIL/FAIL", "e": "PASS/PASS", "f": "NO_CONTROL_FAIL"})
        rows = cmp.classify(kf3, vfio)
        self.assertEqual(rows[0]["class"], "KAYFABE_DEFECT")
        self.assertIn("NO_CONTROL_PASS", {r["class"] for r in cmp.classify(kf3, None)})
        self.assertIn("xid=2", cmp.markdown(rows))


if __name__ == "__main__":
    unittest.main()
