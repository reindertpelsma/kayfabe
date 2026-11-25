#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the walk cut at a per-space limit (STATUS: LIVE, 2026-11-25: a space the kernel cut is a named
ceiling, fails its split and holds its invalidate; docs/design/V3_REPLAY_TESTS.md "Walk cut at a per-space limit"):
the cut branch of kf_mem::vasmgr::VaManager::on_walk_ready, walk_cut_line, entry_diffs' `need`, and kf_cuda::abi::KF_ENTRY_CUT.

Each mutant is a one-site edit of production code; each must be killed by at least two independently named
failing tests. A compile error is not a kill. Every file touched is overwritten in place and ALWAYS restored
(also on an interrupt).
NOT a code branch, so not a mutant: "a channel already running on a cut space is not killed". No code kills a channel
on a cut; the absence of that code is stated in the PR and the design document, and has no test.
NOT covered: dropping the `rate_eprintln!` call altogether (stderr only; the source pin is the one test that sees it).
Run: python3 scripts/ci/walk_cut_mutants.py [CASE ...]   (exit 0 only if no mutant is weak)
Needs CARGO_TARGET_DIR in the environment when the default target directory is not wanted.
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
VAS = "crates/kf-mem/src/vasmgr.rs"
ABI = "crates/kf-cuda/src/abi.rs"
FILES = {}
for rel in (VAS, ABI):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()

MEM = [["cargo", "test", "-p", "kf-mem", "--lib", "vasmgr::tests"]]
CUDA = [["cargo", "test", "-p", "kf-cuda", "--lib", "per_space_limits_match_the_kernel"]]

CUT_HEAD = "                failed.insert(key);\n                self.stats.walk_cut_spaces"


def ed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old, FILES[rel].count(old))
    return rel, FILES[rel].replace(old, new)


CASES = [
    ("cut-check-inverted", ed(VAS, "if e.refused & kf_cuda::abi::KF_ENTRY_CUT != 0 {", "if e.refused & kf_cuda::abi::KF_ENTRY_CUT == 0 {"), MEM),
    ("cut-check-dead", ed(VAS, "if e.refused & kf_cuda::abi::KF_ENTRY_CUT != 0 {", "if false && e.refused & kf_cuda::abi::KF_ENTRY_CUT != 0 {"), MEM),
    ("cut-read-as-absent", ed(VAS, CUT_HEAD, "                self.stats.walk_cut_spaces"), MEM),
    ("cut-marked-absence-only", ed(VAS, CUT_HEAD, "                failed.insert(key);\n                absent_only.insert(key);\n                self.stats.walk_cut_spaces"), MEM),
    ("cut-fails-every-space-of-the-walk", ed(VAS, CUT_HEAD, "                failed.extend(batch.walked.keys().copied());\n                self.stats.walk_cut_spaces"), MEM),
    ("cut-counter-dropped", ed(VAS, "self.stats.walk_cut_spaces = self.stats.walk_cut_spaces.saturating_add(1);", "let _ = 1;"), MEM),
    ("cut-line-not-recorded", ed(VAS, 'kf_arch::rate_eprintln!("kf3: {line}");\n                self.stats.refuse(line);', 'kf_arch::rate_eprintln!("kf3: {line}");'), MEM),
    ("ceiling-log-unlimited", ed(VAS, 'kf_arch::rate_eprintln!("kf3: {line}");', 'eprintln!("kf3: {line}");'), MEM),
    ("entry-need-dropped", ed(VAS, "need: p.need(),", "need: 0,"), MEM),
    ("line-need-dropped", ed(VAS, "(refuse_mask={bits:#x}, reported need {need})", "(refuse_mask={bits:#x}, reported need 0)"), MEM),
    ("line-blames-the-space-alone", ed(VAS, "or the walk's shared total ({KF_WALK_SPACES_AT_BUDGET} x entry_budget), which can cut a space that is under its own budget", "which it exceeded"), MEM),
    ("line-budget-branch-inverted", ed(VAS, "if bits & KFWR_R_BUDGET != 0 {\n        limits.push", "if bits & KFWR_R_BUDGET == 0 {\n        limits.push"), MEM),
    ("line-frontier-branch-inverted", ed(VAS, "if bits & KFWR_R_FRONTIER_CAP != 0 {\n        limits.push", "if bits & KFWR_R_FRONTIER_CAP == 0 {\n        limits.push"), MEM),
    ("cut-mask-frontier-only", ed(ABI, "pub const KF_ENTRY_CUT: u32 = KFWR_R_BUDGET | KFWR_R_FRONTIER_CAP;", "pub const KF_ENTRY_CUT: u32 = KFWR_R_FRONTIER_CAP;"), MEM + CUDA),
    ("cut-mask-budget-only", ed(ABI, "pub const KF_ENTRY_CUT: u32 = KFWR_R_BUDGET | KFWR_R_FRONTIER_CAP;", "pub const KF_ENTRY_CUT: u32 = KFWR_R_BUDGET;"), MEM + CUDA),
]

FAIL = re.compile(r"(?m)^test (\S+) \.\.\. FAILED$")


def run(overrides, cmds):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    failed = []
    for cmd in cmds:
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
        if "could not compile" in out:
            raise SystemExit("the mutant does not compile:\n" + out[-800:])
        failed += FAIL.findall(out)
        if p.returncode != 0 and not FAIL.search(out):
            raise SystemExit("cargo failed without a failing test:\n" + out[-800:])
    return sorted(set(failed))


def restore():
    for rel, text in FILES.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)


def main():
    want = set(sys.argv[1:])
    unknown = want - {c[0] for c in CASES}
    if unknown:
        raise SystemExit("unknown case: %s" % sorted(unknown))
    cases = [c for c in CASES if not want or c[0] in want]
    try:
        base = run({}, MEM + CUDA)
        assert not base, "baseline is not green: %s" % base
        print("baseline: green")
        weak = []
        for name, (rel, text), cmds in cases:
            killers = run({rel: text}, cmds)
            restore()
            print("%-36s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
        return 1 if weak else 0
    finally:
        restore()


if __name__ == "__main__":
    sys.exit(main())
