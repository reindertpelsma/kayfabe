# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The verdict rules of the Windows apps stage (grade.py): the guest passes, or it is not a pass."""
import os
import tempfile
import unittest

import helpers as H  # noqa: F401  (puts appmatrix on sys.path)
import grade


def res(app, verdict, **kw):
    return f"APPRES side=win app={app} verdict={verdict} rc=0 secs=3 quiet=0 note={kw.get('note', '-')} boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out\n"


class Grade(unittest.TestCase):
    def run_dir(self, batched, alone=()):
        d = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(d, ignore_errors=True))
        open(os.path.join(d, "win.res"), "w").write("".join(res(a, v) for a, v in batched))
        if alone:
            open(os.path.join(d, "win_isolated.res"), "w").write("".join(res(a, v) for a, v in alone))
        return d

    def test_all_pass_is_a_pass(self):
        rc, kind, counts, per, extra = grade.grade(self.run_dir([("a", "PASS"), ("b", "PASS*")]), ["a", "b"])
        self.assertEqual((rc, kind, counts["pass"]), (0, "PASS", 2))

    def test_alone_verdict_is_final_in_both_directions(self):
        rc, kind, *_ = grade.grade(self.run_dir([("a", "FAIL")], [("a", "PASS")]), ["a"])
        self.assertEqual((rc, kind), (0, "PASS"))                   # batched fail, alone pass: the alone one is final
        rc, kind, *_ = grade.grade(self.run_dir([("a", "PASS")], [("a", "FAIL")]), ["a"])
        self.assertEqual((rc, kind), (1, "FAIL"))                   # (a pass that fails when alone is a fail: it was re-run)

    def test_fail_and_timeout_are_guest_failures(self):
        for v in ("FAIL", "TIMEOUT"):
            rc, kind, counts, *_ = grade.grade(self.run_dir([("a", "PASS"), ("b", v)]), ["a", "b"])
            self.assertEqual((rc, kind, counts["guest_fail"]), (1, "FAIL", 1), v)

    def test_boot_fail_is_a_failure_of_the_guest_not_a_pass(self):
        rc, kind, counts, *_ = grade.grade(self.run_dir([("a", "BOOT_FAIL")]), ["a"])
        self.assertEqual((rc, kind, counts["boot_fail"]), (1, "FAIL", 1))

    def test_notrun_is_a_harness_fault_never_a_pass(self):
        rc, kind, counts, *_ = grade.grade(self.run_dir([("a", "PASS"), ("b", "NOTRUN")]), ["a", "b"])
        self.assertEqual((rc, kind, counts["harness"]), (3, "RETRY", 1))

    def test_a_missing_row_is_not_run_and_never_a_pass(self):
        rc, kind, counts, per, _ = grade.grade(self.run_dir([("a", "PASS")]), ["a", "b"])
        self.assertEqual((rc, kind, counts["not_run"]), (3, "RETRY", 1))
        self.assertEqual(per["b"]["cls"], "not_run")

    def test_a_guest_failure_outranks_a_harness_fault(self):
        rc, kind, *_ = grade.grade(self.run_dir([("a", "FAIL"), ("b", "NOTRUN")]), ["a", "b"])
        self.assertEqual((rc, kind), (1, "FAIL"))

    def test_unknown_verdict_word_is_never_a_pass(self):
        rc, kind, counts, *_ = grade.grade(self.run_dir([("a", "MAYBE")]), ["a"])
        self.assertEqual((rc, kind, counts["pass"]), (3, "RETRY", 0))

    def test_empty_expectation_and_empty_directory_are_not_a_pass(self):
        rc, kind, *_ = grade.grade(self.run_dir([("a", "PASS")]), [])
        self.assertEqual((rc, kind), (3, "RETRY"))
        rc, kind, *_ = grade.grade(tempfile.mkdtemp(), ["a"])
        self.assertEqual((rc, kind), (3, "RETRY"))

    def test_rows_for_apps_nobody_expected_do_not_pass_the_run(self):
        rc, kind, counts, per, extra = grade.grade(self.run_dir([("zzz", "PASS")]), ["a"])
        self.assertEqual((rc, kind, extra), (3, "RETRY", ["zzz"]))

    def test_cli_exit_codes_and_lines(self):
        import io
        import contextlib
        d = self.run_dir([("a", "PASS"), ("b", "TIMEOUT")])
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = grade.main([d, "--expected", "a,b"])
        out = buf.getvalue()
        self.assertEqual(rc, 1)
        self.assertIn("WINAPPS_GRADE kind=FAIL expected=2 pass=1 guest_fail=1", out)
        self.assertIn("WINAPPS_GRADE_GUEST_FAIL b", out)


if __name__ == "__main__":
    unittest.main()
