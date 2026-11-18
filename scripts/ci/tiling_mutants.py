#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for option C2's exact block tiling (STATUS: LIVE, 2026-11-18;
docs/design/V3_SPARSE_RESIDENCY_RCA.md section 8): crates/kf-mem/src/tiling.rs against the GPU-free tests in
crates/kf-mem/src/sim/tiling.rs.

Each mutant is a one-site edit of the engine; each must be killed by at least two independently named failing
tests. A compile error is not a kill. The file is overwritten in place and ALWAYS restored (also on an
interrupt). Pattern: scripts/ci/sparse_fill_mutants.py.
Run: python3 scripts/ci/tiling_mutants.py [CASE ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
ENG = "crates/kf-mem/src/tiling.rs"
with open(os.path.join(ROOT, ENG), encoding="utf-8") as f:
    TEXT = f.read()

TESTS = [["cargo", "test", "-p", "kf-mem", "--lib", "sim::tiling"]]


def ed(old, new):
    assert TEXT.count(old) == 1, (old, TEXT.count(old))
    return TEXT.replace(old, new)


CASES = [
    ("layout-ignores-2mib-rounding",
     ed("    if span(lo, hi) < HUGE || (lo.is_multiple_of(HUGE) && hi.is_multiple_of(HUGE)) {\n        return vec![(lo, hi, Attr::Default)];",
        "    if true {\n        return vec![(lo, hi, Attr::Default)];")),
    ("sparse-rounds-outward",
     ed("Base::Sparse => (up(a, TILE).ok_or(TileError::Outside)?, down(b, TILE)),",
        "Base::Sparse => (down(a, TILE), up(b, TILE).ok_or(TileError::Outside)?),")),
    ("invalid-rounds-inward",
     ed("Base::Invalid => (down(a, TILE), up(b, TILE).ok_or(TileError::Outside)?),",
        "Base::Invalid => (up(a, TILE).ok_or(TileError::Outside)?, down(b, TILE)),")),
    ("reconcile-frees-occupied-blocks",
     ed("            if self.occupied(k, blk.hi) {\n                segs.extend(seg.take());",
        "            if false {\n                segs.extend(seg.take());")),
    ("refusal-not-rolled-back",
     ed("        for (k, b) in freed {", "        for (k, b) in freed.into_iter().take(0) {")),
    ("s2-cut-like-s1",
     ed("        } else if mandatory {", "        } else if mandatory && false {")),
    ("rows-never-carved",
     ed("            if self.blocks.len().saturating_add(growth) > self.cfg.max_blocks\n",
        "            if true\n")),
    ("release-does-not-retile",
     ed("        self.reconcile(v, t0, t1).map_or(Ok(()), Err)\n    }\n\n    /// The base the host holds",
        "        let _ = (v, t0, t1);\n        Ok(())\n    }\n\n    /// The base the host holds")),
    ("shared-edge-tile-not-split",
     ed("                if edge && c0 < cut && cut < c1 && !cuts.contains(&cut) {",
        "                if edge && false && c0 < cut && cut < c1 && !cuts.contains(&cut) {")),
    ("sparse-reserved-as-invalid",
     ed("                match v.reserve(lo, hi, base, attr) {", "                match v.reserve(lo, hi, Base::Invalid, attr) {")),
    ("s2-retry-cut-by-budget",
     ed("        for (a, b, _) in pending.iter().filter(|p| p.2 == Base::Invalid) {\n",
        "        for (a, b, _) in pending.iter().filter(|p| p.2 == Base::Invalid).take(0) {\n")),
]


def run(text, cmds):
    if text is not None:
        with open(os.path.join(ROOT, ENG), "w", encoding="utf-8") as f:
            f.write(text)
    failed = []
    for cmd in cmds:
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
        if "could not compile" in out:
            raise SystemExit("the mutant does not compile:\n" + out[-800:])
        failed += re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out)
        if p.returncode != 0 and not re.search(r"(?m)^test (\S+) \.\.\. FAILED$", out):
            raise SystemExit("cargo failed without a failing test:\n" + out[-800:])
    return sorted(set(failed))


def restore():
    with open(os.path.join(ROOT, ENG), "w", encoding="utf-8") as f:
        f.write(TEXT)


def main():
    want = set(sys.argv[1:])
    cases = [c for c in CASES if not want or c[0] in want]
    try:
        base = run(None, TESTS)
        assert not base, "baseline is not green: %s" % base
        print("baseline: green")
        weak = []
        for name, text in cases:
            killers = run(text, TESTS)
            restore()
            print("%-34s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
        return 1 if weak else 0
    finally:
        restore()


if __name__ == "__main__":
    sys.exit(main())
