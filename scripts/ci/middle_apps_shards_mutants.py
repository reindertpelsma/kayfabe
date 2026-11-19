#!/usr/bin/env python3
"""Mutation checks of the slicing of scripts/ci/middle_apps_mutants.py --shard I/N (2026-11-17, CI job layout): every
mutant is one exact-once text replacement in a private copy; scripts/ci/test_middle_apps_mutants_shards.py runs against the
copy and the failing tests are the killers. The unmutated copy must be green first and each mutant needs at least two named
killers (stdlib, GPU-free; the 118 mutants themselves are not run here, the test replaces the real tree and tests with fakes).

The slices are what lets the 99-minute harness (run 44088759756, one mutant after another) run on four machines, so these are
the ways it could quietly stop checking: a mutant in no slice, a mutant in two, the name filter ignored, a weak mutant that does
not fail its slice, an empty slice accepted, an out-of-range slice accepted."""
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC_REL = "scripts/ci/middle_apps_mutants.py"
TEST_REL = "scripts/ci/test_middle_apps_mutants_shards.py"
SRC = (ROOT / SRC_REL).read_text()
SLICE = "[args.only in case[0]][index - 1 :: count]"
CASES = [
    ("first-mutant-of-every-slice-dropped", "][index - 1 :: count]", "][index :: count]"),
    ("slices-overlap", "][index - 1 :: count]", "][index - 1 :]"),
    ("every-slice-is-the-first", "][index - 1 :: count]", "][:: count]"),
    ("filter-ignored", "cases = [case for case in CASES if args.only in case[0]]", "cases = [case for case in CASES]"),
    ("weak-mutant-not-reported", 'return int(any(row["count"] < 2 for row in rows))', "return 0"),
    ("empty-slice-accepted", "if count > 1 and not cases:", "if False:"),
    ("slice-bounds-unchecked", "if not 1 <= index <= count:", "if False:"),
]


def run(tree):
    p = subprocess.run([sys.executable, "-B", "-m", "unittest", "test_middle_apps_mutants_shards"], cwd=tree / "scripts/ci",
                       text=True, capture_output=True, timeout=600)
    out = p.stdout + p.stderr
    return p.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    weak = 0
    with tempfile.TemporaryDirectory(prefix="middle-shards-mutants-") as tmp:
        tree = Path(tmp)
        (tree / "scripts/ci").mkdir(parents=True)
        shutil.copy(ROOT / TEST_REL, tree / TEST_REL)
        target = tree / SRC_REL
        target.write_text(SRC)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-3000:])
            return 2
        print("PASS baseline", flush=True)
        for name, old, new in CASES:
            if SRC.count(old) != 1:
                print("BAD mutant %s: anchor found %d times" % (name, SRC.count(old)))
                return 2
            target.write_text(SRC.replace(old, new))
            try:
                code, killers, _ = run(tree)
            finally:
                target.write_text(SRC)
            ok = code != 0 and len(killers) >= 2
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(killers), killers[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
