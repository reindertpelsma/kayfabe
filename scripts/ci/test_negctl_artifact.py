# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The archive scan of scripts/ci/negctl_artifact.py finds the name prefix and nothing else (STATUS: LIVE, 2026-11-25)."""

import importlib.util
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("negctl_artifact", ROOT / "scripts/ci/negctl_artifact.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


class Scan(unittest.TestCase):
    def check(self, data):
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d) / "lib.a"
            p.write_bytes(data)
            return mod.holds_name(p)

    def test_the_name_is_found_anywhere_in_the_archive(self):
        name = "KF3_NEG" + "CTL_HEAP"
        self.assertTrue(self.check(b"\0\1" + name.encode() + b"\0"))
        self.assertTrue(self.check(name.encode()))

    def test_an_archive_without_it_passes(self):
        self.assertFalse(self.check(b"KF3_TSPACE\0KF3_NEG\0CTL_\0" * 3))
        self.assertFalse(self.check(b""))

    def test_the_script_is_wired_into_ci_and_the_bench_build_only(self):
        wf = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("scripts/ci/negctl_artifact.py", wf)
        self.assertIn("--features kf-qemu/negctl", wf)


if __name__ == "__main__":
    unittest.main()
