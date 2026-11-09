#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for scripts/ci/full_tree_tests.py (STATUS: LIVE, 2026-11-09).

Each mutant is a small change of the script; each must be killed by at least two independently named
failing tests of scripts/ci/test_full_tree_tests.py. Run:
  python3 scripts/ci/full_tree_tests_mutants.py [--jobs N]   (exit 0 only if no mutant is weak)
"""

import concurrent.futures
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = open(os.path.join(HERE, "full_tree_tests.py")).read()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    ("skips-not-exact", changed('return args + ["--exact"]', "return args")),
    ("skips-name-the-target", changed('args += ["--skip", test]', 'args += ["--skip", _target]')),
    ("cargo-failure-ignored", changed("        if p.returncode != 0:\n", "        if False:\n")),
    ("missing-test-tolerated", changed("            if not ok:\n", "            if False:\n")),
    ("only-ok-counts-lost", changed('r"(?m)^test %s \\.\\.\\. ok$"', 'r"(?m)^test %s \\.\\.\\. "')),
    (
        "rename-not-detected",
        changed(
            '        if not re.search(r"(?m)^\\s*(?:pub', '        if re.search(r"(?m)^\\s*(?:pub'
        ),
    ),
    (
        "same-name-elsewhere-tolerated",
        changed('if not name.endswith(".rs") or full in listed:', "if True:"),
    ),
    (
        "missing-target-tolerated",
        changed(
            "        if src is None:\n            problems.append",
            "        if False:\n            problems.append",
        ),
    ),
    ("duplicates-tolerated", changed("    if len(set(keys)) != len(keys):", "    if False:")),
    ("empty-list-tolerated", changed("    if not out:\n", "    if False:\n")),
    (
        "column-count-unchecked",
        changed("            if len(cols) != 4 or not all(cols):", "            if False:"),
    ),
    (
        "name-syntax-unchecked",
        changed(
            "            if not (PACKAGE.match(package) and NAME.match(target) and NAME.match(test)):",
            "            if False:",
        ),
    ),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('.',pattern='test_full_tree_tests.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(source):
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(HERE, "test_full_tree_tests.py")) as f:
            tests = f.read()
        with open(os.path.join(d, "test_full_tree_tests.py"), "w") as f:
            f.write(tests)
        mutated = os.path.join(d, "full_tree_tests.py")
        with open(mutated, "w") as f:
            f.write(source)
        p = subprocess.run(
            [sys.executable, "-W", "ignore", "-c", COLLECT],
            cwd=d,
            capture_output=True,
            text=True,
            timeout=600,
            env={
                **os.environ,
                "KF_FULL_TREE_SCRIPT": mutated,
                "KF_FULL_TREE_ROOT": os.path.dirname(os.path.dirname(HERE)),
            },
        )
        assert p.returncode == 0, p.stderr[-400:]
        return json.loads(p.stdout.splitlines()[-1])


def main():
    jobs = int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 1
    base = run(SOURCE)
    assert base["killers"] == [], base
    print("baseline: %d tests, 0 failing" % base["tests"])
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        results = list(pool.map(lambda case: run(case[1])["killers"], CASES))
    weak = []
    for (name, _src), killers in zip(CASES, results):
        print(
            "%-34s killers=%d %s" % (name, len(killers), "" if len(killers) >= 2 else "  <-- WEAK")
        )
        if len(killers) < 2:
            weak.append(name)
    print("MUTANTS total=%d weak=%d" % (len(CASES), len(weak)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
