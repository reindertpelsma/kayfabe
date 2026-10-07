#!/usr/bin/env python3
"""Bite harness for the BAR0 moving window (`#146`).

★★★ Every mutation here is chosen to **COMPILE**. A mutation that breaks compilation is not
a bite — the test never ran, so nothing was shown to be load-bearing. The harness reports
`COMPILE-FAIL` as a distinct, *failing* outcome rather than folding it into "bit".

★★ Files are restored with an explicit `os.utime` bump. `shutil.copy2`/`cp -a` preserve the
OLD mtime, so cargo serves a stale rlib and every later mutation reads as a non-biter —
measured twice on 2026-07-28.

★ What these mutations are *for*. `docs/design/boot_measured_2026_08_01.md` §18 records that
`kbusInitBar2` programs this window and **never reads any of it back**, so a window that
silently drops writes is caught only at `kbusVerifyBar2`, hundreds of operations later. Each
mutation below is one of the ways a write is lost, and the test it must kill is the one that
states that mechanism — checked by NAME, not by exit status, because a mutation that turns
some other test red is not evidence about the behaviour it was planted to probe.

★ **Mutation strings must not carry leading indentation.** `cargo fmt` collapsed a
three-line `match` arm to one line between two runs of this harness, and the mutation that
had bitten an hour earlier came back `NOT-PLANTABLE (0 matches)`. The harness reported that
distinctly rather than as a survivor, which is the only reason it took one step to see —
`suspect_the_instrument_first`, seventh instance. Anchor on the distinctive expression, not
on a whole formatted line.

Usage:  python3 scripts/bite_bar0_window.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

FBWIN = "crates/kayfabe-device/src/fbwin.rs"
PLANE = "crates/kayfabe-device/src/plane.rs"
LIB = "crates/kayfabe-device/src/lib.rs"
SHIM = "crates/kayfabe-qemu-raw/src/shim.rs"

# (name, file, old, new, test-target, expected-test)
MUTATIONS = [
    # ── 1. The window register stops being a latch ────────────────────────────────────
    (
        "the window register answers a defaulted zero (not a latch)",
        PLANE,
        "            return ReadOutcome::Bar0Window(mask(u64::from(s.bar0_window.raw()), size));",
        "            return ReadOutcome::Bar0Window(mask(0, size));",
        "bar0_window",
        "the_window_register_read_modify_write_composes_because_it_is_a_latch",
    ),
    (
        "the window register write is dropped",
        PLANE,
        "            s.bar0_window.set_raw(val as u32);",
        "            let _ = (&mut s.bar0_window, val);",
        "bar0_window",
        "the_verify_bar0_window_subtest_passes_at_the_address_the_boot_measured",
    ),
    (
        "the window register keeps only the fields this port decodes",
        FBWIN,
        "    pub fn set_raw(&mut self, raw: u32) {\n        self.raw = raw;\n    }",
        "    pub fn set_raw(&mut self, raw: u32) {\n        self.raw = raw & (BASE_MASK | (TARGET_MASK << TARGET_SHIFT));\n    }",
        "bar0_window",
        "a_reserved_bit_this_port_does_not_decode_still_reads_back",
    ),
    # ── 2. The one address function is wrong ──────────────────────────────────────────
    (
        "the window offset is OR-ed into the origin instead of added",
        FBWIN,
        "        (u64::from(self.base()) << BASE_SHIFT).saturating_add(window_off)",
        "        (u64::from(self.base()) << BASE_SHIFT) | window_off",
        "bar0_window",
        "the_window_offset_is_added_to_the_origin_and_not_or_ed_into_it",
    ),
    (
        "the BASE field is masked to 20 bits instead of 24",
        FBWIN,
        "const BASE_MASK: u32 = 0x00FF_FFFF;",
        "const BASE_MASK: u32 = 0x000F_FFFF;",
        "bar0_window",
        "the_base_field_is_twenty_four_bits_wide_and_none_of_it_is_truncated",
    ),
    (
        "the window address is computed in 32 bits",
        FBWIN,
        "        (u64::from(self.base()) << BASE_SHIFT).saturating_add(window_off)",
        "        u64::from((self.base() << BASE_SHIFT).wrapping_add(window_off as u32))",
        "bar0_window",
        "the_base_field_is_twenty_four_bits_wide_and_none_of_it_is_truncated",
    ),
    (
        "the window offset is measured from BAR0 zero, not the window's base",
        PLANE,
        "FbWindow::Pramin => Some(s.bar0_window.fb_addr(off - self.chip.pramin_window.base)),",
        "FbWindow::Pramin => Some(s.bar0_window.fb_addr(off)),",
        "bar0_window",
        "the_verify_bar0_window_subtest_passes_at_the_address_the_boot_measured",
    ),
    # ── 3. A write is dropped, or half-applied ────────────────────────────────────────
    (
        "a refused framebuffer write reports success",
        PLANE,
        "            Err(e) => {\n                self.c.fb_refusals.fetch_add(1, Ordering::Relaxed);",
        "            Err(e) => {\n                let _ = &e;\n                return WriteOutcome {\n                    fb_landed: Some(phys),\n                    ..WriteOutcome::nothing()\n                };\n            }\n            #[allow(unreachable_patterns)]\n            Err(e) => {\n                self.c.fb_refusals.fetch_add(1, Ordering::Relaxed);",
        "bar0_window",
        "a_framebuffer_write_with_no_store_refuses_by_name_and_never_reports_success",
    ),
    (
        "the store's page geometry changes under both sides at once",
        FBWIN,
        "            let frame = at / FB_PAGE;",
        "            let frame = at / FB_PAGE / 2;",
        "bar0_window",
        "a_dword_written_through_the_window_reads_back_at_every_base_and_offset",
    ),
    (
        "the READ side resolves a different page from the WRITE side",
        FBWIN,
        "            match self.pages.get(&frame) {",
        "            match self.pages.get(&(frame ^ 1)) {",
        "bar0_window",
        "a_dword_written_through_the_window_reads_back_at_every_base_and_offset",
    ),
    (
        "an out-of-framebuffer address is masked into range instead of refused",
        FBWIN,
        "        match phys.checked_add(len) {\n            Some(end) => end <= self.limit,\n            None => false,\n        }",
        "        let _ = len;\n        phys < self.limit || true",
        "bar0_window",
        "an_address_past_the_advertised_framebuffer_refuses_instead_of_wrapping",
    ),
    (
        "the residency ceiling is checked per page instead of for the whole access",
        FBWIN,
        "        if self.resident_bytes() + fresh * FB_PAGE > self.cap {",
        "        if self.resident_bytes() + FB_PAGE.min(fresh * FB_PAGE) > self.cap {",
        "bar0_window",
        "a_straddling_write_at_the_ceiling_is_all_or_nothing",
    ),
    (
        "a read allocates the page it touched",
        FBWIN,
        "                None => buf[done..done + take].fill(0),",
        "                None => {\n                    self.pages\n                        .entry(frame)\n                        .or_insert_with(|| Box::new([0u8; FB_PAGE as usize]));\n                    buf[done..done + take].fill(0);\n                }",
        "bar0_window",
        "an_unwritten_framebuffer_address_reads_zero_rather_than_refusing",
    ),
    # ── 4. Lifetime and wiring ────────────────────────────────────────────────────────
    (
        "a device reset keeps the previous guest's framebuffer",
        PLANE,
        "        s.fb.device_reset();",
        "        let _ = &s.fb;",
        "bar0_window",
        "a_device_reset_forgets_the_framebuffer_and_re_points_the_window",
    ),
    (
        "a device reset leaves the window where the last guest pointed it",
        PLANE,
        "        s.bar0_window = Bar0Window::new();",
        "        let _ = &s.bar0_window;",
        "bar0_window",
        "a_device_reset_forgets_the_framebuffer_and_re_points_the_window",
    ),
    (
        "a half-filled chip row (window, no register) is accepted at realize",
        PLANE,
        "    if (chip.pramin_window.len == 0) != (chip.bar0_window_reg == 0) {",
        "    if false {",
        "bar0_window",
        "a_chip_with_a_window_and_no_register_to_move_it_is_refused_at_realize",
    ),
    (
        "the window register is not checked against the other read sources",
        PLANE,
        "        if model.decode_reg(0, off).is_some() {\n            return Err(ChipError::OverlappingSources {\n                off,\n                a: \"the BAR0 moving window's own register\",\n                b: \"the GSP register model\",\n            });\n        }",
        "        if false {\n            return Err(ChipError::OverlappingSources {\n                off,\n                a: \"the BAR0 moving window's own register\",\n                b: \"the GSP register model\",\n            });\n        }",
        "bar0_window",
        "a_window_register_placed_over_another_source_is_refused_at_realize",
    ),
    (
        "the shell's store is sized from something other than the chip's fb_length",
        SHIM,
        "        plane.set_fb(Box::new(kayfabe_device::SparseFb::new(chip.fb_length)));",
        "        plane.set_fb(Box::new(kayfabe_device::SparseFb::new(chip.fb_length / 2)));",
        "shim_logic",
        "the_register_plane_answers_through_the_seam_the_c_shim_calls",
    ),
    (
        "the composition root never installs a framebuffer at all",
        SHIM,
        "        plane.set_fb(Box::new(kayfabe_device::SparseFb::new(chip.fb_length)));",
        "        let _ = chip.fb_length;",
        "shim_logic",
        "the_register_plane_answers_through_the_seam_the_c_shim_calls",
    ),
    (
        "the chip row's window register offset is ignored",
        LIB,
        "    pub bar0_window_reg: u64,",
        "    pub bar0_window_reg: u64,\n    #[doc(hidden)]\n    pub _mutant: (),",
        "bar0_window",
        "COMPILE-FAIL-EXPECTED",
    ),
]

# ⊘ The last row is deliberately a compile-breaker and is DROPPED, not run: it exists only
# as the worked example of what this harness refuses to count. Keeping it in the list as a
# comment would rot; keeping it here and filtering it means the rule is executable.
MUTATIONS = [m for m in MUTATIONS if m[5] != "COMPILE-FAIL-EXPECTED"]


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
        pkg = "kayfabe-qemu-raw" if t == "shim_logic" else "kayfabe-device"
        r = run(["cargo", "test", "-p", pkg, "--test", t, "--no-fail-fast"], env)
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
            pkg = "kayfabe-qemu-raw" if target == "shim_logic" else "kayfabe-device"
            r = run(
                ["cargo", "test", "-p", pkg, "--test", target, "--no-fail-fast"], env
            )
            out = r.stdout + r.stderr
            if "error[E" in out or "error: could not compile" in out:
                print(f"COMPILE-FAIL   {name}  ⊘ NOT A BITE — the test never ran")
                compile_fail.append(name)
            elif r.returncode == 0:
                print(f"SURVIVED       {name}")
                survived.append(name)
            elif f"{expect}" in out and f"---- {expect}" in out:
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
