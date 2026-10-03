#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""drmnv.py — MEASURE the nvidia-drm / NVKMS private ABI the GPU-copy broker rung uses, per tag.

★ Why (docs/design/V3_DISPLAY.md §8.11, "version tolerance"). The rung issues two nvidia-drm
ioctls (`DRM_IOCTL_NVIDIA_GET_DEV_INFO`, `DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY`) and hands
NVKMS one private struct (`NvKmsKapiPrivImportMemoryParams`). None of them is a stable ABI, and
the DRM core ZERO-FILLS or TRUNCATES an argument whose size differs from the kernel's instead of
refusing it (`drm_ioctl`'s `ksize`/`asize` handling), so a field inserted mid-struct at some tag
would be misread SILENTLY. kf-abi (`crate::drmnv`) therefore offers the rung only at a tag where
this probe measured every value equal to its transcription; elsewhere it refuses by name.

★ The parser is the compiler (owner, 2026-09-21, the rule `dm.py` follows): the tag's OWN
headers are compiled with gcc and a `printf` of every `sizeof`, `offsetof`, enumerator and ioctl
number reads back what a compiler of that tag would use. No regex over C decides a value; the
include closure is resolved by fetching what each `#include "..."` names.

Headers come from GitHub's raw view of `NVIDIA/open-gpu-kernel-modules` at each tag (one small
file per request, cached under --work), or from a local tree with --local TAG=DIR. A tag whose
closure cannot be fetched or compiled is written as `UNMEASURED` with the reason — never a value
borrowed from a neighbouring tag.

usage:
  drmnv.py [--tags "T1 T2 .."] [--work DIR] [--out FILE] [--local TAG=DIR ...]
  (default tags: tools/drivermatrix/tags.txt; default out: traces/driver_matrix/drmnv.tsv)
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
RAW = "https://raw.githubusercontent.com/NVIDIA/open-gpu-kernel-modules/{tag}/{path}"

# Where a quoted include may live, in search order (the 580 layout first; older tags moved few).
DIRS = [
    "kernel-open/nvidia-drm",
    "kernel-open/common/inc",
    "src/nvidia-modeset/kapi/interface",
    "src/nvidia-modeset/interface",
    "src/common/unix/common/inc",
    "src/common/sdk/nvidia/inc",
    "src/common/inc",
    "src/nvidia-modeset/include",
]
# Each root is a list of the names it has had: nvidia-drm's uapi header is `nvidia-drm-ioctl.h`
# through 580.x and `nv_drm_common_ioctl.h` from 590.48.01 (probed at each tag; the first present
# name is compiled).
ROOTS = [
    ["kernel-open/nvidia-drm/nvidia-drm-ioctl.h", "kernel-open/nvidia-drm/nv_drm_common_ioctl.h"],
    ["src/nvidia-modeset/kapi/interface/nvkms-kapi-private.h"],
]

# What the rung consumes. ★ Every member kf-abi's encoders write or decoders read is here.
STRUCTS = {
    "drm_nvidia_get_dev_info_params": [
        "gpu_id", "mig_device", "primary_index", "supports_alloc", "generic_page_kind",
        "page_kind_generation", "sector_layout", "supports_sync_fd", "supports_semsurf",
    ],
    "drm_nvidia_gem_import_nvkms_memory_params": [
        "mem_size", "nvkms_params_ptr", "nvkms_params_size", "handle",
    ],
    "NvKmsKapiPrivImportMemoryParams": [
        "memFd", "surfaceParams.layout", "surfaceParams.blockLinear.log2GobsPerBlock.x",
        "surfaceParams.blockLinear.log2GobsPerBlock.y",
        "surfaceParams.blockLinear.log2GobsPerBlock.z", "surfaceParams.blockLinear.pitchInBlocks",
        "surfaceParams.blockLinear.genericMemory",
    ],
}
IOCTLS = ["DRM_IOCTL_NVIDIA_GET_DEV_INFO", "DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY"]
ENUMS = ["NvKmsSurfaceMemoryLayoutBlockLinear", "NvKmsSurfaceMemoryLayoutPitch"]


def fetch(tag, path, work, local):
    """One file of `tag`, cached. None when the tag has no such file."""
    dst = os.path.join(work, tag, path)
    if os.path.exists(dst):
        return dst
    if os.path.exists(dst + ".absent"):
        return None
    if tag in local:
        src = os.path.join(local[tag], path)
        if not os.path.exists(src):
            return None
        data = open(src, "rb").read()
    else:
        try:
            with urllib.request.urlopen(RAW.format(tag=tag, path=path), timeout=30) as r:
                data = r.read()
        except urllib.error.HTTPError as e:
            if e.code == 404:
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                open(dst + ".absent", "w").close()
                return None
            raise
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    with open(dst, "wb") as f:
        f.write(data)
    return dst


