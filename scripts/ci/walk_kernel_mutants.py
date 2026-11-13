#!/usr/bin/env python3
"""Walk-kernel source mutants, graded GPU-free by kf_cpu_walk (STATUS: LIVE, 2026-11-13).

Each mutant rewrites one anchor of a scratch copy of `cuda/walk/kf_walk.cu`, rebuilds
`kf_cpu_walk` there with g++ and runs it. A mutant is killed when at least two DISTINCT checks
(`FAIL kf_cpu_walk.cpp:<line>`) fail. Refuses a missing or repeated anchor, a red baseline, or a
build failure of a mutant (a mutant that does not compile proves nothing).

Only checks the CPU harness can reach are listed here (the serial walk and the decode it shares with
the parallel walk). The warp-parallel kernels' checks are graded by `kf_tests` on a GPU.
"""

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MUTANTS = [
    # review 3 F-1 (S1-60): VER3 "no valid 4 KiB page below this big PTE"
    ("ver3-big-pte-veto-removed",
     "        return kf_field(e, F.pcf.lo, F.pcf.bits, 0) == (uint64_t)KF_VER3_PTE_PCF_NO_VALID_4KB_PAGE;",
     "        return false;"),
    ("ver3-big-pte-veto-too-wide",
     "        return kf_field(e, F.pcf.lo, F.pcf.bits, 0) == (uint64_t)KF_VER3_PTE_PCF_NO_VALID_4KB_PAGE;",
     "        return kf_field(e, F.pcf.lo, F.pcf.bits, 0) >= 2ull;"),
    ("ver3-big-pte-veto-on-sparse",
     "        return kf_field(e, F.pcf.lo, F.pcf.bits, 0) == (uint64_t)KF_VER3_PTE_PCF_NO_VALID_4KB_PAGE;",
     "        return kf_field(e, F.pcf.lo, F.pcf.bits, 0) == (uint64_t)KF_VER3_PTE_PCF_NO_VALID_4KB_PAGE ||\n"
     "               kf_field(e, F.pcf.lo, F.pcf.bits, 0) == 1ull;"),
    # review 3 F-1 (S1-67): a VER3 PDE is sparse on PCF bit 0, whatever its ATS bit
    ("ver3-pde-sparse-as-pte-equality",
     "        return (kf_field(raw, F.pcf.lo, F.pcf.bits, 0) & KF_VER3_PDE_PCF_SPARSE_BIT) != 0ull;",
     "        return kf_field(raw, F.pcf.lo, F.pcf.bits, 0) == (uint64_t)F.pcf_sparse;"),
    ("ver3-pde-sparse-on-ats-bit",
     "        return (kf_field(raw, F.pcf.lo, F.pcf.bits, 0) & KF_VER3_PDE_PCF_SPARSE_BIT) != 0ull;",
     "        return (kf_field(raw, F.pcf.lo, F.pcf.bits, 0) & 2ull) != 0ull;"),
    # the VER2 arms the change must leave alone
    ("ver2-unmapped-veto-removed",
     "        if (kf_pte_sparse(F, e)) return false;\n        return ((e >> F.bit_privilege) & 1ull) != 0ull;",
     "        return false;"),
    ("ver2-unmapped-veto-ignores-sparse",
     "        if (kf_pte_sparse(F, e)) return false;\n        return ((e >> F.bit_privilege) & 1ull) != 0ull;",
     "        return ((e >> F.bit_privilege) & 1ull) != 0ull;"),
    ("ver2-pde-sparse-removed",
     "    case KF_TBL_VER2:\n        return ((raw >> F.bit_volatile) & 1ull) != 0ull;\n    default:\n        return (kf_field(raw, F.pcf.lo, F.pcf.bits, 0) & KF_VER3_PDE_PCF_SPARSE_BIT) != 0ull;",
     "    case KF_TBL_VER2:\n        return false;\n    default:\n        return (kf_field(raw, F.pcf.lo, F.pcf.bits, 0) & KF_VER3_PDE_PCF_SPARSE_BIT) != 0ull;"),
]
FAIL = re.compile(r"FAIL kf_cpu_walk\.cpp:(\d+)")


def build_and_run(d: Path):
    b = subprocess.run(["make", "-s", "-C", str(d), "kf_cpu_walk"], capture_output=True, text=True, timeout=600)
    if b.returncode:
        return None, b.stderr[-600:]
    r = subprocess.run(["./kf_cpu_walk"], cwd=d, capture_output=True, text=True, timeout=600)
    return sorted(set(FAIL.findall(r.stdout))), r.stdout


def main():
    with tempfile.TemporaryDirectory(prefix="kf-walk-mutants-") as td:
        d = Path(td) / "walk"
        shutil.copytree(ROOT / "cuda/walk", d, ignore=shutil.ignore_patterns("kf_cpu_walk", "kf_tests*", "evidence"))
        src = (d / "kf_walk.cu").read_text()
        base, out = build_and_run(d)
        if base is None or base:
            print(f"baseline is not green: {base} {out[-400:]}")
            return 1
        bad = 0
        for name, old, new in MUTANTS:
            if src.count(old) != 1:
                print(f"FAIL {name}: anchor found {src.count(old)} times")
                bad += 1
                continue
            (d / "kf_walk.cu").write_text(src.replace(old, new))
            (d / "kf_cpu_walk").unlink(missing_ok=True)
            killers, out = build_and_run(d)
            if killers is None:
                print(f"FAIL {name}: the mutant does not build: {out}")
                bad += 1
                continue
            ok = len(killers) >= 2
            bad += 0 if ok else 1
            print(f"{'ok  ' if ok else 'FAIL'} {name}: {len(killers)} distinct failing checks (lines {', '.join(killers)})")
        (d / "kf_walk.cu").write_text(src)
    print(f"walk kernel mutants: {len(MUTANTS) - bad}/{len(MUTANTS)} with >= 2 killers")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
