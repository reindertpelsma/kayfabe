#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/full_tree_tests.py (STATUS: LIVE, 2026-11-09).

Run: python3 -m unittest scripts/ci/test_full_tree_tests.py
Falsifier: a listed test that is not run and passed on the full tree while the checker stays green, or a
test skipped on the stripped tree that is not on the list.
"""

import os
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
SCRIPT = os.environ.get("KF_FULL_TREE_SCRIPT") or os.path.join(HERE, "full_tree_tests.py")


def load():
    import importlib.util

    spec = importlib.util.spec_from_file_location("full_tree_tests_under_test", SCRIPT)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


F = load()
GOOD = "pkg-a\ttgt\ttest_one\treads docs\npkg-b\ttgt2\ttest_two\treads archive\n"


def write(tmp, text, name="list.tsv"):
    p = os.path.join(tmp, name)
    with open(p, "w") as f:
        f.write(text)
    return p


def crate(tmp, package, target, body):
    d = os.path.join(tmp, "crates", package, "tests")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, target + ".rs"), "w") as f:
        f.write(body)


class Rows(unittest.TestCase):
    def rows(self, text):
        with tempfile.TemporaryDirectory() as tmp:
            return F.rows(write(tmp, text))

    def test_a_good_list_parses(self):
        self.assertEqual(
            self.rows("# c\n" + GOOD),
            [
                ("pkg-a", "tgt", "test_one", "reads docs"),
                ("pkg-b", "tgt2", "test_two", "reads archive"),
            ],
        )

    def test_a_row_with_the_wrong_column_count_is_refused(self):
        for bad in ("a\tb\tc\n", "a\tb\tc\td\te\n", "a\tb\t\td\n"):
            with self.assertRaises(SystemExit, msg=bad):
                self.rows(bad)

    def test_a_malformed_name_is_refused(self):
        for bad in ("Pkg\tt\tn\tr\n", "pkg\t1t\tn\tr\n", "pkg\tt\tn-1\tr\n", "pkg\tt\tn n\tr\n"):
            with self.assertRaises(SystemExit, msg=bad):
                self.rows(bad)

    def test_a_duplicate_is_refused(self):
        with self.assertRaises(SystemExit):
            self.rows(GOOD + GOOD.splitlines()[0] + "\n")

    def test_an_empty_list_is_refused(self):
        for text in ("", "# only a comment\n\n"):
            with self.assertRaises(SystemExit, msg=repr(text)):
                self.rows(text)


class Skips(unittest.TestCase):
    def test_skips_name_every_test_exactly(self):
        rows = [("p", "t", "one", "x"), ("q", "u", "two", "y")]
        self.assertEqual(F.skips(rows), ["--skip", "one", "--skip", "two", "--exact"])

    def test_the_real_list_is_skipped_by_the_cli_in_one_line(self):
        p = subprocess.run([sys.executable, SCRIPT, "skips"], capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stderr)
        words = p.stdout.split()
        self.assertEqual(words[-1], "--exact")
        self.assertEqual(words.count("--skip"), len(F.rows()))


class Cli(unittest.TestCase):
    """The command line, against a throwaway repository root holding one list."""

    def cli(self, command, tsv, with_test=True):
        with tempfile.TemporaryDirectory() as tmp:
            os.makedirs(os.path.join(tmp, "scripts/ci"))
            with open(os.path.join(tmp, "scripts/ci/full_tree_tests.tsv"), "w") as f:
                f.write(tsv)
            if with_test:
                crate(tmp, "pkg-a", "tgt", "#[test]\nfn test_one() {}\n")
            return subprocess.run(
                [sys.executable, SCRIPT, command],
                env=dict(os.environ, KF_FULL_TREE_ROOT=tmp),
                capture_output=True,
                text=True,
            )

    def test_a_good_list_checks_clean(self):
        p = self.cli("check", "pkg-a\ttgt\ttest_one\tx\n")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("FULL_TREE_TESTS_OK 1 tests", p.stdout)

    def test_skips_prints_the_listed_names(self):
        p = self.cli("skips", "pkg-a\ttgt\ttest_one\tx\n")
        self.assertEqual(p.stdout.split(), ["--skip", "test_one", "--exact"])

    def test_the_cli_refuses_a_list_with_a_duplicate(self):
        row = "pkg-a\ttgt\ttest_one\tx\n"
        self.assertNotEqual(self.cli("check", row + row).returncode, 0)

    def test_the_cli_refuses_an_empty_list(self):
        self.assertNotEqual(self.cli("check", "# nothing\n").returncode, 0)

    def test_the_cli_refuses_a_row_with_too_few_columns(self):
        p = self.cli("check", "pkg-a\ttgt\ttest_one\n")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("4 non-empty tab separated columns", p.stderr)

    def test_the_cli_refuses_a_bad_test_name(self):
        p = self.cli("check", "pkg-a\ttgt\ttest-one\tx\n")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("bad package, target or test name", p.stderr)

    def test_the_cli_refuses_a_test_that_is_not_in_the_crate(self):
        p = self.cli("check", "pkg-a\ttgt\ttest_gone\tx\n")
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("FULL_TREE_TESTS_PROBLEM", p.stdout)

    def test_the_cli_refuses_a_target_under_the_wrong_package(self):
        p = self.cli("check", "pkg-b\ttgt\ttest_one\tx\n")
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("no such test target", p.stdout)


class Check(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.saved = F.ROOT
        F.ROOT = self.tmp.name
        self.addCleanup(setattr, F, "ROOT", self.saved)

    ROWS = [("pkg-a", "tgt", "test_one", "x")]

    def test_a_listed_test_that_exists_passes(self):
        crate(self.tmp.name, "pkg-a", "tgt", "#[test]\nfn test_one() {}\n")
        self.assertEqual(F.check(self.ROWS), [])

    def test_a_missing_test_target_is_reported(self):
        self.assertTrue(any("no such test target" in p for p in F.check(self.ROWS)))

    def test_a_renamed_test_is_reported(self):
        crate(self.tmp.name, "pkg-a", "tgt", "#[test]\nfn test_one_renamed() {}\n")
        self.assertTrue(any("no fn test_one" in p for p in F.check(self.ROWS)))

    def test_a_same_named_test_elsewhere_would_be_skipped_too_and_is_reported(self):
        crate(self.tmp.name, "pkg-a", "tgt", "#[test]\nfn test_one() {}\n")
        crate(self.tmp.name, "other", "elsewhere", "#[test]\nfn test_one() {}\n")
        self.assertTrue(any("another test of that name" in p for p in F.check(self.ROWS)))

    def test_a_same_named_test_in_a_source_file_is_reported_too(self):
        crate(self.tmp.name, "pkg-a", "tgt", "#[test]\nfn test_one() {}\n")
        d = os.path.join(self.tmp.name, "crates", "other", "src")
        os.makedirs(d)
        with open(os.path.join(d, "lib.rs"), "w") as f:
            f.write("#[cfg(test)]\nmod t {\n    #[test]\n    fn test_one() {}\n}\n")
        self.assertTrue(any("another test of that name" in p for p in F.check(self.ROWS)))

    def test_a_prefix_of_a_name_is_not_a_match(self):
        crate(self.tmp.name, "pkg-a", "tgt", "#[test]\nfn test_one() {}\n")
        crate(self.tmp.name, "other", "elsewhere", "#[test]\nfn test_one_more() {}\n")
        self.assertEqual(F.check(self.ROWS), [])


class Run(unittest.TestCase):
    """`run` against a fake `cargo` that prints a chosen libtest log."""

    def run_script(self, log, rc, rows="pkg-a\ttgt\ttest_one\tx\n"):
        with tempfile.TemporaryDirectory() as tmp:
            bindir = os.path.join(tmp, "bin")
            os.makedirs(bindir)
            with open(os.path.join(tmp, "fake.log"), "w") as f:
                f.write(log)
            cargo = os.path.join(bindir, "cargo")
            with open(cargo, "w") as f:
                f.write('#!/bin/bash\ncat "$FAKE_LOG"\nexit "$FAKE_RC"\n')
            os.chmod(cargo, os.stat(cargo).st_mode | stat.S_IEXEC)
            rowsfile = write(tmp, rows)
            code = textwrap.dedent("""
                import importlib.util, sys
                spec = importlib.util.spec_from_file_location("m", sys.argv[1])
                m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
                sys.exit(m.run(m.rows(sys.argv[2])))
                """)
            env = dict(
                os.environ,
                PATH=bindir + os.pathsep + os.environ["PATH"],
                FAKE_LOG=os.path.join(tmp, "fake.log"),
                FAKE_RC=str(rc),
            )
            return subprocess.run(
                [sys.executable, "-c", code, SCRIPT, rowsfile],
                env=env,
                capture_output=True,
                text=True,
            )

    OK = "running 1 test\ntest test_one ... ok\n\ntest result: ok. 1 passed; 0 failed\n"

    def test_a_passing_listed_test_is_green(self):
        p = self.run_script(self.OK, 0)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("FULL_TREE_TESTS ran=1 passed=1", p.stdout)

    def test_a_failing_cargo_is_red(self):
        p = self.run_script(self.OK, 101)
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("exited 101", p.stdout)

    def test_another_failing_test_of_the_target_is_red_even_though_the_listed_one_passed(self):
        p = self.run_script("test test_one ... ok\ntest other ... FAILED\n", 101)
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("exited 101", p.stdout)
        self.assertIn("passed=1", p.stdout)

    def test_a_listed_test_that_did_not_run_is_red_even_when_cargo_passes(self):
        p = self.run_script("running 0 tests\n\ntest result: ok. 0 passed; 0 failed\n", 0)
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("did not run and pass", p.stdout)

    def test_a_listed_test_that_failed_is_red(self):
        p = self.run_script("test test_one ... FAILED\n", 101)
        self.assertEqual(p.returncode, 1)
        self.assertIn("FULL_TREE_TEST FAIL pkg-a/tgt test_one", p.stdout)

    def test_a_test_that_is_only_ignored_is_red(self):
        p = self.run_script("test test_one ... ignored\n", 0)
        self.assertEqual(p.returncode, 1, p.stdout)

    def test_one_missing_test_among_several_is_red(self):
        rows = "pkg-a\ttgt\ttest_one\tx\npkg-a\ttgt\ttest_two\tx\n"
        p = self.run_script(self.OK, 0, rows)
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("test_two did not run and pass", p.stdout)
        self.assertIn("ran=2 passed=1", p.stdout)


class RealList(unittest.TestCase):
    def test_the_real_list_checks_clean_against_the_real_tree(self):
        self.assertEqual(F.check(F.rows()), [])

    def test_the_real_list_is_not_empty_and_every_row_says_what_it_reads(self):
        rows = F.rows()
        self.assertGreaterEqual(len(rows), 7)
        self.assertTrue(all(r[3] for r in rows))

    def test_the_workflow_runs_the_list_in_stable_and_skips_it_in_stripped(self):
        import yaml

        with open(os.path.join(F.ROOT, ".github/workflows/ci.yml")) as f:
            jobs = yaml.safe_load(f)["jobs"]
        stable = "\n".join(str(s.get("run", "")) for s in jobs["stable"]["steps"])
        stripped = "\n".join(str(s.get("run", "")) for s in jobs["stripped"]["steps"])
        self.assertIn("full_tree_tests.py run", stable)
        self.assertIn("full_tree_tests.py skips", stripped)
        self.assertIn("-- $skips", stripped)
        self.assertNotIn("full_tree_tests.py run", stripped)
        self.assertNotIn("full_tree_tests.py skips", stable)


if __name__ == "__main__":
    sys.exit(unittest.main())
