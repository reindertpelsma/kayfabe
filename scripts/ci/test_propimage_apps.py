#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage_apps.py fixtures: no VM, ssh or guest."""
import importlib.util
from pathlib import Path
import unittest

SRC = Path(__file__).resolve().parents[2] / "scripts/bench/windows/propimage_apps.py"
spec = importlib.util.spec_from_file_location("fixture_propimage_apps", SRC)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class T(unittest.TestCase):
    def test_every_check_renders_and_writes_its_json(self):
        for n in m.CHECKS:
            s = m.render_check(n, {"SP_CLICKS": "1,2;3,4"})
            self.assertIn("function Shot", s)
            self.assertIn("$D\\%s.json" % n, s)

    def test_calibration_vars_are_validated(self):
        with self.assertRaises(ValueError):
            m.env_prefix({"X": "1; calc"})
        with self.assertRaises(ValueError):
            m.env_prefix({"bad": "1"})

    def test_no_check_signs_in_or_types_credentials(self):
        for n, c in m.CHECKS.items():
            self.assertNotRegex(c["body"], r"(?i)password|sendkeys|Invoke-WebRequest|Invoke-RestMethod")

    def test_task_is_interactive(self):
        self.assertIn("-LogonType Interactive", m.task_wrapper("x"))

    def test_table(self):
        t = m.table([{"app": "a", "ok": True, "secs": 3, "signal": "s"}, {"app": "b", "secs": 1, "error": "timeout"}])
        self.assertIn("a\tyes\t3\ts", t)
        self.assertIn("b\tNO\t1\ttimeout", t)


if __name__ == "__main__":
    unittest.main()
