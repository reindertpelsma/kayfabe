#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the guest-PTE -> host-PTE fixes and their fuzz (STATUS: LIVE, 2026-11-18).

Each mutant is one small change to production code (or to the oracle, or to the model) that the
PTE fuzz exists to catch. Each must be killed by at least TWO independently named failing tests
of `crates/kf-mem/src/sim/pte_fuzz.rs` (docs/design/V3_PTE_MODEL_FUZZ.md). The mutants are
applied IN PLACE, one at a time, and the original file is always restored. Run from a clean tree:
  python3 scripts/ci/pte_fuzz_mutants.py        (exit 0 only if every mutant is killed twice)
Needs cargo and g++, but no GPU.
"""

import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
T = "sim::pte_fuzz::"
F = T + "findings::"

# (name, file, old, new, killers)
MUTANTS = [
    (
        "turing-pd1-holds-a-page",
        "crates/kf-chip/src/lib.rs",
        "        !matches!(self, Family::Turing)\n",
        "        true\n",
        [F + "pf1_turing_pd1_bit0_is_not_a_512m_page", T + "pte_fuzz_quick"],
    ),
    (
        "blackwell-pd2-uncoded-off",
        "crates/kf-mem/src/apply.rs",
        "        kf_chip::MmuFormat::Ver3 if family.pd2_holds_page() => kf_cuda::abi::kf_format_ver3_pd2_page(),\n",
        "",
        [F + "pf2_blackwell_256g_page_is_refused_by_name_never_misread", T + "pte_fuzz_quick"],
    ),
    (
        "walker-uncoded-refusal-skipped",
        "cuda/walk/kf_walk.cu",
        "    if (L.leaf_ps == KF_PS_UNCODED && kf_valid(F, raw)) {   /* PF-2: a page we cannot name */\n"
        "        c.refuse |= KFWR_R_UNCODED_PAGE; c.refusals++;\n",
        "    if (L.leaf_ps == KF_PS_UNCODED && kf_valid(F, raw)) {   /* PF-2: a page we cannot name */\n",
        [F + "pf2_blackwell_256g_page_is_refused_by_name_never_misread", T + "pte_fuzz_quick"],
    ),
    (
        "model-drops-the-attributes",
        "crates/kf-mem/src/sim.rs",
        "                attr: self.cur_attr,\n",
        "                attr: (0, MapPerm::READ_WRITE),\n",
        [F + "plain_leaves_translate_exactly_on_every_family", T + "pte_fuzz_quick"],
    ),
    (
        "oracle-ver2-kind-off-by-one",
        "crates/kf-mem/src/sim/pte_oracle.rs",
        "            kind: bits(raw, 63, 56) as u8,",
        "            kind: bits(raw, 62, 55) as u8,",
        [F + "oracle_round_trips_every_field", F + "plain_leaves_translate_exactly_on_every_family"],
    ),
    (
        "readonly-not-carried",
        "crates/kf-mem/src/apply.rs",
        "            read_only: flags & KFWR_RF_READ_ONLY != 0,\n",
        "            read_only: false,\n",
        [F + "plain_leaves_translate_exactly_on_every_family", T + "pte_fuzz_quick"],
    ),
]


def failing(names):
    """Run each named test exactly; return the names that FAILED."""
    out = []
    for n in names:
        p = subprocess.run(
            ["cargo", "test", "-q", "-p", "kf-mem", "--release", "--lib", "--", "--exact", n],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        ran = f"test {n} ..." in p.stdout or "1 passed" in p.stdout or "1 failed" in p.stdout
        if p.returncode != 0 and ("1 failed" in p.stdout or "panicked" in p.stdout + p.stderr):
            out.append(n)
        elif p.returncode == 0 and not ran:
            print(f"  ! {n}: no such test", file=sys.stderr)
    return out


def main():
    weak = []
    baseline = failing(sorted({k for m in MUTANTS for k in m[4]}))
    if baseline:
        print(f"baseline not green: {baseline}")
        return 2
    for name, path, old, new, killers in MUTANTS:
        full = os.path.join(ROOT, path)
        src = open(full).read()
        if src.count(old) != 1:
            print(f"{name}: anchor not found exactly once in {path}")
            return 2
        try:
            open(full, "w").write(src.replace(old, new))
            killed = failing(killers)
        finally:
            open(full, "w").write(src)
        ok = len(killed) >= 2
        print(f"{'KILLED' if ok else 'WEAK  '} {name}: {len(killed)}/{len(killers)} killers fail {killed}")
        if not ok:
            weak.append(name)
    print(f"pte_fuzz_mutants: {len(MUTANTS) - len(weak)}/{len(MUTANTS)} killed twice")
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
