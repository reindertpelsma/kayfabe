"""scripts/ci/middle_apps_mutants.py --shard I/N: the N slices are every mutant exactly once, and each slice judges its own.

STATUS: LIVE, 2026-11-17.

FALSIFIER (written before the code): a mutant is in no slice (it silently stops being checked) or in two; the slices
come out different for the same arguments; a slice's weak mutant (fewer than two killers) does not fail that slice; a
malformed mutant (an anchor that is absent) does not fail the run; a slice that holds no mutant exits 0; `--shard 1/1`
(the default) is not the whole list in the order of CASES, as before.

Why a slice per machine and not a pool of workers on one: the tests the mutants run share fixed /tmp paths
(scripts/apps/setup_side.sh writes /tmp/setup_hf.out and reads it back), so two copies on one machine disturbed each
other (an extra killer from `test_apps_pins.SetupSideRows` in 2 of 2 six-worker runs against the sequential harness). A
spurious killer counts toward the minimum of two.

The real harness copies the repository and runs the real tests (about a minute a mutant), so this test replaces the two
functions that touch the real world (`prepare`, `collect`) with small fakes over one file per tree and runs the real
`main()` and `run_case()`.
"""

import importlib.util
import io
import json
import sys
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("middle_apps_mutants_under_test", HERE / "middle_apps_mutants.py")
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

ORIGINAL = "alpha = 1\nbeta = 2\ngamma = 3\n"
# name, file, old, new, mode: the fake `collect` counts one killer per line that differs from ORIGINAL, times a factor per mode
CASES = [
    ("alpha-changed", "scripts/x.py", "alpha = 1", "alpha = 9", "py"),
    ("beta-changed", "scripts/x.py", "beta = 2", "beta = 9", "node"),
    ("gamma-changed", "scripts/x.py", "gamma = 3", "gamma = 9", "config"),
    ("alpha-and-beta", "scripts/x.py", ("alpha = 1", "beta = 2"), ("alpha = 8", "beta = 8"), "py"),
    ("beta-again", "scripts/x.py", "beta = 2", "beta = 7", "py"),
    ("gamma-again", "scripts/x.py", "gamma = 3", "gamma = 7", "node"),
    ("alpha-again", "scripts/x.py", "alpha = 1", "alpha = 6", "config"),
]
FACTOR = {"py": 2, "node": 3, "config": 2, "single": 1}


def fake_prepare(root):
    (root / "scripts").mkdir(parents=True, exist_ok=True)
    (root / "scripts/x.py").write_text(ORIGINAL)


def fake_collect(root, mode):
    text = (root / "scripts/x.py").read_text()
    differing = sum(1 for a, b in zip(text.splitlines(), ORIGINAL.splitlines()) if a != b)
    return dict(tests=10, killers=["t%d_%s" % (n, mode) for n in range(differing * FACTOR[mode])], details="")


def run_main(shard=None, cases=CASES, only=""):
    argv = ["middle_apps_mutants.py"] + (["--shard", shard] if shard else []) + ([only] if only else [])
    out, err = io.StringIO(), io.StringIO()
    with mock.patch.object(M, "CASES", cases), mock.patch.object(M, "prepare", fake_prepare), mock.patch.object(
        M, "collect", fake_collect
    ), mock.patch.object(sys, "argv", argv), redirect_stdout(out), redirect_stderr(err):
        try:
            code, error = M.main(), None
        except (Exception, SystemExit) as caught:  # a malformed mutant or a bad argument must surface
            code, error = None, caught
    return code, out.getvalue(), err.getvalue(), error


def names(output):
    return [row["mutant"] for row in json.loads(output)["cases"]]


