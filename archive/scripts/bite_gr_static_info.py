#!/usr/bin/env python3
"""★ The bite harness for the state-load rung (#150).

Plants one defect at a time in the **exact committed content**, runs the test target that
is supposed to notice, and restores. A test that stays green through its own defect is a
test that was never load-bearing, and this script is the only thing that says which is
which.

    python3 scripts/bite_gr_static_info.py            # measure every bite
    python3 scripts/bite_gr_static_info.py --list     # just print them

⚠ Two traps encoded here, both measured on this repository before:
  - **Touch after restore.** `shutil.copy2`/`cp -a` preserve mtime, so cargo serves a stale
    rlib and the next bite is measured against the PREVIOUS bite's build. Every write here
    is followed by an explicit `os.utime(None)`.
  - **`--no-fail-fast`.** cargo stops after the first failing *target*, which manufactures
    false non-biters when one bite legitimately reddens two targets.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# (name, file, old, new, target that must go red)
BITES = [
    (
        "floorsweeping: tpcCount written where tpcMask goes",
        "crates/kayfabe-abi/src/grstatic.rs",
        "        put32(row, O_TPC_MASK + 4 * i, g.tpc_mask);\n"
        "        put32(row, O_TPC_COUNT + 4 * i, g.tpc_count);",
        "        put32(row, O_TPC_MASK + 4 * i, g.tpc_count);\n"
        "        put32(row, O_TPC_COUNT + 4 * i, g.tpc_mask);",
        "gr_static_info",
    ),
    (
        "SM order: every SM claims localSmId 0",
        "crates/kayfabe-abi/src/grstatic.rs",
        "            put16(e, O_LOCAL_SM, local_sm);",
        "            put16(e, O_LOCAL_SM, 0);",
        "gr_static_info",
    ),
    (
        "SM order: numSm/numTpc placed after the USED entries, not the array",
        "crates/kayfabe-abi/src/grstatic.rs",
        "    put16(&mut out, GR_MAX_SM * SM_ENTRY_SIZE, num_sm);\n"
        "    put16(&mut out, GR_MAX_SM * SM_ENTRY_SIZE + 2, num_tpc);",
        "    put16(&mut out, sm * SM_ENTRY_SIZE, num_sm);\n"
        "    put16(&mut out, sm * SM_ENTRY_SIZE + 2, num_tpc);",
        "gr_static_info",
    ),
    (
        "gpcMask: a zero-GPC profile is allowed through (the rejected shortcut)",
        "crates/kayfabe-abi/src/grstatic.rs",
        "        if n == 0 || n > GR_MAX_GPC {",
        "        if n > GR_MAX_GPC {",
        "gr_static_info",
    ),
    (
        "the geometry cross-check is skipped: mask and count may disagree",
        "crates/kayfabe-abi/src/grstatic.rs",
        "            if g.tpc_mask.count_ones() != g.tpc_count {",
        "            if false && g.tpc_mask.count_ones() != g.tpc_count {",
        "gr_static_info",
    ),
    (
        "PDE publication: the alignment rule read as `hi` rather than `hi + 1`",
        "crates/kayfabe-abi/src/gvaspacepdes.rs",
        "    if !hi_end.is_multiple_of(page_size) {",
        "    if !virt_addr_hi.is_multiple_of(page_size) {",
        "gvaspace_pdes",
    ),
    (
        "PDE publication: a meaningful level of zero bytes is accepted",
        "crates/kayfabe-abi/src/gvaspacepdes.rs",
        "        if (i as u32) < num_levels && lv.size == 0 {",
        "        if false && (i as u32) < num_levels && lv.size == 0 {",
        "gvaspace_pdes",
    ),
    (
        "PDE publication: numLevelsToCopy is not bounded by GMMU_FMT_MAX_LEVELS",
        "crates/kayfabe-abi/src/gvaspacepdes.rs",
        "    if num_levels == 0 || num_levels as usize > GMMU_FMT_MAX_LEVELS {",
        "    if num_levels == 0 {",
        "gvaspace_pdes",
    ),
    (
        "PDE publication: re-encode becomes an echo of the request",
        "crates/kayfabe-abi/src/gvaspacepdes.rs",
        "        out[at + 20] = lv.page_shift;",
        "        out[at + 20] = 0;",
        "gvaspace_pdes",
    ),
    (
        "triage: 0x20800a1f goes back to the REFUTED RefusalHalts disposition",
        "crates/kayfabe-device/src/sweep.rs",
        '        cmd: 0x2080_0a1f,\n        engine: "KernelGraphics",\n'
        "        disposition: SweepDisposition::AmputationUnsurvivable,",
        '        cmd: 0x2080_0a1f,\n        engine: "KernelGraphics",\n'
        "        disposition: SweepDisposition::RefusalHalts,",
        "sweep_triage",
    ),
    (
        "the served universe silently loses GrPdbProperties",
        "crates/kayfabe-device/src/inittables.rs",
        "        Self::GrPdbProperties,\n        Self::GvaspaceServerReservedPdes,",
        "        Self::GvaspaceServerReservedPdes,",
        "sweep_triage",
    ),
]


def run(target: str) -> bool:
    """True if the target is GREEN."""
    p = subprocess.run(
        ["cargo", "test", "--workspace", "--no-fail-fast", "--test", target],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    return p.returncode == 0


def write(path: str, text: str) -> None:
    with open(path, "w") as f:
        f.write(text)
    # ⚠ Load-bearing: cargo keys on mtime, and a restore that preserves it serves a stale
    # rlib to the NEXT bite.
    os.utime(path, None)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()
    if args.list:
        for i, (name, _, _, _, t) in enumerate(BITES, 1):
            print(f"{i:2}. [{t}] {name}")
        return 0

    fails = 0
    for i, (name, rel, old, new, target) in enumerate(BITES, 1):
        path = os.path.join(REPO, rel)
        orig = open(path).read()
        if orig.count(old) != 1:
            print(f"{i:2}. ✗ ANCHOR MISSING ({orig.count(old)} matches) — {name}")
            print("      ⊘ A bite whose anchor moved measures NOTHING and must not be")
            print("        reported as a biter. Re-anchor it against the committed file.")
            fails += 1
            continue
        try:
            write(path, orig.replace(old, new))
            green = run(target)
        finally:
            write(path, orig)
        if green:
            print(f"{i:2}. ✗ NON-BITER [{target}] {name}")
            fails += 1
        else:
            print(f"{i:2}. ✓ bites     [{target}] {name}")
    print(f"\n{len(BITES) - fails}/{len(BITES)} bite")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
