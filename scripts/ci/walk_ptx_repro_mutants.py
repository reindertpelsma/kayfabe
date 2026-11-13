#!/usr/bin/env python3
"""walk_ptx_repro.py mutants; every regression must have two distinct killers (STATUS: LIVE, 2026-11-13).

Each mutant rewrites one line of a temporary copy of walk_ptx_repro.py and runs test_walk_ptx_repro.py
against it. Refuses a missing or repeated anchor, a red baseline, a harness error, or fewer than two
failing test methods for any mutant.
"""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
MUTANTS = [
    ("blob-pin-ignored", "        if got != want:", "        if False:"),
    ("blob-missing-accepted", "        if not p.is_file():\n            out.append(", "        if not p.is_file():\n            continue\n            out.append("),
    ("constants-compared-loosely", "        elif k[name] != r[name]:", "        elif k[name] < r[name]:"),
    ("constant-missing-on-launcher-accepted", "        elif name not in r:\n            out.append(", "        elif name not in r:\n            continue\n            out.append("),
    ("expression-unchecked", '    if not re.fullmatch(r"[0-9\\s+\\-*()<>]+", e):', "    if False:"),
    ("compare-always-equal", "    if committed == rebuilt:", "    if True:"),
    ("walk-rebuild-failure-ignored", "        if p.returncode != 0 or not out.is_file():", "        if not out.is_file():"),
    ("walk-compiler-unchecked", "    if NVRTC_HEADER.encode() not in rebuilt[:400]:", "    if False:"),
    ("walk-committed-header-unchecked", "    if NVRTC_HEADER.encode() not in committed[:400]:", "    if False:"),
    ("display-any-clang", "        if want != clang_version:", "        if False:"),
    ("display-script-failure-ignored", "            if p.returncode != 0:", "            if False:"),
    ("display-not-compared", '            out += compare(f"cuda/display/{ptx}"', '            out += [] and compare(f"cuda/display/{ptx}"'),
]
COLLECT = """import io,json,sys,unittest
suite=unittest.defaultTestLoader.discover('.',pattern='test_walk_ptx_repro.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({'tests':r.testsRun,'killers':sorted(set(t.id().split(' (',1)[0] for t,_ in r.failures+r.errors))}))
"""


def run(directory):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT], cwd=directory, capture_output=True, text=True, timeout=300)
    if done.returncode:
        raise RuntimeError(f"mutant harness failure: {done.stderr[-800:]}")
    return json.loads(done.stdout.splitlines()[-1])


def main():
    source = (HERE / "walk_ptx_repro.py").read_text()
    with tempfile.TemporaryDirectory(prefix="kf-ptx-mutants-") as parent:
        d = Path(parent) / "scripts" / "ci"
        d.mkdir(parents=True)
        shutil.copyfile(HERE / "test_walk_ptx_repro.py", d / "test_walk_ptx_repro.py")
        # The tests read the real tree through ROOT = parents[2]; point the copy at it.
        real_root = HERE.parents[1]
        test = (d / "test_walk_ptx_repro.py").read_text()
        test = test.replace("ROOT = Path(__file__).resolve().parents[2]", f"ROOT = Path({str(real_root)!r})")
        (d / "test_walk_ptx_repro.py").write_text(test)
        src = source.replace("ROOT = Path(__file__).resolve().parents[2]", f"ROOT = Path({str(real_root)!r})")
        (d / "walk_ptx_repro.py").write_text(src)
        base = run(d)
        if base["killers"]:
            print(f"baseline is red: {base['killers']}")
            return 1
        bad = 0
        for name, old, new in MUTANTS:
            if src.count(old) != 1:
                print(f"{name}: anchor found {src.count(old)} times")
                bad += 1
                continue
            (d / "walk_ptx_repro.py").write_text(src.replace(old, new))
            got = run(d)
            k = got["killers"]
            ok = len(k) >= 2
            bad += 0 if ok else 1
            print(f"{'ok  ' if ok else 'FAIL'} {name}: {len(k)} killers {k[:3]}")
        (d / "walk_ptx_repro.py").write_text(src)
    print(f"walk_ptx_repro mutants: {len(MUTANTS) - bad}/{len(MUTANTS)} with >= 2 killers")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
