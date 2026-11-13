#!/usr/bin/env python3
"""The shipped GPU programs are the programs their sources compile to (security review 3 F-5, F-8).

STATUS: LIVE, 2026-11-13. Stdlib only. Three checks, each a list of findings (empty = pass):

  static    no tools needed; runs everywhere.
            * the two HAND-WRITTEN display PTX files (kf_scanout.ptx, kf_sum.ptx) have no source in the
              tree, so nothing can rebuild them: they are ACCEPTED UNREVIEWED BLOBS, pinned by sha256
              below. Any edit fails here until the pin is changed in the same commit (a reviewer then
              sees the blob change and its pin change together). Remove an entry when its source lands.
            * every constant the kernel and the Rust launcher must agree on (shared memory per warp,
              frontier and scratch capacities, grid, block, warp) is read from both
              `cuda/walk/kf_walk.cu` and `crates/kf-cuda/src/walk.rs` and compared. A disagreement is a
              shared-memory or buffer overrun, not a style issue.
  walk      `cuda/walk/kf_walk.ptx` is rebuilt from `kf_walk.cu` with `cuda/walk/make_ptx.py` and the
            pinned NVRTC (nvidia-cuda-nvrtc-cu12==12.2.140, the wheel's sha256 is pinned in
            `scripts/ci/nvrtc-12.2.140.requirements.txt`) and compared byte for byte.
  display   `kf_yuv.ptx`, `kf_color.ptx`, `kf_bl_pack.ptx` are rebuilt with their `make_*_ptx.sh`
            scripts in a scratch copy of `cuda/display` and compared byte for byte. The compiler is
            part of the output (each header records it), so the check REFUSES to run with any clang
            other than the recorded one rather than reporting a meaningless difference.

usage: walk_ptx_repro.py static
       walk_ptx_repro.py walk --nvrtc /path/to/libnvrtc.so.12
       walk_ptx_repro.py display
"""

from __future__ import annotations

import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]

# ── accepted unreviewed blobs: hand-written PTX with no source anywhere in the tree or history ──
# (review 3 F-8). Each is loaded into kayfabe's display context (`crates/kf-cuda/src/display.rs`).
ACCEPTED_BLOBS: dict[str, str] = {
    "cuda/display/kf_scanout.ptx": "2daa436dacbca9a37c6005e41e32ed55d7887320007f1dbd39242984a5c9831c",
    "cuda/display/kf_sum.ptx": "047d842f632ca8d60a37a62bdfa75a0729ac5d93e5fcb313fdfe149c6e19c996",
}

GENERATED_DISPLAY = {
    "kf_yuv.ptx": "make_yuv_ptx.sh",
    "kf_color.ptx": "make_color_ptx.sh",
    "kf_bl_pack.ptx": "make_pack_ptx.sh",
}

# kernel #define  ->  Rust const in crates/kf-cuda/src/walk.rs (same name)
SHARED_CONSTANTS = (
    "KF_DIRS",
    "KF_MAX_ENT",
    "KF_SHWORDS",
    "KF_MAX_FRONTIER",
    "KF_MAX_SCRATCH",
    "KF_PAR_BLOCK",
    "KF_PAR_GRID",
    "KF_SCAN_BLOCK",
    "KF_WARP",
)

NVRTC_HEADER = "// Cuda compilation tools, release 12.2, V12.2.140"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_blobs(root: Path) -> list[str]:
    out = []
    for rel, want in ACCEPTED_BLOBS.items():
        p = root / rel
        if not p.is_file():
            out.append(f"{rel}: accepted blob is missing (remove its pin if it was replaced by a source)")
            continue
        got = sha256(p)
        if got != want:
            out.append(f"{rel}: sha256 {got} is not the pinned {want} (hand-written PTX changed: review it, then re-pin)")
    return out


def _eval_const(expr: str, known: dict[str, int]) -> int:
    """A C/Rust constant expression of integers, names already known, + - * << and parentheses."""
    e = re.sub(r"//.*|/\*.*?\*/", "", expr)
    e = re.sub(r"\b(\d+)[uU]?[lL]*\b", r"\1", e).replace("_", "")
    for name, val in sorted(known.items(), key=lambda kv: -len(kv[0])):
        e = re.sub(rf"\b{name.replace('_', '')}\b", str(val), e)
    e = re.sub(r"\bas\s+\w+", "", e).strip()
    if not re.fullmatch(r"[0-9\s+\-*()<>]+", e):
        raise ValueError(f"cannot evaluate {expr!r} (got {e!r})")
    return int(eval(e, {"__builtins__": {}}, {}))  # noqa: S307 -- digits and operators only (checked)


def kernel_constants(cu: str) -> dict[str, int]:
    out: dict[str, int] = {}
    for m in re.finditer(r"^\s*#define\s+(KF_[A-Z0-9_]+)\s+(.+?)\s*$", cu, re.M):
        name, expr = m.group(1), m.group(2)
        if name in SHARED_CONSTANTS and name not in out:
            out[name] = _eval_const(expr, out)
    return out


