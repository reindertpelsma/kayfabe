#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf-cuda security-fix mutants (security review 2, 2026-11-13): every new check must be killed by
at least two distinct tests, the repo's pattern (`scripts/ci/*_mutants.py`).

Each mutant names a file under crates/kf-cuda/src, an anchor that must occur exactly once, and its
replacement. The script refuses a red baseline, a missing or repeated anchor, a mutant that does not
compile (a compile error proves nothing about the tests), and fewer than two failing tests.

The mutation is applied IN PLACE and restored in `finally` (the file's bytes are checked after the
restore): a Rust crate inside a workspace does not build from a copied directory.

Run from anywhere: `python3 -B crates/kf-cuda/mutants/secfix_mutants.py [name-substring]`.
"""

import os
from pathlib import Path
import re
import subprocess
import sys

CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]

# (name, file relative to crates/kf-cuda/src, anchor, replacement)
MUTANTS = [
    # ── D-03: the frame a kernel writes is bounded by the allocation (display.rs) ──
    (
        "d03-length-check-dropped",
        "display.rs",
        "Some(n) if n > 0 && n <= len as u64 => Ok(buf.span(0, n as usize)),",
        "Some(n) if n > 0 => Ok(buf.whole()),",
    ),
    (
        "d03-empty-frame-accepted",
        "display.rs",
        "Some(n) if n > 0 && n <= len as u64 => Ok(buf.span(0, n as usize)),",
        "Some(n) if n <= len as u64 => Ok(buf.span(0, n as usize)),",
    ),
    (
        "d03-overflow-wraps",
        "display.rs",
        """    let need = u64::from(w)
        .checked_mul(u64::from(h))
        .and_then(|p| p.checked_mul(bpp));""",
        """    let need = Some(u64::from(w).wrapping_mul(u64::from(h)).wrapping_mul(bpp));""",
    ),
    (
        "d03-compose-discards-the-staging-length",
        "display.rs",
        "        let dst = frame_for(self.staging.as_ref(), w, h, 4, what)?;\n        let u = |x: u32| x.to_le_bytes().to_vec();\n        let i =",
        "        let dst = self.staging.as_ref().ok_or_else(|| refused(what, \"none\".into()))?.whole();\n        let u = |x: u32| x.to_le_bytes().to_vec();\n        let i =",
    ),
    (
        "d03-yuv-discards-the-staging-length",
        "display.rs",
        "        let dst = frame_for(self.staging.as_ref(), w, h, 4, what)?;\n        self.launch_yuv(f, dst, l, w, h, what)",
        "        let dst = self.staging.as_ref().ok_or_else(|| refused(what, \"none\".into()))?.whole();\n        self.launch_yuv(f, dst, l, w, h, what)",
    ),
    # ── D-11: caps (display.rs) ──
    (
        "d11-pixel-cap-dropped",
        "display.rs",
        "    if pixels == 0 || w > 16384 || h > 16384 || pixels > MAX_STAGING_PIXELS {",
        "    if pixels == 0 || w > 16384 || h > 16384 {",
    ),
    (
        "d11-slot-count-cap-dropped",
        "display.rs",
        "    if imported >= MAX_IMPORTED_SLOTS {",
        "    if false {",
    ),
    (
        "d11-slot-count-cap-off-by-one",
        "display.rs",
        "    if imported >= MAX_IMPORTED_SLOTS {",
        "    if imported > MAX_IMPORTED_SLOTS {",
    ),
    # ── D-01: launch parameter lists (driver_unsafe.rs) ──
    (
        "d01-count-check-dropped",
        "driver_unsafe.rs",
        "            sizes.len(),\n            self.params.len(),\n            \"{what}: `{}` declares",
        "            self.params.len(),\n            self.params.len(),\n            \"{what}: `{}` declares",
    ),
    (
        "d01-size-check-dropped",
        "driver_unsafe.rs",
        "            assert_eq!(\n                got, want,",
        "            assert_eq!(\n                want, want,",
    ),
    (
        "d01-launch-raw-unchecked",
        "driver_unsafe.rs",
        "        f.check_params(&[param.len()], \"cuLaunchKernel\");\n",
        "",
    ),
    (
        "d01-launch-args-unchecked",
        "driver_unsafe.rs",
        "        live(&stream.life, what);\n        let sizes: Vec<usize> = params.iter().map(Vec::len).collect();\n        f.check_params(&sizes, what);",
        "        live(&stream.life, what);\n        let sizes: Vec<usize> = params.iter().map(Vec::len).collect();\n        let _ = &sizes;",
    ),
    (
        "d01-graph-node-membership-dropped",
        "driver_unsafe.rs",
        "            Arc::ptr_eq(&node.graph, &e.graph),",
        "            true,",
    ),
    (
        "d01-unknown-ptx-type-guessed",
        "driver_unsafe.rs",
        "                    _ => return Err(format!(\"an unknown token {t:?}\")),",
        "                    _ => 8,",
    ),
    (
        "d01-entry-not-in-the-ptx-accepted",
        "driver_unsafe.rs",
        "let Some(entry) = m.entries.iter().find(|e| e.name == sym) else {",
        "let Some(entry) = m.entries.iter().find(|e| e.name == sym).or(m.entries.first()) else {",
    ),
    # ── D-02: device ranges (driver_unsafe.rs) ──
    (
        "d02-span-bound-dropped",
        "driver_unsafe.rs",
        "        if end > self.len as u64 {\n            return None;",
        "        if end > u64::MAX - 1 {\n            return None;",
    ),
    (
        "d02-copy-length-unchecked",
        "driver_unsafe.rs",
        "            n <= self.len,\n            \"{what}: {n:#x} bytes do not fit",
        "            true,\n            \"{what}: {n:#x} bytes do not fit",
    ),
    (
        "d02-pinned-view-unbounded",
        "driver_unsafe.rs",
        "            off.checked_add(len).is_some_and(|e| e <= buf.len),\n            \"pinned_device_view:",
        "            off.checked_add(len).is_some(),\n            \"pinned_device_view:",
    ),
    (
        "d02-d2h-async-pinned-unbounded",
        "driver_unsafe.rs",
        "            off.checked_add(n).is_some_and(|e| e <= buf.len),\n            \"{what}: range leaves the pinned buffer\"\n        );\n        live(&s.life, what);",
        "            off.checked_add(n).is_some(),\n            \"{what}: range leaves the pinned buffer\"\n        );\n        live(&s.life, what);",
    ),
    (
        "d02-negative-fd-becomes-stdin",
        "driver_unsafe.rs",
        "        let Ok(fd_bits) = usize::try_from(fd) else {\n            return Err(format!(\"import_and_map: fd {fd} is negative\"));\n        };",
        "        let fd_bits = usize::try_from(fd).unwrap_or(0);",
    ),
    # ── D-04: lifetimes (driver_unsafe.rs) ──
    (
        "d04-ctx-destroy-keeps-children-alive",
        "driver_unsafe.rs",
        "        ctx.life.end();\n",
        "",
    ),
    (
        "d04-free-after-context-death",
        "driver_unsafe.rs",
        "        if b.kind == DevKind::Owned && b.ptr != 0 && b.life.alive() {",
        "        if b.kind == DevKind::Owned && b.ptr != 0 {",
    ),
    (
        "d04-pinned-access-after-context-death",
        "driver_unsafe.rs",
        "        live(&self.ctx, \"PinnedBuf::read\");\n",
        "",
    ),
    (
        "d04-span-of-a-released-buffer",
        "driver_unsafe.rs",
        "            self.buf.alive(),\n            \"{what}: the device allocation was released",
        "            true,\n            \"{what}: the device allocation was released",
    ),
    # ── F-2 host half: a space cut at its own limit fails only itself (walk.rs, 2026-11-14) ──
    (
        "f2-any-budget-bit-aborts-the-retry",
        "walk.rs",
        "        let aborted = h.refuse_mask & walk_level & !entry_bits != 0;",
        "        let aborted = h.refuse_mask & walk_level != 0;",
    ),
    (
        "f2-header-only-abort-missed",
        "walk.rs",
        "        let aborted = h.refuse_mask & walk_level & !entry_bits != 0;",
        "        let aborted = h.refuse_mask & walk_level & !entry_bits & 0 != 0;",
    ),
    (
        "f2-cut-space-re-walked",
        "walk.rs",
        "            if p.refused_bits() & (KFWR_R_BUDGET | KFWR_R_FRONTIER_CAP) != 0 {",
        "            if false {",
    ),
    # ── F-10 host half: the descriptor checked before CUDA opens (walk.rs, 2026-11-14) ──
    (
        "f10-check-not-called",
        "walk.rs",
        "        if let Err(name) = check_format(&fmt) {",
        "        if let Err(name) = Ok::<(), String>(()) {",
    ),
    (
        "f10-snapshot-rule-dropped",
        "walk.rs",
        "        if n * wpe > KF_SHWORDS {",
        "        if n > KF_SHWORDS {",
    ),
    (
        "f10-leaf-snapshot-rule-dropped",
        "walk.rs",
        "    if big > KF_SHWORDS - KF_MAX_ENT || !pow2(big) {",
        "    if big > KF_SHWORDS || !pow2(big) {",
    ),
    (
        "f10-field-rule-dropped",
        "walk.rs",
        "        if k.lo < 64 && k.bits < 64 && u32::from(k.lo) + u32::from(k.bits) <= 64 && k.shift < 64 {",
        "        if k.lo < 64 && k.bits < 64 && k.shift < 64 {",
    ),
    (
        "f10-bit-rule-dropped",
        "walk.rs",
        "        if b >= 64 {",
        "        if b > 250 {",
    ),
    (
        "f10-pcf-values-unchecked",
        "walk.rs",
        "    if (f.pte_pcf_no_valid_4k, f.pde_pcf_sparse_bit) != pcf {",
        "    if pcf.0 == 0xff {",
    ),
    # ── S1-69: Report::validate checks what the .cu's kf_validate_report checks (2026-11-14) ──
    (
        "s169-version-unchecked",
        "walk.rs",
        "        if h.version != crate::abi::KFWR_VERSION {",
        "        if false {",
    ),
    (
        "s169-page-size-table-unchecked",
        "walk.rs",
        "        if h.ps_log2.iter().any(|&l| !(12..=40).contains(&l)) {",
        "        if h.ps_log2.iter().any(|&l| l > 63) {",
    ),
    (
        "s169-sample-count-unchecked",
        "walk.rs",
        "        if h.sample_count as usize > crate::abi::KF_REFUSAL_SAMPLES {",
        "        if false {",
    ),
    (
        "s169-op-unchecked",
        "walk.rs",
        "                crate::abi::KFWR_OP_MAP | KFWR_OP_UNMAP | crate::abi::KFWR_OP_REMAP",
        "                _",
    ),
    (
        "s169-page-size-code-aliased",
        "walk.rs",
        "            let Some(&log2) = h.ps_log2.get(usize::from(code)) else {",
        "            let Some(&log2) = h.ps_log2.get(usize::from(code) & 3) else {",
    ),
    (
        "s169-alignment-unchecked",
        "walk.rs",
        "                if let Some(value) = value.filter(|v| v % page != 0) {",
        "                if let Some(value) = value.filter(|_| false) {",
    ),
    (
        "s169-unmap-gpga-checked",
        "walk.rs",
        "            let gpga = (r.op != KFWR_OP_UNMAP).then_some(r.gpga);",
        "            let gpga = Some(r.gpga);",
    ),
    # ── D-08: field lists ──
    (
        "d08-a-field-missing-from-the-list",
        "driver_unsafe.rs",
        "        pcf_sparse: u8, pte_pcf_no_valid_4k: u8, pde_pcf_sparse_bit: u8, pad1: [u8; 1], kind: KfField,\n        ps_log2: [u8; 4] }",
        "        pcf_sparse: u8, pte_pcf_no_valid_4k: u8, pde_pcf_sparse_bit: u8, pad1: [u8; 1], kind: KfField }",
    ),
]

FAILED = re.compile(r"^test (\S+) \.\.\. FAILED$", re.M)
PANIC_FAILED = re.compile(r"^---- (\S+) stdout ----$", re.M)


def run_tests():
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))
    p = subprocess.run(
        ["cargo", "test", "-q", "-p", "kf-cuda", "--lib", "--tests", "--no-fail-fast"],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    out = p.stdout + p.stderr
    compiled = "error[E" not in out and "could not compile" not in out
    failed = set(FAILED.findall(out)) | set(PANIC_FAILED.findall(out))
    return p.returncode, compiled, failed, out


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    rc, compiled, failed, out = run_tests()
    if rc != 0 or not compiled or failed:
        print(out[-4000:])
        print("BASELINE IS RED: refuse to judge mutants")
        return 2
    bad = 0
    for name, rel, anchor, repl in MUTANTS:
        if only and only not in name:
            continue
        path = CRATE / "src" / rel
        orig = path.read_bytes()
        text = orig.decode()
        n = text.count(anchor)
        if n != 1:
            print(f"ANCHOR {name}: found {n} times in {rel}")
            bad += 1
            continue
        try:
            path.write_text(text.replace(anchor, repl))
            rc, compiled, failed, out = run_tests()
        finally:
            path.write_bytes(orig)
        assert path.read_bytes() == orig, f"{rel} was not restored"
        if not compiled:
            print(f"NOCOMPILE {name}: the mutant does not build (proves nothing)")
            print(out[-2000:])
            bad += 1
        elif len(failed) < 2:
            print(f"SURVIVED {name}: killers={sorted(failed)}")
            bad += 1
        else:
            print(f"KILLED {name}: {len(failed)} killers: {', '.join(sorted(failed))}")
    print("mutants: " + ("FAIL" if bad else "OK"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
