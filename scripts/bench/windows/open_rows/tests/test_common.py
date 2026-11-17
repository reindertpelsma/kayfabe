# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kfopen_common.py: the process runner (argument lists only), the output protocol and the trust record. GPU-free; runs the host's Python as the child."""
import contextlib
import io
import sys
import unittest

import helpers  # noqa: F401  (puts open_rows on sys.path)
import kfopen_common as K
from kfopen_common import OpenRowError, Runner, Trust


class RunnerTests(unittest.TestCase):
    def test_runner_refuses_a_shell_string(self):
        with self.assertRaisesRegex(OpenRowError, "argv must be a list"):
            Runner().run("echo hello && echo world")

    def test_runner_refuses_non_string_arguments(self):
        with self.assertRaises(OpenRowError):
            Runner().run([sys.executable, 1])

    def test_runner_returns_output_and_exit_code(self):
        r = Runner().run([sys.executable, "-c", "import sys; print('hi'); sys.exit(7)"], timeout=30)
        self.assertEqual(7, r.rc)
        self.assertIn("hi", r.out)
        self.assertFalse(r.timed_out)

    def test_runner_kills_on_timeout_and_says_so(self):
        r = Runner().run([sys.executable, "-c", "import time; time.sleep(30)"], timeout=1)
        self.assertTrue(r.timed_out)
        self.assertEqual(124, r.rc)

    def test_runner_reports_a_missing_program_as_127(self):
        r = Runner().run(["/no/such/program-kfopen"])
        self.assertEqual(127, r.rc)
        self.assertIn("START_FAILED", r.out)

    def test_shell_metacharacters_in_an_argument_are_data(self):
        r = Runner().run([sys.executable, "-c", "import sys; print(sys.argv[1])", "a; echo injected && b"], timeout=30)
        self.assertIn("a; echo injected && b", r.out)


class Output(unittest.TestCase):
    def test_say_and_check_line_formats(self):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            K.say("VERDICT", "PASS row=x")
            K.check_line("a", True)
            K.check_line("b", False, "because")
            K.check_line("c", False)
        self.assertEqual("KFOPEN VERDICT PASS row=x\nCHECK a ok\nCHECK b FAIL because\nCHECK c FAIL\n", buf.getvalue())

    def test_the_fail_line_matches_the_matrix_default_fail_regex(self):
        import re
        self.assertIsNotNone(re.search(r"^CHECK .*FAIL", "CHECK b FAIL because", re.M))
        self.assertIsNone(re.search(r"^CHECK .*FAIL", "CHECK a ok", re.M))


class TrustRecord(unittest.TestCase):
    def test_trust_record_says_git_only_until_something_is_lifted(self):
        self.assertEqual("git-only", Trust().record()["mode"])
        self.assertEqual("lifted", Trust(prebuilt="C:\\mine").record()["mode"])
        self.assertEqual("lifted", Trust(commit_overrides={"cts": "1" * 40}).record()["mode"])

    def test_fake_uuid_prefix_is_the_one_of_the_repo_gate(self):
        self.assertEqual("fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e", K.FAKE_UUID_PREFIX)


if __name__ == "__main__":
    unittest.main()
