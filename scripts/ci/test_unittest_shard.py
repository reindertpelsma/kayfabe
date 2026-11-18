"""scripts/ci/unittest_shard.py: a partition of the discovered test modules, and the plain runner's exit status.

STATUS: LIVE, 2026-11-17.

FALSIFIER (written before the code): a test module is in no shard (a test silently stops running in CI), or in two
(a shard runs it twice), or the partition differs between two computations of the same tree (two parallel jobs
disagree about who runs what); a shard that holds a failing test, a module that cannot be imported, or no test at
all exits 0; or the shards together run a different number of tests than a plain `unittest discover`.
"""

import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SHARD = HERE / "unittest_shard.py"

MODULES = {
    "test_alpha.py": "import unittest\nclass A(unittest.TestCase):\n    def test_one(self): pass\n    def test_two(self): pass\n",
    "test_bravo.py": "import unittest\nclass B(unittest.TestCase):\n    def test_fails(self): self.fail('deliberate')\n",
    "test_charlie.py": "import unittest\nclass C(unittest.TestCase):\n    def test_a(self): pass\n    def test_b(self): pass\n    def test_c(self): pass\n"
    + "#" * 600
    + "\n",
    "test_delta.py": "import unittest\nclass D(unittest.TestCase):\n    def test_only(self): pass\n",
    "test_echo.py": "import unittest\nclass E(unittest.TestCase):\n    def test_only(self): pass\n",
}
BROKEN = {"test_zulu.py": "this is not python (\n"}


def make_tree(files):
    tmp = tempfile.TemporaryDirectory(prefix="unittest-shard-test-")
    for name, text in files.items():
        Path(tmp.name, name).write_text(text)
    return tmp


def run(tree, *args):
    return subprocess.run(
        [sys.executable, "-I", "-B", str(SHARD), "-s", tree, "-p", "test_*.py", *args],
        capture_output=True,
        text=True,
        cwd=tree,
        timeout=300,
    )


def listing(tree, count):
    out = run(tree, "--count", str(count), "--list")
    assert out.returncode == 0, out.stderr
    shards, current = [], None
    for line in out.stdout.splitlines():
        if line.startswith("shard "):
            current = []
            shards.append(current)
        elif line.startswith("  "):
            current.append(line.strip())
    return shards


def ran(result):
    m = re.search(r"^Ran (\d+) tests?", result.stderr, re.M)
    return int(m.group(1)) if m else 0


class Partition(unittest.TestCase):
    def test_every_module_is_in_exactly_one_shard_for_every_shard_count(self):
        with make_tree({**MODULES, **BROKEN}) as tree:
            for count in range(1, 8):
                shards = listing(tree, count)
                flat = [name for shard in shards for name in shard]
                self.assertEqual(len(shards), count)
                self.assertEqual(sorted(flat), sorted(set(flat)), "a module is in two shards at count %d" % count)
                self.assertEqual(
                    sorted(flat),
                    sorted(n[:-3] for n in {**MODULES, **BROKEN}),
                    "a module is in no shard at count %d" % count,
                )

    def test_the_partition_is_the_same_every_time_it_is_computed(self):
        with make_tree(MODULES) as tree:
            self.assertEqual(listing(tree, 3), listing(tree, 3))

    def test_the_heaviest_module_does_not_share_a_shard_with_the_next_heaviest_when_there_is_room(self):
        with make_tree(MODULES) as tree:
            shards = listing(tree, 2)
            heavy = next(s for s in shards if "test_charlie" in s)
            self.assertEqual(len(heavy), 1, shards)

    def test_the_real_tree_partition_holds_every_test_discovery_finds(self):
        count = 3
        out = subprocess.run(
            [sys.executable, "-I", "-B", str(SHARD), "-s", "scripts/ci", "--count", str(count), "--list"],
            capture_output=True,
            text=True,
            cwd=ROOT,
            timeout=600,
        )
        self.assertEqual(out.returncode, 0, out.stderr)
        per_shard = [int(n) for n in re.findall(r"^shard \d+/\d+: \d+ modules, (\d+) tests$", out.stdout, re.M)]
        self.assertEqual(len(per_shard), count)
        # A plain discovery in a fresh interpreter: a test that leaked state into this process cannot change the count.
        plain = subprocess.run(
            [sys.executable, "-I", "-B", "-c",
             "import unittest;print(unittest.TestLoader().discover('scripts/ci', pattern='test_*.py').countTestCases())"],
            capture_output=True, text=True, cwd=ROOT, timeout=600,
        )
        self.assertEqual(plain.returncode, 0, plain.stderr[-400:])
        discovered = int(plain.stdout.split()[-1])
        self.assertGreater(discovered, 1000)
        self.assertEqual(sum(per_shard), discovered, "a test is in no shard, or in two")
        # a module that gets its tests through `load_tests` (test_vast_s4_import) is named by the tests it loads
        listed = {line.strip() for line in out.stdout.splitlines() if line.startswith("  ")}
        self.assertIn("test_ci_concurrency", listed)
        self.assertGreaterEqual(len(listed), 100)


