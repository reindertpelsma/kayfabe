#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for scripts/ci/release_claims.py (PROPOSAL, 2026-11-07).

Each mutant must be killed by at least two independently named failing tests of
scripts/ci/test_release_claims.py, like the other *_mutants.py harnesses in this directory.
Run: python3 scripts/ci/release_claims_mutants.py   (exit 0 only if every mutant has >= 2 killers)

A mutant runs in a scratch tree that links every top-level entry of the repository except the
mutated script and its test, so the tests that read the real README and ledger still work.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
SOURCE = open(os.path.join(HERE, "release_claims.py")).read()
TEST = os.path.join(HERE, "test_release_claims.py")


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    ("hash-normalisation-dropped", changed('return " ".join(text.split())', "return text")),
    ("verified-evidence-rules-skipped", changed('if status in ("verified", "outdated"):', 'if status in ("outdated",):')),
    ("stale-row-not-flagged", changed("if h not in by_hash:", "if False:")),
    ("uncovered-line-not-flagged", changed("if h not in covered:", "if False:")),
    ("duplicate-id-ignored", changed("if cid in seen:", "if False:")),
    (
        "wording-may-be-verified",
        changed('            if status == "verified":\n                errors.append(f"{at}: a wording row cannot be verified',
                '            if False:\n                errors.append(f"{at}: a wording row cannot be verified'),
    ),
    ("evidence-existence-skipped", changed("if not (root / path).exists():", "if False:")),
    ("future-date-allowed", changed("> today:", "> datetime.date.max:")),
    ("rev-prefix-always-matches", changed("b.startswith(a)", "True")),
    ("table-header-not-exempt", changed("continue  # table header row", "pass")),
    ("broken-link-not-flagged", changed("if not p.exists():", "if False:")),
    ("contradicted-note-not-needed", changed('if not row["note"] or row["note"] == "-":', "if False:")),
    ("auto-owner-hardware-allowed", changed('if where == "owner hardware":', "if False:")),
    ("new-check-may-be-kept", changed('if action == "keep" and status != "verified":', "if False:")),
    ("moved-line-number-ignored", changed("if by_hash[h][0] != sline:", "if False:")),
    ("manual-checklist-id-unchecked", changed("elif doc_text and ref not in mids:", "elif False:")),
]


def run_mutant(source):
    tmp = tempfile.mkdtemp(prefix="rc-mutant-")
    try:
        for name in os.listdir(REPO):
            if name in (".git", "scripts"):
                continue
            os.symlink(os.path.join(REPO, name), os.path.join(tmp, name))
        scripts = os.path.join(tmp, "scripts")
        os.mkdir(scripts)
        for name in os.listdir(os.path.join(REPO, "scripts")):
            if name != "ci":
                os.symlink(os.path.join(REPO, "scripts", name), os.path.join(scripts, name))
        ci = os.path.join(scripts, "ci")
        os.mkdir(ci)
        for name in os.listdir(os.path.join(REPO, "scripts", "ci")):
            if name not in ("release_claims.py", "test_release_claims.py"):
                os.symlink(os.path.join(REPO, "scripts", "ci", name), os.path.join(ci, name))
        open(os.path.join(ci, "release_claims.py"), "w").write(source)
        shutil.copy(TEST, os.path.join(ci, "test_release_claims.py"))
        r = subprocess.run(
            [sys.executable, "-I", os.path.join(ci, "test_release_claims.py")],
            cwd=tmp, capture_output=True, text=True, timeout=300,
        )
        out = r.stderr + r.stdout
        killers = sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, flags=re.M)))
        return r.returncode, killers
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    code, killers = run_mutant(SOURCE)
    if code != 0:
        print("baseline: the unmutated tests fail; fix them first")
        return 2
    weak = 0
    for name, source in CASES:
        code, killers = run_mutant(source)
        status = "ok" if len(killers) >= 2 else "WEAK"
        weak += status != "ok"
        print(f"{status:4} {name}: {len(killers)} killer(s) {killers[:4]}")
    print(f"{len(CASES) - weak}/{len(CASES)} mutants killed by two or more tests")
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
