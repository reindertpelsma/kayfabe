"""kfml build-wheels: build compile-heavy upstream projects (flash-attn, mamba-ssm, Linux only) from a
PINNED commit + our patch against the venv's torch/CUDA flavor, then scan each wheel. Never uploads.
UNVERIFIED on hardware: the coordinator runs it on a container."""
from __future__ import annotations

import json
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

from . import licenses, pins
from .spec import Upstream
from . import versions


def arch_list(cuda: str) -> str:
    """sm_75..sm_120 covering list; 12.0 needs CUDA >= 12.8. +PTX on the newest."""
    archs = ["7.5", "8.0", "8.6", "8.9", "9.0"]
    if versions.ge(cuda, "12.8"):
        archs += ["10.0", "12.0"]
    return " ".join(archs[:-1] + [archs[-1] + "+PTX"])


def load_builds(pins_doc: dict):
    out = []
    for b in pins_doc.get("wheel_builds", []):
        out.append(Upstream(name=b["name"], repo_url=b["repo_url"], commit=b["commit"], license=b["license"],
                            role="fetched", patch=b.get("patch")))
    return out


def record_for(scan: dict, up, env: dict, glibc: str, patch_sha):
    return {"wheel": scan["filename"], "tags": scan["tags"], "python": env.get("python"),
            "torch": env.get("torch"), "cuda_flavor": env.get("cuda"), "glibc": glibc,
            "upstream": up.name, "upstream_commit": up.commit, "patch_sha256": patch_sha,
            "license_spdx": scan["license_spdx"], "bundled_libs": scan["bundled_libs"],
            "nvidia_origin_libs": scan["nvidia_origin_libs"], "redistributable": scan["redistributable"],
            "refused": bool(scan["nvidia_origin_libs"]),
            "note": "refused: NVIDIA-origin libraries inside" if scan["nvidia_origin_libs"] else ""}


def build_all(out_dir, builds, env: dict, run=subprocess.run, resolve_fn=pins.resolve, tmp_root=None):
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    tmp_root = tmp_root or tempfile.gettempdir()
    glibc = "%s-%s" % platform.libc_ver()
    records = []
    for up in builds:
        r = resolve_fn(up, tmp_root)
        wenv = dict(os.environ, TORCH_CUDA_ARCH_LIST=arch_list(env.get("cuda") or "0"), MAX_JOBS="4",
                    SOURCE_DATE_EPOCH="0")
        p = run([sys.executable, "-m", "pip", "wheel", "--no-deps", "--no-build-isolation", "-w", str(out_dir),
                 r.path], env=wenv, capture_output=True, text=True)
        if p.returncode != 0:
            records.append({"upstream": up.name, "upstream_commit": up.commit, "error": p.stderr[-800:]})
            continue
        for w in sorted(out_dir.glob("%s*.whl" % up.name.replace("-", "_"))):
            records.append(record_for(licenses.scan_wheel(str(w)), up, env, glibc, r.patch_sha256))
    (out_dir / "build-records.json").write_text(json.dumps(records, indent=1, sort_keys=True) + "\n")
    return records