class Status(unittest.TestCase):
    def test_a_shard_with_a_failing_test_fails_and_a_clean_shard_passes(self):
        with make_tree(MODULES) as tree:
            shards = listing(tree, 5)
            for index, names in enumerate(shards, 1):
                result = run(tree, "--count", "5", "--index", str(index))
                expect = 1 if "test_bravo" in names else 0
                self.assertEqual(result.returncode, expect, (index, names, result.stderr[-400:]))

    def test_a_module_that_cannot_be_imported_fails_its_shard(self):
        with make_tree({"test_ok.py": MODULES["test_alpha.py"], **BROKEN}) as tree:
            self.assertEqual(run(tree, "--count", "1", "--index", "1").returncode, 1)
            shards = listing(tree, 2)
            self.assertIn("test_zulu", [n for shard in shards for n in shard])
            codes = [run(tree, "--count", "2", "--index", str(i)).returncode for i in (1, 2)]
            self.assertEqual(sorted(codes), [0, 1], codes)

    def test_each_shard_runs_exactly_the_modules_and_tests_the_listing_gives_it(self):
        with make_tree(MODULES) as tree:
            listed = run(tree, "--count", "3", "--list").stdout
            expected = re.findall(r"^shard (\d+)/3: (\d+) modules, (\d+) tests$", listed, re.M)
            self.assertEqual(len(expected), 3)
            for number, modules, tests in expected:
                result = run(tree, "--count", "3", "--index", number)
                header = re.search(r"shard %s/3 runs (\d+) of \d+ modules \((\d+) of \d+ tests\)" % number, result.stdout)
                self.assertIsNotNone(header, result.stdout)
                self.assertEqual((header[1], header[2]), (modules, tests))
                self.assertEqual(str(ran(result)), tests, "shard %s ran a different number of tests than it announced" % number)

    def test_a_shard_that_holds_no_test_fails(self):
        with make_tree({"test_ok.py": MODULES["test_alpha.py"]}) as tree:
            codes = [run(tree, "--count", "3", "--index", str(i)).returncode for i in (1, 2, 3)]
            self.assertEqual(sorted(codes), [0, 1, 1], codes)

    def test_more_shards_than_modules_leaves_empty_shards_that_fail(self):
        with make_tree({"test_alpha.py": MODULES["test_alpha.py"], "test_delta.py": MODULES["test_delta.py"]}) as tree:
            codes = [run(tree, "--count", "4", "--index", str(i)).returncode for i in (1, 2, 3, 4)]
            self.assertEqual(sorted(codes), [0, 0, 1, 1], codes)

    def test_the_shards_run_exactly_the_tests_a_plain_discover_runs(self):
        with make_tree(MODULES) as tree:
            plain = subprocess.run(
                [sys.executable, "-I", "-B", "-m", "unittest", "discover", "-s", tree, "-p", "test_*.py"],
                capture_output=True,
                text=True,
                cwd=tree,
                timeout=300,
            )
            total = sum(ran(run(tree, "--count", "3", "--index", str(i))) for i in (1, 2, 3))
            self.assertEqual(total, ran(plain))
            self.assertEqual(plain.returncode, 1)

    def test_index_out_of_range_and_count_zero_are_refused(self):
        with make_tree(MODULES) as tree:
            for args in (["--count", "3", "--index", "0"], ["--count", "3", "--index", "4"], ["--count", "0", "--index", "1"], ["--count", "3"]):
                self.assertEqual(run(tree, *args).returncode, 2, args)


if __name__ == "__main__":
    sys.exit(unittest.main())
