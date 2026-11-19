#!/usr/bin/env python3
"""Mutation checks of scripts/ci/unittest_shard.py (2026-11-17, CI job layout): every mutant is one exact-once text
replacement in a private copy; scripts/ci/test_unittest_shard.py runs against the copy and the failing tests are the
killers. The unmutated copy must be green first and each mutant needs at least two named killers (stdlib, GPU-free).

The sharder decides which tests a CI job runs, so a wrong partition is a gate that silently stops checking: these are the
ways to get it wrong that matter (a module dropped, a module doubled, a failure not reported, an empty shard accepted)."""
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC_REL = "scripts/ci/unittest_shard.py"
TEST_REL = "scripts/ci/test_unittest_shard.py"
SRC = (ROOT / SRC_REL).read_text()
CASES = [
    # (name, exact text, replacement)
    ("shard-index-off-by-one", "chosen = shards[args.index - 1]", "chosen = shards[args.index % args.count]"),
    ("failure-exits-zero", "return 0 if result.wasSuccessful() else 1", "return 0"),
    ("empty-shard-passes", "if result.testsRun == 0:", "if False:"),
    (
        "import-error-modules-dropped",
        "    for test in flatten(suite):\n        modules.setdefault",
        "    for test in flatten(suite):\n        if type(test).__name__ == '_FailedTest':\n            continue\n        modules.setdefault",
    ),
    ("everything-in-the-first-shard", "target = min(range(count), key=lambda i: (load[i], i))", "target = 0"),
    ("module-also-in-the-first-shard", "        shards[target].append(name)\n", "        shards[target].append(name)\n        shards[0].append(name)\n"),
    ("only-the-first-module-of-a-shard-runs", "        mine.addTests(modules[name])\n", "        mine.addTests(modules[name])\n        break\n"),
]


def run(tree):
    listing = subprocess.run(
        [sys.executable, "-I", "-B", "-c",
         "import sys,unittest;sys.path.insert(0,'scripts/ci');import test_unittest_shard as t\n"
         "ids=[x.id() for g in unittest.TestLoader().loadTestsFromModule(t) for c in g for x in [c]]\n"
         "print('\\n'.join(i for i in ids if 'real_tree' not in i))"],
        cwd=tree, text=True, capture_output=True, timeout=120,
    )
    ids = [i for i in listing.stdout.split() if i]
    p = subprocess.run([sys.executable, "-B", "-m", "unittest", *ids], cwd=tree / "scripts/ci", text=True, capture_output=True, timeout=900)
    out = p.stdout + p.stderr
    return p.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    weak = 0
    with tempfile.TemporaryDirectory(prefix="unittest-shard-mutants-") as tmp:
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
