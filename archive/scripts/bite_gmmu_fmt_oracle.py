#!/usr/bin/env python3
"""Bite harness for the GMMU format oracle.

Plant a defect in **our** decoder (`crates/kayfabe-chips/src/ga10x.rs`), run the oracle
suite, and require it to go RED. A guard nobody has watched fail is not a guard.

★★ It also runs the crate's OWN unit tests (`crates/kayfabe-chips/tests/ga10x_gmmu.rs`)
for every bite, and reports which bites **only the oracle** catches. That number is the
honest measure of what the oracle adds over the transcription it was built to replace —
and if it came out zero, that would be a real (negative) result worth stating plainly.

Usage:
    scripts/bite_gmmu_fmt_oracle.py [--only N] [--list]

Every bite is applied to the working tree and restored afterwards, and the file's mtime is
touched after restoring — `shutil`/`cp -a` preserve mtime, cargo then serves a stale rlib,
and the next bite is measured against the previous one's binary.
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = os.path.join(ROOT, "crates/kayfabe-chips/src/ga10x.rs")

ORACLE_TEST = ["cargo", "test", "-q", "--test", "gmmu_fmt_oracle"]
OWN_TEST = ["cargo", "test", "-q", "-p", "kayfabe-chips", "--test", "ga10x_gmmu"]

# (name, old, new) — `old` must appear EXACTLY ONCE in the file.
BITES = [
    # ---- the two GA10x traps that cost weeks ----
    (
        "PD1 is not a 512 MiB leaf level (#13, verbatim)",
        "L_PD1 => ver2_leaf(lo, PageSize(512 << 20)).unwrap_or_else(|| ver2_pde(lo, L_PD0)),",
        "L_PD1 => ver2_pde(lo, L_PD0),",
    ),
    (
        "the PD1 leaf is sized 2 MiB instead of 512 MiB",
        "L_PD1 => ver2_leaf(lo, PageSize(512 << 20)).unwrap_or_else(|| ver2_pde(lo, L_PD0)),",
        "L_PD1 => ver2_leaf(lo, PageSize(2 << 20)).unwrap_or_else(|| ver2_pde(lo, L_PD0)),",
    ),
    (
        "the dual PD0 entry drops its BIG half",
        "                    (Some(e), also) => PteDecode::Pde { edge: e, also },",
        "                    (Some(e), _) => PteDecode::Pde { edge: e, also: None },",
    ),
    (
        "the dual PD0 entry drops its SMALL half",
        """                match (small, big) {
                    (None, None) => ver2_empty(lo),
                    (Some(e), also) => PteDecode::Pde { edge: e, also },""",
        """                match (big, small) {
                    (None, None) => ver2_empty(lo),
                    (Some(e), also) => PteDecode::Pde { edge: e, also },""",
    ),
    (
        "the BIG half of a dual PDE uses the SMALL half's shift (12, not 8)",
        "const VER2_BIG_ADDR_SHIFT: u32 = 8;",
        "const VER2_BIG_ADDR_SHIFT: u32 = 12;",
    ),
    (
        "the BIG half reads the SMALL half's word",
        """                let big = pde_aperture(lo).map(|aperture| PdeEdge {
                    next: field_addr(
                        lo,""",
        """                let big = pde_aperture(hi).map(|aperture| PdeEdge {
                    next: field_addr(
                        hi,""",
    ),
    # ---- the aperture tables are not one table ----
    (
        "the PDE aperture table is used for PTEs",
        """fn pte_aperture(raw: u64) -> Aperture {
    match (raw >> VER2_APERTURE_SHIFT) & VER2_APERTURE_MASK {
        0 => Aperture::Vidmem,
        1 => Aperture::Peer,""",
        """fn pte_aperture(raw: u64) -> Aperture {
    match (raw >> VER2_APERTURE_SHIFT) & VER2_APERTURE_MASK {
        0 => Aperture::Peer,
        1 => Aperture::Vidmem,""",
    ),
    (
        "a PDE aperture of 0 is read as video memory rather than 'absent'",
        """        0 => None,
        1 => Some(Aperture::Vidmem),""",
        """        0 => Some(Aperture::Vidmem),
        1 => Some(Aperture::Vidmem),""",
    ),
    # ---- address fields ----
    (
        "the vidmem address field is one bit too wide",
        "const VER2_ADDR_VID_BITS: u32 = 25;",
        "const VER2_ADDR_VID_BITS: u32 = 26;",
    ),
    (
        "the sysmem address field is used for vidmem too",
        """        Aperture::Vidmem | Aperture::Peer => VER2_ADDR_VID_BITS,
        Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => VER2_ADDR_SYS_BITS,
    };
    field_addr(raw, 8, bits, VER2_ADDR_SHIFT)""",
        """        Aperture::Vidmem | Aperture::Peer => VER2_ADDR_SYS_BITS,
        Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => VER2_ADDR_SYS_BITS,
    };
    field_addr(raw, 8, bits, VER2_ADDR_SHIFT)""",
    ),
    (
        "the address shift is 16 rather than 12",
        "const VER2_ADDR_SHIFT: u32 = 12;",
        "const VER2_ADDR_SHIFT: u32 = 16;",
    ),
    (
        "the address field starts at bit 4 rather than bit 8",
        "    field_addr(raw, 8, bits, VER2_ADDR_SHIFT)",
        "    field_addr(raw, 4, bits, VER2_ADDR_SHIFT)",
    ),
    # ---- the flag bits ----
    (
        "SPARSE is read off the wrong bit (4, not 3)",
        "const VER2_VOL: u64 = 1 << 3;",
        "const VER2_VOL: u64 = 1 << 4;",
    ),
    (
        "sparse and invalid are conflated",
        """const fn ver2_empty(raw: u64) -> PteDecode {
    if raw & VER2_VOL != 0 {
        PteDecode::Sparse
    } else {
        PteDecode::Invalid
    }
}""",
        """const fn ver2_empty(raw: u64) -> PteDecode {
    let _ = raw;
    PteDecode::Invalid
}""",
    ),
    (
        "the read-only bit is read off bit 5",
        "const VER2_READ_ONLY: u64 = 1 << 6;",
        "const VER2_READ_ONLY: u64 = 1 << 5;",
    ),
    (
        "the valid bit is bit 1",
        "const VER2_VALID: u64 = 1 << 0;",
        "const VER2_VALID: u64 = 1 << 1;",
    ),
    (
        "the aperture field is at bits 3:2",
        "const VER2_APERTURE_SHIFT: u32 = 1;",
        "const VER2_APERTURE_SHIFT: u32 = 2;",
    ),
    # ---- the geometry ----
    (
        "PT_BIG is given 512 entries instead of 32 (the 3 840-byte over-read)",
        "L_PT_BIG => (16, 32),    // 20:16",
        "L_PT_BIG => (16, 512),   // 20:16",
    ),
    (
        "the root directory is given 512 entries instead of 4",
        "L_PD3 => (47, 4),        // 48:47",
        "L_PD3 => (47, 512),      // 48:47",
    ),
    (
        "PD0's slot is 8 bytes, not 16",
        """            L_PD3 | L_PD2 | L_PD1 | L_PT_BIG | L_PT_SMALL => 8,
            L_PD0 => 16,""",
        """            L_PD3 | L_PD2 | L_PD1 | L_PT_BIG | L_PT_SMALL => 8,
            L_PD0 => 8,""",
    ),
    (
        "PD1's stride is 28 rather than 29",
        "L_PD1 => (29, 512),      // 37:29",
        "L_PD1 => (28, 512),      // 37:29",
    ),
    (
        "the walk is six levels deep, not five",
        """    fn levels(&self) -> u8 {
        5
    }""",
        """    fn levels(&self) -> u8 {
        6
    }""",
    ),
    (
        "512 MiB is dropped from the page-size enumeration",
        """    PageSize(2 << 20),
    PageSize(512 << 20),
];""",
        """    PageSize(2 << 20),
];""",
    ),
    # ---- the leaf sizes ----
    (
        "the big-page leaf is 4 KiB",
        "L_PT_BIG => ver2_leaf(lo, PageSize(64 << 10)).unwrap_or_else(|| ver2_empty(lo)),",
        "L_PT_BIG => ver2_leaf(lo, PageSize(4 << 10)).unwrap_or_else(|| ver2_empty(lo)),",
    ),
    (
        "PD0's 2 MiB leaf is never recognised",
        """                if let Some(leaf) = ver2_leaf(lo, PageSize(2 << 20)) {
                    return leaf;
                }""",
        "",
    ),
    (
        "the small and big page tables are swapped in the child-level edges",
        "                    child_level: L_PT_SMALL,",
        "                    child_level: L_PT_BIG,",
    ),
    # ---- ★ the FIELD-WIDTH class, probed deliberately ----
    # Bites 8 and 9 were the only two the crate's own tests missed, and both are field
    # widths. That is not a coincidence: a width is a number copied out of `dev_mmu.h` into
    # the decoder and then copied into the test's expected value from the same reading, so
    # the two agree by construction. These four extend the probe across every remaining
    # width the format has.
    (
        "the sysmem address field is 45 bits, not 46",
        "const VER2_ADDR_SYS_BITS: u32 = 46;",
        "const VER2_ADDR_SYS_BITS: u32 = 45;",
    ),
    (
        "the dual-PDE BIG vidmem address field is 30 bits, not 29",
        "const VER2_BIG_ADDR_VID_BITS: u32 = 29;",
        "const VER2_BIG_ADDR_VID_BITS: u32 = 30;",
    ),
    (
        "the dual-PDE BIG sysmem address field is 46 bits, not 50",
        "const VER2_BIG_ADDR_SYS_BITS: u32 = 50;",
        "const VER2_BIG_ADDR_SYS_BITS: u32 = 46;",
    ),
    (
        "the dual-PDE BIG address field starts at bit 8, not bit 4",
        """                let big = pde_aperture(lo).map(|aperture| PdeEdge {
                    next: field_addr(
                        lo,
                        4,""",
        """                let big = pde_aperture(lo).map(|aperture| PdeEdge {
                    next: field_addr(
                        lo,
                        8,""",
    ),
]


def run(cmd, env):
    return subprocess.run(
        cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", type=int, default=None)
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    if args.list:
        for i, (name, _, _) in enumerate(BITES):
            print(f"{i:2d}  {name}")
        return 0

    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", os.environ.get("CARGO_TARGET_DIR", ""))
    if not env["CARGO_TARGET_DIR"]:
        del env["CARGO_TARGET_DIR"]

    original = open(TARGET, encoding="utf-8").read()

    # The clean tree must be GREEN, or a red bite proves nothing.
    print("== baseline ==", flush=True)
    base_oracle = run(ORACLE_TEST, env)
    base_own = run(OWN_TEST, env)
    if base_oracle.returncode != 0 or base_own.returncode != 0:
        print("BASELINE IS NOT GREEN — every bite below would be meaningless.")
        print(base_oracle.stdout.decode()[-3000:])
        print(base_own.stdout.decode()[-3000:])
        return 2
    print("baseline: oracle GREEN, kayfabe-chips own tests GREEN\n", flush=True)

    results = []
    todo = range(len(BITES)) if args.only is None else [args.only]
    try:
        for i in todo:
            name, old, new = BITES[i]
            n = original.count(old)
            if n != 1:
                print(f"{i:2d}  {name}\n    ★ ANCHOR MATCHED {n} TIMES — bite not applied")
                results.append((i, name, None, None))
                continue
            with open(TARGET, "w", encoding="utf-8") as f:
                f.write(original.replace(old, new))
            os.utime(TARGET, None)
            time.sleep(0.01)
            o = run(ORACLE_TEST, env)
            w = run(OWN_TEST, env)
            oracle_red = o.returncode != 0
            own_red = w.returncode != 0
            mark = "RED " if oracle_red else "GREEN"
            only = " <== ONLY THE ORACLE" if oracle_red and not own_red else ""
            miss = "  ★★ MISSED BY BOTH" if not oracle_red and not own_red else ""
            print(
                f"{i:2d}  oracle={mark} own={'RED ' if own_red else 'GREEN'}  "
                f"{name}{only}{miss}",
                flush=True,
            )
            results.append((i, name, oracle_red, own_red))
    finally:
        with open(TARGET, "w", encoding="utf-8") as f:
            f.write(original)
        os.utime(TARGET, None)

    caught = [r for r in results if r[2]]
    only_oracle = [r for r in results if r[2] and r[3] is False]
    missed = [r for r in results if r[2] is False]
    print(
        f"\n{len(caught)}/{len(results)} bites caught by the ORACLE; "
        f"{len(only_oracle)} caught by the ORACLE ALONE; {len(missed)} missed by it."
    )
    for i, name, _, _ in missed:
        print(f"  MISSED: {i:2d} {name}")
    return 1 if missed else 0


if __name__ == "__main__":
    sys.exit(main())
