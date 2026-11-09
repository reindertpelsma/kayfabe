#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for scripts/ci/fetch_oracles.sh (STATUS: LIVE, 2026-11-09).

Each mutant is a one-line change of the script; each must be killed by at least two independently
named failing tests of scripts/ci/test_fetch_oracles.py, like the other *_mutants.py harnesses here.
Run: python3 scripts/ci/fetch_oracles_mutants.py [--jobs N]  (exit 0 only if every mutant has >= 2 killers)
"""

import concurrent.futures
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = open(os.path.join(HERE, "fetch_oracles.sh")).read()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    (
        "pin-check-inverted",
        changed('if [ "$linked" != "$PIN" ]; then', 'if [ "$linked" = "$PIN" ]; then'),
    ),
    ("pin-check-skipped", changed('if [ "$linked" != "$PIN" ]; then', "if false; then")),
    (
        "fetch-failure-ignored",
        changed(
            '[ "$ok" = 1 ] || { echo "ORACLE_FETCH_FAIL fetch',
            'true || { echo "ORACLE_FETCH_FAIL fetch',
        ),
    ),
    ("single-attempt", changed("for attempt in 1 2 3; do", "for attempt in 1; do")),
    ("fetch-not-shallow", changed("fetch -q --depth 1 --no-tags", "fetch -q --no-tags")),
    (
        "reference-header-not-required",
        changed(
            '[ -s "$dest/src/common/inc/swref/published/nv_ref.h" ] || {',
            "true || {",
        ),
    ),
    (
        "stray-files-accepted",
        changed(
            'if [ -n "$(ls -A "$dest")" ] && [ ! -d "$dest/.git" ]; then',
            "if false; then",
        ),
    ),
    (
        "always-fetch",
        changed(
            'if [ "$(git -C "$dest" rev-parse HEAD 2>/dev/null || true)" != "$PIN" ]; then',
            "if true; then",
        ),
    ),
    ("never-checks-out", changed('git -C "$dest" checkout -q --detach "$PIN"', "true")),
    (
        "ok-line-without-size",
        changed(
            'echo "ORACLE_FETCH_OK ogkm-580 sha=$got files=$files worktree_bytes=$bytes git_bytes=$gitbytes seconds=$(($(date +%s) - start))"',
            'echo "ORACLE_FETCH_OK ogkm-580 sha=$got"',
        ),
    ),
    ("no-fresh-init", changed('[ -d "$dest/.git" ] || git -C "$dest" init -q', "true")),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('.',pattern='test_fetch_oracles.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'skipped':len(r.skipped),'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(source):
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(HERE, "test_fetch_oracles.py")) as f:
            tests = f.read()
        with open(os.path.join(d, "test_fetch_oracles.py"), "w") as f:
            f.write(tests)
        mutated = os.path.join(d, "fetch_oracles.sh")
        with open(mutated, "w") as f:
            f.write(source)
        p = subprocess.run(
            [sys.executable, "-W", "ignore", "-c", COLLECT],
            cwd=d,
            capture_output=True,
            text=True,
            timeout=900,
            env={
                **os.environ,
                "KF_FETCH_ORACLES_SCRIPT": mutated,
                "KF_LADDER_REPO": os.path.dirname(os.path.dirname(HERE)),
            },
        )
        assert p.returncode == 0, p.stderr[-300:]
        return json.loads(p.stdout.splitlines()[-1])


def main():
    jobs = int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 1
    base = run(SOURCE)
    assert base["killers"] == [], base
    print("baseline: %d tests, %d skipped, 0 failing" % (base["tests"], base["skipped"]))
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        results = list(pool.map(lambda case: run(case[1])["killers"], CASES))
    weak = []
    for (name, _src), killers in zip(CASES, results):
        print(
            "%-36s killers=%d %s" % (name, len(killers), "" if len(killers) >= 2 else "  <-- WEAK")
        )
        if len(killers) < 2:
            weak.append(name)
    print("MUTANTS total=%d weak=%d" % (len(CASES), len(weak)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
