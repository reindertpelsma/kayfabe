#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The Rust tests that need the FULL tree (STATUS: LIVE, 2026-11-09).

scripts/ci/full_tree_tests.tsv lists, per package and test target, the tests that read documents, archived
sources or cited evidence which `.workerignore` deletes. One list serves both jobs of .github/workflows/ci.yml:

  skips   print the libtest arguments (`--skip NAME ... --exact`) the `stripped` job appends after `--`;
  run     run those tests, and the other tests of their targets, on the full tree (the `stable` job) and fail
          unless every listed name ran and passed;
  check   fail if a row is malformed, duplicated, or names a test that does not exist in the crate's sources.

A test is skipped on the stripped tree only by NAME, exactly (`--exact`), so a renamed test is not skipped there
and fails there until the list is updated; and `run` fails when a listed name did not run.
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# KF_FULL_TREE_ROOT: only the mutation harness sets it (it runs a mutated copy of this file elsewhere).
ROOT = os.environ.get("KF_FULL_TREE_ROOT") or os.path.dirname(os.path.dirname(HERE))
NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
PACKAGE = re.compile(r"^[a-z][a-z0-9-]*$")


def rows(path=None):
    path = path or os.path.join(ROOT, "scripts/ci/full_tree_tests.tsv")
    out = []
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f.read().split("\n"), 1):
            if not line or line.startswith("#"):
                continue
            cols = line.split("\t")
            if len(cols) != 4 or not all(cols):
                raise SystemExit("%s:%d: want 4 non-empty tab separated columns" % (path, n))
            package, target, test, reads = cols
            if not (PACKAGE.match(package) and NAME.match(target) and NAME.match(test)):
                raise SystemExit("%s:%d: bad package, target or test name" % (path, n))
            out.append((package, target, test, reads))
    if not out:
        raise SystemExit(
            "%s lists no test: the stripped job would skip nothing and stable run nothing" % path
        )
    keys = [r[:3] for r in out]
    if len(set(keys)) != len(keys):
        raise SystemExit("%s: a test is listed twice" % path)
    return out


def source_of(package, target):
    p = os.path.join(ROOT, "crates", package, "tests", target + ".rs")
    return p if os.path.exists(p) else None


def check(root_rows):
    problems = []
    for package, target, test, _reads in root_rows:
        src = source_of(package, target)
        if src is None:
            problems.append("%s/%s: no such test target" % (package, target))
            continue
        with open(src, encoding="utf-8", errors="replace") as f:
            text = f.read()
        if not re.search(r"(?m)^\s*(?:pub\s+)?(?:async\s+)?fn\s+%s\s*\(" % re.escape(test), text):
            problems.append("%s/%s: no fn %s" % (package, target, test))
    # The skip is by bare name across the whole workspace: no test outside the listed targets may share it.
    listed = {source_of(r[0], r[1]) for r in root_rows}
    for test in sorted({r[2] for r in root_rows}):
        pattern = re.compile(r"(?m)^\s*(?:pub\s+)?(?:async\s+)?fn\s+%s\s*\(" % re.escape(test))
        for base, dirs, files in os.walk(os.path.join(ROOT, "crates")):
            dirs[:] = [d for d in dirs if d not in ("target", ".git")]
            for name in files:
                full = os.path.join(base, name)
                if not name.endswith(".rs") or full in listed:
                    continue
                with open(full, encoding="utf-8", errors="replace") as f:
                    if pattern.search(f.read()):
                        problems.append(
                            "%s: another test of that name would be skipped too"
                            % full[len(ROOT) + 1 :]
                        )
    return problems


def skips(root_rows):
    args = []
    for _package, _target, test, _reads in root_rows:
        args += ["--skip", test]
    return args + ["--exact"]


def run(root_rows):
    failed = []
    seen = []
    for package, target in sorted({r[:2] for r in root_rows}):
        p = subprocess.run(
            ["cargo", "test", "-p", package, "--test", target, "--no-fail-fast"],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        sys.stdout.write(p.stdout[-4000:])
        sys.stderr.write(p.stderr[-2000:])
        if p.returncode != 0:
            failed.append("%s/%s exited %d" % (package, target, p.returncode))
        for pk, tg, test, _reads in root_rows:
            if (pk, tg) != (package, target):
                continue
            ok = re.search(r"(?m)^test %s \.\.\. ok$" % re.escape(test), p.stdout)
            seen.append((pk, tg, test, bool(ok)))
            if not ok:
                failed.append("%s/%s: %s did not run and pass" % (pk, tg, test))
    for pk, tg, test, ok in seen:
        print("FULL_TREE_TEST %s %s/%s %s" % ("PASS" if ok else "FAIL", pk, tg, test))
    print("FULL_TREE_TESTS ran=%d passed=%d" % (len(seen), sum(1 for s in seen if s[3])))
    for f in failed:
        print("FULL_TREE_TESTS_FAIL " + f)
    return 1 if failed else 0


def main(argv):
    if len(argv) != 2 or argv[1] not in ("skips", "run", "check"):
        print(__doc__)
        return 2
    r = rows()
    if argv[1] == "skips":
        print(" ".join(skips(r)))
        return 0
    problems = check(r)
    for p in problems:
        print("FULL_TREE_TESTS_PROBLEM " + p)
    if problems:
        return 1
    if argv[1] == "check":
        print("FULL_TREE_TESTS_OK %d tests" % len(r))
        return 0
    return run(r)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
