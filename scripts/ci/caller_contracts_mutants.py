#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for scripts/ci/caller_contracts.py (2026-11-14).

Each mutant must be killed by at least two independently named failing tests of
scripts/ci/test_caller_contracts.py, like the other *_mutants.py harnesses in this directory.
Run: python3 -B scripts/ci/caller_contracts_mutants.py  (exit 0 only if every mutant has >= 2 killers)
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SOURCE = open(os.path.join(HERE, "caller_contracts.py")).read()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    ("stale-row-check-dropped", changed("        if key not in counts:\n", "        if False:\n")),
    ("count-compare-dropped", changed("elif rows[key][0] != n:", "elif False:")),
    ("citations-count-as-definitions", changed("if not DEF.match(line):", 'if TOKEN not in line:')),
    ("empty-reason-accepted", changed("if not f[3].strip():", "if False:")),
    (
        "missing-item-accepted",
        changed("            if item is None:\n                errors.append(", "            if item is None:\n                continue\n                errors.append("),
    ),
    ("missing-row-accepted", changed("        if key not in rows:\n", "        if False:\n")),
    ("untracked-files-scanned", changed('["git", "-C", str(root), "ls-files", "-z"]', '["git", "-C", str(root), "ls-files", "-z", "-o", "-c"]')),
    ("window-unbounded", changed("min(len(lines), i + 1 + WINDOW)", "len(lines)")),
    ("c-body-statement-skipped", changed("if not rust and s[:1].isspace():", "if False:")),
]


def killers(src):
    with tempfile.TemporaryDirectory() as d:
        src = re.sub(r"^ROOT = .*$", f"ROOT = Path({ROOT!r})", src, count=1, flags=re.M)
        open(os.path.join(d, "caller_contracts.py"), "w").write(src)
        shutil.copy(os.path.join(HERE, "test_caller_contracts.py"), d)
        r = subprocess.run(
            [sys.executable, "-B", "-m", "unittest", "-v", "test_caller_contracts"],
            cwd=d, capture_output=True, text=True, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
        )
        return sorted(set(re.findall(r"^(test_\w+).*\.\.\. (?:FAIL|ERROR)$", r.stderr, re.M)))


def main():
    bad = 0
    print(f"{'mutant':36} killers")
    for name, src in CASES:
        k = killers(src)
        flag = "" if len(k) >= 2 else "   <-- FEWER THAN 2"
        bad += len(k) < 2
        print(f"{name:36} {len(k)}  {', '.join(k)}{flag}")
    print(f"{len(CASES)} mutants, {bad} under-killed")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
