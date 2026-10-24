# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free checks of the PR #92 automated lanes' graders (run by CI's
`python3 -m unittest discover -s scripts/ci -p 'test_*.py'`).

* the Windows hue/vibrance grader's selftest (synthetic screendumps and logs: a good setting passes,
  a frozen console, an unapplied matrix and a refusal line fail, non-identity tables and a missing
  program line are INCONCLUSIVE, the four hand-computed pixels of the hardware example);
* the line format kf-qemu prints (`program_line`, pinned by its own unit test) is the one the
  grader parses: the pinned expected strings are read out of display.rs, so a change on one side
  fails here;
* the Linux cursor grader's selftest.
"""
import importlib.util
import io
import os
import re
import unittest
from contextlib import redirect_stdout

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def load(rel, name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(ROOT, rel))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class HueVibranceGrader(unittest.TestCase):
    def setUp(self):
        self.hv = load("scripts/bench/windows/hue_vibrance_check.py", "hue_vibrance_check")

    def test_selftest_passes(self):
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.hv.selftest()
        self.assertEqual(rc, 0, out.getvalue())
        self.assertIn("HV_SELFTEST=PASS", out.getvalue())

    def test_the_kf_qemu_line_is_the_parsed_line(self):
        with open(os.path.join(ROOT, "crates/kf-qemu/src/display.rs"), encoding="utf-8") as f:
            src = f.read()
        # the expected strings of `the_colour_program_line_carries_the_decoded_matrix_and_table_flags`
        body = src.split("fn the_colour_program_line_carries_the_decoded_matrix_and_table_flags()", 1)[1]
        body = body.split("\n    }\n", 1)[0]
        lines = [re.sub(r"\\\n\s*", "", s) for s in re.findall(r'"(kf3: display: colour program #.*?)"', body, re.S)]
        self.assertEqual(len(lines), 2, body)
        first = self.hv.last_program(lines[0])
        self.assertIsNotNone(first, lines[0])
        self.assertEqual(first["matrix"], [65536, -32768, 0, 32, 0, 65536, 0, 0, 0, 0, 65536, 0])
        self.assertTrue(first["mirror"] and first["signed_behind_mirror"])
        second = self.hv.PROGRAM_RE.search(lines[1])
        self.assertIsNotNone(second, lines[1])
        self.assertEqual(second.group("olut"), "bypass")


class CursorGrader(unittest.TestCase):
    def test_selftest_passes(self):
        path = os.path.join(ROOT, "scripts/bench/display/cursor_check.py")
        if not os.path.exists(path):
            self.skipTest("cursor_check.py not present")
        cc = load("scripts/bench/display/cursor_check.py", "cursor_check")
        out = io.StringIO()
        with redirect_stdout(out):
            rc = cc.selftest()
        self.assertEqual(rc, 0, out.getvalue())


if __name__ == "__main__":
    unittest.main()
