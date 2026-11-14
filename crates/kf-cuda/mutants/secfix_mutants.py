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
    # ── D-03: the frame a kernel writes is bounded by the allocation ──
    (
        "d03-length-check-dropped",
        "display.rs",
        "Some(n) if n > 0 && n <= len as u64 => Ok(dst),",
        "Some(n) if n > 0 => Ok(dst),",
    ),
    (
        "d03-empty-frame-accepted",
        "display.rs",
        "Some(n) if n > 0 && n <= len as u64 => Ok(dst),",
        "Some(n) if n <= len as u64 => Ok(dst),",
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
        "        let dst = frame_for(self.staging, w, h, 4, what)?;\n        let u = |x: u32| x.to_le_bytes().to_vec();\n        let i =",
        "        let Some((dst, _)) = self.staging else {\n            return Err(refused(what, \"no composition was begun\".into()));\n        };\n        let u = |x: u32| x.to_le_bytes().to_vec();\n        let i =",
    ),
    (
        "d03-yuv-discards-the-staging-length",
        "display.rs",
        "        let dst = frame_for(self.staging, w, h, 4, what)?;\n        self.launch_yuv(f, dst, l, w, h, what)",
        "        let Some((dst, _)) = self.staging else {\n            return Err(refused(what, \"no composition was begun\".into()));\n        };\n        self.launch_yuv(f, dst, l, w, h, what)",
    ),
    # ── D-11: caps ──
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
]

FAILED = re.compile(r"^test (\S+) \.\.\. FAILED$", re.M)
PANIC_FAILED = re.compile(r"^---- (\S+) stdout ----$", re.M)


def run_tests():
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))
    p = subprocess.run(
        ["cargo", "test", "-q", "-p", "kf-cuda", "--lib", "--no-fail-fast"],
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
