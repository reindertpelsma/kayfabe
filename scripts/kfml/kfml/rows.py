"""One JSON row per test."""
from __future__ import annotations

STATUSES = ("PASS", "FAIL", "NOT_RUN")
FIELDS = ("name", "status", "reason", "wall_s", "gpu_name", "compute_cap", "vram_gb", "driver",
          "torch", "cuda", "metrics")
EXTRA = ("cudnn", "triton", "lock_sha256", "upstream_commits", "patch_sha256", "dataset_sha256",
         "tier", "launch_bound", "guest_estimate_s")


def make_row(name, status, reason, wall_s, env, metrics=None, **extra):
    if status not in STATUSES:
        raise ValueError("bad status %r" % status)
    row = {
        "name": name,
        "status": status,
        "reason": reason or "",
        "wall_s": round(float(wall_s), 3),
        "gpu_name": getattr(env, "gpu_name", None),
        "compute_cap": ("%d.%d" % tuple(env.cc)) if getattr(env, "cc", None) else None,
        "vram_gb": getattr(env, "vram_total_gb", None),
        "driver": getattr(env, "driver", None),
        "torch": getattr(env, "torch", None),
        "cuda": getattr(env, "cuda", None),
        "metrics": dict(metrics or {}),
        "cudnn": getattr(env, "cudnn", None),
        "triton": getattr(env, "triton", None),
        "lock_sha256": getattr(env, "lock_sha256", None),
        "upstream_commits": {},
        "patch_sha256": {},
        "dataset_sha256": {},
    }
    for k, v in extra.items():
        if k not in EXTRA:
            raise ValueError("unknown row field %r" % k)
        row[k] = v
    return row