def closure(tag, work, local):
    """Fetch the roots and every quoted include they reach. Returns (dirs, roots, missing)."""
    roots, missing = [], []
    for names in ROOTS:
        hit = next((n for n in names if fetch(tag, n, work, local) is not None), None)
        if hit is None:
            missing.append(" or ".join(names))
        else:
            roots.append(hit)
    seen, todo = set(), list(roots)
    while todo:
        path = todo.pop()
        if path in seen:
            continue
        seen.add(path)
        got = fetch(tag, path, work, local)
        if got is None:
            missing.append(path)
            continue
        for name in re.findall(r'^\s*#\s*include\s+"([^"]+)"', open(got, errors="replace").read(),
                               re.M):
            for d in DIRS:
                cand = f"{d}/{name}"
                if cand in seen or fetch(tag, cand, work, local) is not None:
                    todo.append(cand)
                    break
            else:
                missing.append(name)
    return sorted({os.path.join(work, tag, d) for d in DIRS}), roots, missing


def probe_source(roots):
    lines = [
        "#include <stdio.h>", "#include <stddef.h>", "#include <stdint.h>",
        "#include <drm/drm.h>",
    ] + [f'#include "{os.path.basename(r)}"' for r in roots] + ["int main(void) {"]
    for s, fields in STRUCTS.items():
        t = f"struct {s}"
        lines.append(f'printf("sizeof\\t{s}\\t%zu\\n", sizeof({t}));')
        for f in fields:
            lines.append(f'printf("offsetof\\t{s}.{f}\\t%zu\\n", offsetof({t}, {f}));')
            lines.append(
                f'printf("sizeof\\t{s}.{f}\\t%zu\\n", sizeof((({t} *)0)->{f}));')
    for i in IOCTLS:
        lines.append(f'printf("ioctl\\t{i}\\t%#lx\\n", (unsigned long){i});')
    for e in ENUMS:
        lines.append(f'printf("enum\\t{e}\\t%d\\n", (int){e});')
    lines += ["return 0;", "}"]
    return "\n".join(lines) + "\n"


def measure(tag, work, local):
    try:
        dirs, roots, missing = closure(tag, work, local)
    except Exception as e:  # network: the tag is unmeasured, never guessed
        return [("status", f"UNMEASURED fetch: {e}")]
    with tempfile.TemporaryDirectory() as td:
        c = os.path.join(td, "probe.c")
        exe = os.path.join(td, "probe")
        open(c, "w").write(probe_source(roots))
        cmd = ["gcc", "-std=gnu11", "-o", exe, c] + [f"-I{d}" for d in dirs]
        r = subprocess.run(cmd, capture_output=True, text=True,
                           env=dict(os.environ, LC_ALL="C"))
        if r.returncode != 0:
            # the temporary directory's name is scrubbed, so a re-run reproduces the file
            errs = [l for l in r.stderr.replace(td, "<probe>").splitlines() if "error:" in l]
            err = " ".join(" | ".join(errs[:2] or [r.stderr]).split())[:300]
            why = f"missing {missing[:4]}; " if missing else ""
            return [("status", f"UNMEASURED compile: {why}{err}")]
        out = subprocess.run([exe], capture_output=True, text=True, check=True).stdout
    rows = [l.split("\t") for l in out.splitlines()]
    return [("status", "MEASURED"), ("header", " ".join(roots))] + [
        (f"{k}:{n}", v) for k, n, v in rows]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tags")
    ap.add_argument("--work", default=os.environ.get("DM_WORK", "/var/tmp/kf-drivermatrix") + "/drmnv")
    ap.add_argument("--out", default=os.path.join(REPO, "traces/driver_matrix/drmnv.tsv"))
    ap.add_argument("--local", action="append", default=[])
    a = ap.parse_args()
    if a.tags:
        tags = a.tags.split()
    else:
        tags = [t.strip() for t in open(os.path.join(HERE, "tags.txt"))
                if t.strip() and not t.startswith("#")]
    local = dict(x.split("=", 1) for x in a.local)
    out = [
        "# tag\titem\tvalue    (measured by tools/drivermatrix/drmnv.py: gcc over each tag's OWN "
        "nvidia-drm/NVKMS headers; read by kf_abi::drmnv)",
    ]
    for tag in tags:
        rows = measure(tag, a.work, local)
        print(f"{tag}: {rows[0][1]}", file=sys.stderr)
        out += [f"{tag}\t{k}\t{v}" for k, v in rows]
    with open(a.out, "w") as f:
        f.write("\n".join(out) + "\n")
    print(f"wrote {a.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