class Slices(unittest.TestCase):
    def test_the_slices_together_are_every_mutant_exactly_once_for_every_slice_count(self):
        for count in range(1, len(CASES) + 1):
            seen = []
            for index in range(1, count + 1):
                code, out, _, _ = run_main("%d/%d" % (index, count))
                self.assertEqual(code, 0, (index, count))
                seen += names(out)
            self.assertEqual(sorted(seen), sorted(c[0] for c in CASES), "count %d" % count)

    def test_the_default_is_the_whole_list_in_the_order_of_cases(self):
        for shard in (None, "1/1"):
            code, out, _, _ = run_main(shard)
            self.assertEqual((code, names(out)), (0, [c[0] for c in CASES]), shard)

    def test_the_same_arguments_give_the_same_slice(self):
        self.assertEqual(run_main("2/3")[1], run_main("2/3")[1])

    def test_slices_are_strided_so_the_test_modes_stay_mixed(self):
        _, out, _, _ = run_main("1/2")
        self.assertEqual(names(out), ["alpha-changed", "gamma-changed", "beta-again", "alpha-again"])
        _, out, _, _ = run_main("2/2")
        self.assertEqual(names(out), ["beta-changed", "alpha-and-beta", "gamma-again"])

    def test_a_slice_composes_with_the_name_filter(self):
        seen = []
        for index in (1, 2):
            seen += names(run_main("%d/2" % index, only="again")[1])
        self.assertEqual(sorted(seen), ["alpha-again", "beta-again", "gamma-again"])

    def test_the_real_mutant_list_is_unique_and_four_slices_differ_by_at_most_one(self):
        real = [c[0] for c in M.CASES]
        self.assertEqual(len(real), len(set(real)), "a duplicate mutant name would be counted once by a filter")
        sizes = [len(real[i::4]) for i in range(4)]
        self.assertLessEqual(max(sizes) - min(sizes), 1)
        self.assertEqual(sum(sizes), len(real))


class Verdicts(unittest.TestCase):
    def test_a_weak_mutant_fails_its_own_slice_and_only_that_slice(self):
        weak = CASES + [("weak", "scripts/x.py", "gamma = 3", "gamma = 5", "single")]  # one killer only, index 8 of 8
        codes = [run_main("%d/4" % i, cases=weak)[0] for i in (1, 2, 3, 4)]
        self.assertEqual(sorted(codes), [0, 0, 0, 1], codes)
        self.assertEqual(run_main("4/4", cases=weak)[0], 1)

    def test_a_weak_mutant_first_in_the_list_fails_the_run_too(self):
        weak = [("weak", "scripts/x.py", "gamma = 3", "gamma = 5", "single")] + CASES
        code, out, _, _ = run_main(None, cases=weak)
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(out)["cases"][0]["count"], 1)

    def test_strong_mutants_exit_zero(self):
        self.assertEqual(run_main(None)[0], 0)

    def test_a_malformed_mutant_fails_the_run(self):
        bad = CASES + [("anchor-absent", "scripts/x.py", "delta = 4", "delta = 5", "py")]
        code, _, _, error = run_main("1/1", cases=bad)
        self.assertIsNone(code)
        self.assertIn("anchor absent/ambiguous", str(error))

    def test_a_slice_with_no_mutant_fails_when_the_run_is_sliced(self):
        code, _, err, _ = run_main("5/5", cases=CASES[:3])
        self.assertEqual(code, 1)
        self.assertIn("holds no mutant", err)

    def test_an_unsliced_run_with_no_matching_mutant_is_empty_and_green_as_before(self):
        code, out, _, _ = run_main(None, only="no-such-mutant")
        self.assertEqual((code, json.loads(out)["cases"]), (0, []))

    def test_a_slice_index_above_the_count_is_refused(self):
        for text in ("5/4", "2/1", "9/8"):
            code, _, _, error = run_main(text)
            self.assertIsNone(code, text)
            self.assertIsInstance(error, SystemExit, text)

    def test_a_slice_index_of_zero_or_below_or_a_count_of_zero_is_refused(self):
        for text in ("0/4", "-1/4", "1/0", "0/0"):
            code, _, _, error = run_main(text)
            self.assertIsNone(code, text)
            self.assertIsInstance(error, SystemExit, text)

    def test_text_that_is_not_i_over_n_is_refused(self):
        for text in ("x", "1-4", "1/2/3", "1/", "a/b"):
            code, _, _, error = run_main(text)
            self.assertIsNone(code, text)
            self.assertIsInstance(error, SystemExit, text)

    def test_more_slices_than_mutants_leave_empty_slices_that_fail(self):
        codes = [run_main("%d/4" % i, cases=CASES[:2])[0] for i in (1, 2, 3, 4)]
        self.assertEqual(codes, [0, 0, 1, 1])

    def test_a_slice_prints_one_line_per_mutant_it_ran(self):
        _, _, err, _ = run_main("1/2")
        self.assertEqual(len([l for l in err.splitlines() if l.endswith("killers")]), 4)


if __name__ == "__main__":
    sys.exit(unittest.main())