def rust_constants(rs: str) -> dict[str, int]:
    out: dict[str, int] = {}
    for m in re.finditer(r"^\s*(?:pub\s+)?const\s+(KF_[A-Z0-9_]+)\s*:\s*\w+\s*=\s*(.+?);", rs, re.M):
        name, expr = m.group(1), m.group(2)
        if name in SHARED_CONSTANTS and name not in out:
            out[name] = _eval_const(expr, out)
    return out


def check_constants(root: Path) -> list[str]:
    cu = (root / "cuda/walk/kf_walk.cu").read_text()
    rs = (root / "crates/kf-cuda/src/walk.rs").read_text()
    k, r = kernel_constants(cu), rust_constants(rs)
    out = []
    for name in SHARED_CONSTANTS:
        if name not in k:
            out.append(f"{name}: not defined in cuda/walk/kf_walk.cu")
        elif name not in r:
            out.append(f"{name}: not defined in crates/kf-cuda/src/walk.rs")
        elif k[name] != r[name]:
            out.append(f"{name}: kernel {k[name]} != launcher {r[name]} (the launch would overrun a kernel buffer)")
    return out


def compare(name: str, committed: bytes, rebuilt: bytes) -> list[str]:
    if committed == rebuilt:
        return []
    a, b = committed.splitlines(), rebuilt.splitlines()
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return [f"{name}: committed PTX differs from the rebuild at line {i + 1}: {x[:100]!r} vs {y[:100]!r}"]
    return [f"{name}: committed PTX ({len(committed)} B) differs from the rebuild ({len(rebuilt)} B) in length"]


Runner = Callable[..., "subprocess.CompletedProcess[bytes]"]


def check_walk(root: Path, nvrtc: str, run: Runner = subprocess.run) -> list[str]:
    committed = (root / "cuda/walk/kf_walk.ptx").read_bytes()
    if NVRTC_HEADER.encode() not in committed[:400]:
        return [f"kf_walk.ptx: header does not name the pinned compiler ({NVRTC_HEADER!r})"]
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "kf_walk.ptx"
        env = dict(os.environ, NVRTC_SO=nvrtc)
        p = run(
            [sys.executable, "-B", "-I", str(root / "cuda/walk/make_ptx.py"),
             str(root / "cuda/walk/kf_walk.cu"), str(out), "compute_75"],
            env=env, capture_output=True,
        )
        if p.returncode != 0 or not out.is_file():
            return [f"kf_walk.ptx: rebuild failed rc={p.returncode}: {p.stderr[-600:]!r}"]
        rebuilt = out.read_bytes()
    if NVRTC_HEADER.encode() not in rebuilt[:400]:
        return [f"kf_walk.ptx: the NVRTC used is not 12.2.140 (header {rebuilt[:200]!r})"]
    return compare("cuda/walk/kf_walk.ptx", committed, rebuilt)


def recorded_compiler(ptx: bytes) -> str | None:
    m = re.search(rb"^// compiler: (.*) \(NVPTX", ptx, re.M)
    return m.group(1).decode() if m else None


def check_display(root: Path, clang_version: str | None = None, run: Runner = subprocess.run) -> list[str]:
    src = root / "cuda/display"
    if clang_version is None:
        p = run(["clang", "--version"], capture_output=True)
        clang_version = p.stdout.decode(errors="replace").splitlines()[0].strip() if p.returncode == 0 else ""
    out = []
    for ptx, script in GENERATED_DISPLAY.items():
        want = recorded_compiler((src / ptx).read_bytes())
        if want != clang_version:
            out.append(f"{ptx}: recorded compiler {want!r} is not this clang {clang_version!r}: refusing to compare")
    if out:
        return out
    with tempfile.TemporaryDirectory() as td:
        work = Path(td) / "display"
        shutil.copytree(src, work)
        for ptx, script in GENERATED_DISPLAY.items():
            p = run(["bash", str(work / script)], capture_output=True)
            if p.returncode != 0:
                out.append(f"{ptx}: {script} failed rc={p.returncode}: {p.stderr[-400:]!r}")
                continue
            out += compare(f"cuda/display/{ptx}", (src / ptx).read_bytes(), (work / ptx).read_bytes())
    return out


def main(argv: list[str]) -> int:
    if not argv or argv[0] not in ("static", "walk", "display"):
        print(__doc__, file=sys.stderr)
        return 2
    cmd = argv[0]
    if cmd == "static":
        findings = check_blobs(ROOT) + check_constants(ROOT)
    elif cmd == "walk":
        if len(argv) != 3 or argv[1] != "--nvrtc":
            print("usage: walk_ptx_repro.py walk --nvrtc /path/to/libnvrtc.so.12", file=sys.stderr)
            return 2
        findings = check_walk(ROOT, argv[2])
    else:
        findings = check_display(ROOT)
    for f in findings:
        print(f"walk_ptx_repro {cmd}: FAIL {f}")
    if not findings:
        print(f"walk_ptx_repro {cmd}: ok")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
