#!/usr/bin/env python3
"""Bite harness for the first GMMU translation (`#149`).

★★★ Every mutation here is chosen to **COMPILE**. A mutation that breaks compilation is not
a bite — the test never ran, so nothing was shown to be load-bearing. The harness reports
`COMPILE-FAIL` as a distinct, *failing* outcome rather than folding it into "bit".

★★ Files are restored with an explicit `os.utime` bump. `shutil.copy2`/`cp -a` preserve the
OLD mtime, so cargo serves a stale rlib and every later mutation reads as a non-biter —
measured twice on 2026-07-28.

★ **Mutation strings must not carry leading indentation that `cargo fmt` can move.** A
three-line `match` arm was collapsed to one line between two runs of the `#146` harness and
the mutation that had bitten an hour earlier came back `NOT-PLANTABLE (0 matches)`. That is
reported distinctly here for the same reason: a non-plantable mutation is a broken
instrument, not a survivor.

★ What these mutations are *for*. Boot `l2evict1` (2026-08-01, rev `9551dd1`) failed at
`kbusVerifyBar2_GM107`'s MMU sub-test: sixteen bytes written through the translated BAR2
aperture read back as `0x0` through the untranslated BAR0 window. Every mutation below is
one of the ways that write is lost again — a wrong entry width, a dropped sub-table, a
guessed level, a wrong aperture table, a root nobody latched — and each is checked against
the test that states *that mechanism*, by NAME, because a mutation that turns some other
test red is not evidence about the behaviour it was planted to probe.

Usage:  python3 scripts/bite_bar2_translation.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

CHIPS = "crates/kayfabe-chips/src/ga10x.rs"
WALKER = "crates/kayfabe-mmu/src/walker.rs"
PLANE = "crates/kayfabe-device/src/plane.rs"
BAR2 = "crates/kayfabe-device/src/bar2.rs"
SHIM = "crates/kayfabe-qemu-raw/src/shim.rs"

# Which package each test target lives in.
PKG = {
    "ga10x_gmmu": "kayfabe-chips",
    "bar2_translation": "kayfabe-tests",
    "shim_logic": "kayfabe-qemu-raw",
}

# (name, file, old, new, test-target, expected-test)
MUTATIONS = [
    # ── 1. The GA10x format: the two traps that cost weeks ────────────────────────────
    (
        "the dual PD0 entry's SECOND sub-table is dropped (#13, one level up)",
        CHIPS,
        "(Some(e), also) => PteDecode::Pde { edge: e, also },",
        "(Some(e), _also) => PteDecode::Pde { edge: e, also: None },",
        "ga10x_gmmu",
        "a_dual_pd0_entry_names_two_sub_tables_at_two_different_shifts",
    ),
    (
        "the BIG half of a dual entry is decoded with the SMALL half's shift",
        CHIPS,
        "const VER2_BIG_ADDR_SHIFT: u32 = 8;",
        "const VER2_BIG_ADDR_SHIFT: u32 = 12;",
        "ga10x_gmmu",
        "a_dual_pd0_entry_names_two_sub_tables_at_two_different_shifts",
    ),
    (
        "PD1 stops being a leaf level — the GA10x 512 MiB gap, restored",
        CHIPS,
        "L_PD1 => ver2_leaf(lo, PageSize(512 << 20)).unwrap_or_else(|| ver2_pde(lo, L_PD0)),",
        "L_PD1 => ver2_pde(lo, L_PD0),",
        "ga10x_gmmu",
        "a_pd1_slot_with_the_valid_bit_is_a_512_mib_leaf",
    ),
    (
        "the big-page table's entry COUNT is derived rather than read (32 -> 512)",
        CHIPS,
        "L_PT_BIG => (16, 32),",
        "L_PT_BIG => (16, 512),",
        "ga10x_gmmu",
        "the_level_geometry_is_the_drivers_own_bit_ranges",
    ),
    (
        "the PDE aperture table is shared with the PTE one (0 means VIDEO)",
        CHIPS,
        "        0 => None,\n        1 => Some(Aperture::Vidmem),",
        "        0 => Some(Aperture::Vidmem),\n        1 => Some(Aperture::Vidmem),",
        "ga10x_gmmu",
        "the_pde_aperture_table_and_the_pte_aperture_table_are_different_tables",
    ),
    (
        "SPARSE is folded into INVALID — the declaration disappears",
        CHIPS,
        "    if raw & VER2_VOL != 0 {\n        PteDecode::Sparse\n    } else {\n        PteDecode::Invalid\n    }",
        "    let _ = raw & VER2_VOL;\n    PteDecode::Invalid",
        "ga10x_gmmu",
        "an_empty_slot_is_sparse_when_volatile_is_set_and_invalid_when_it_is_not",
    ),
    (
        "the guest's read-only bit is dropped in the decode",
        CHIPS,
        "        read_only: raw & VER2_READ_ONLY != 0,",
        "        read_only: false,",
        "ga10x_gmmu",
        "the_guests_read_only_bit_survives_the_decode",
    ),
    # ── 2. The point query — the walk itself ──────────────────────────────────────────
    (
        "the offset within the leaf page is not added",
        WALKER,
        "                phys: phys.saturating_add(va & (size.0 - 1)),",
        "                phys,",
        "bar2_translation",
        "the_offset_within_the_leaf_page_comes_from_the_leafs_own_size",
    ),
    (
        "the descent follows only the FIRST half of a dual slot",
        WALKER,
        "            for e in [Some(edge), also].into_iter().flatten() {\n                // A null sub-table pointer is not a sub-table",
        "            for e in [Some(edge)].into_iter().flatten() {\n                let _ = also;\n                // A null sub-table pointer is not a sub-table",
        "bar2_translation",
        "a_big_page_mapping_under_the_dual_entrys_second_sub_table_resolves",
    ),
    (
        "a SPARSE declaration is reported as an ordinary miss",
        WALKER,
        "        PteDecode::Sparse => Err(TranslateFault::Sparse { va, level }),",
        "        PteDecode::Sparse => Err(TranslateFault::Unmapped { va, level }),",
        "bar2_translation",
        "a_sparse_declaration_is_reported_as_a_declaration",
    ),
    # ── 3. The plane's refusals — every one a way the write is lost silently ──────────
    (
        "a system-memory leaf is answered out of the framebuffer store",
        PLANE,
        "        if t.aperture != Aperture::Vidmem {",
        "        if false {",
        "bar2_translation",
        "a_system_memory_leaf_is_refused_and_not_answered_out_of_the_framebuffer",
    ),
    (
        "an address outside the ONE published root slot is resolved anyway",
        PLANE,
        "        if geo.entries == 0 || (va >> geo.shift) & u64::from(geo.entries.saturating_sub(1)) != 0 {",
        "        if geo.entries == 0 {",
        "bar2_translation",
        "an_address_outside_the_one_published_root_slot_is_refused",
    ),
    (
        "a write to a read-only leaf is let through",
        PLANE,
        "        if write && t.read_only {",
        "        if false {",
        "bar2_translation",
        "a_write_to_a_read_only_leaf_is_refused_and_a_read_of_it_is_not",
    ),
    (
        "the root's level is ASSUMED to be zero rather than derived from its shift",
        PLANE,
        "            .find(|(_, g)| u64::from(g.shift) == root.level_shift)",
        "            .find(|(l, _)| *l == 0)",
        "bar2_translation",
        "a_root_published_at_a_level_shift_this_format_does_not_have_is_refused",
    ),
    (
        "a device reset keeps the previous guest's page-table root",
        PLANE,
        "        self.bar_pdes.device_reset();",
        "        let _ = &self.bar_pdes;",
        "bar2_translation",
        "a_device_reset_forgets_the_published_root",
    ),
    # ── 4. The publication itself ─────────────────────────────────────────────────────
    (
        "entryValue is read at offset 4 — the alignment padding trap",
        BAR2,
        "const ENTRY_VALUE_OFF: usize = 8;",
        "const ENTRY_VALUE_OFF: usize = 4;",
        "bar2_translation",
        "the_mmu_subtest_agrees_about_every_byte_in_both_directions",
    ),
    (
        "barType is not checked, so a root is latched under an aperture nobody named",
        BAR2,
        "    if bar_type != BAR_TYPE_1 && bar_type != BAR_TYPE_2 {",
        "    if false {",
        "bar2_translation",
        "a_short_or_unknown_body_is_refused_and_publishes_nothing",
    ),
    (
        "a BAR1 root is latched as the BAR2 one",
        BAR2,
        "            BAR_TYPE_1 => p.bar1 = Some(pde),",
        "            BAR_TYPE_1 => p.bar2 = Some(pde),",
        "bar2_translation",
        "the_bar1_root_is_recorded_even_though_nothing_translates_that_window",
    ),
    # ── 5. The composition root ───────────────────────────────────────────────────────
    (
        "the composition root never installs a page-table format",
        SHIM,
        "        plane.set_mmu(Box::new(kayfabe_chips::Ga10xGmmu::new()));",
        "        let _ = kayfabe_chips::Ga10xGmmu::new();",
        "shim_logic",
        "the_register_plane_answers_through_the_seam_the_c_shim_calls",
    ),
]


def run(cmd, env, cwd=ROOT):
    return subprocess.run(
        cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=1800
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cargo-target", default=os.environ.get("CARGO_TARGET_DIR"))
    args = ap.parse_args()

    env = dict(os.environ)
    env["PATH"] = "/root/.cargo/bin:" + env.get("PATH", "")
    env["KAYFABE_NO_KVM"] = "1"
    if args.cargo_target:
        env["CARGO_TARGET_DIR"] = args.cargo_target

    # ★ Baseline first. A harness that never establishes green cannot tell a bite from a
    # tree that was already red.
    targets = sorted({m[4] for m in MUTATIONS})
    for t in targets:
        r = run(["cargo", "test", "-p", PKG[t], "--test", t, "--no-fail-fast"], env)
        if r.returncode != 0:
            print(f"BASELINE RED for {t}:\n{r.stdout[-4000:]}\n{r.stderr[-2000:]}")
            return 2
    print(f"baseline green: {', '.join(targets)}\n")

    bit = 0
    survived = []
    compile_fail = []
    for name, path, old, new, target, expect in MUTATIONS:
        full = os.path.join(ROOT, path)
        original = open(full).read()
        if original.count(old) != 1:
            print(f"NOT-PLANTABLE  {name}  ({path}: {original.count(old)} matches)")
            survived.append(name)
            continue
        open(full, "w").write(original.replace(old, new, 1))
        os.utime(full, (time.time(), time.time()))
        try:
            r = run(
                ["cargo", "test", "-p", PKG[target], "--test", target, "--no-fail-fast"],
                env,
            )
            out = r.stdout + r.stderr
            if "error[E" in out or "error: could not compile" in out:
                print(f"COMPILE-FAIL   {name}  ⊘ NOT A BITE — the test never ran")
                compile_fail.append(name)
            elif r.returncode == 0:
                print(f"SURVIVED       {name}")
                survived.append(name)
            elif f"---- {expect}" in out:
                print(f"BIT            {name}  → {expect}")
                bit += 1
            else:
                print(
                    f"WRONG-TEST     {name}  (red, but {expect} is not among the failures)"
                )
                survived.append(name)
        finally:
            open(full, "w").write(original)
            os.utime(full, (time.time(), time.time()))

    total = len(MUTATIONS)
    print(f"\nbit {bit}/{total}")
    if compile_fail:
        print("COMPILE-FAIL (not bites): " + "; ".join(compile_fail))
    if survived:
        print("SURVIVED: " + "; ".join(survived))
    return 0 if bit == total else 1


if __name__ == "__main__":
    sys.exit(main())
