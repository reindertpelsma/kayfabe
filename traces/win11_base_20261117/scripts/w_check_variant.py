#!/usr/bin/env python3
"""Ablation of the kf3 launch properties on an OFFLINE throwaway clone of a sealed image, through the repo's own propimage_check pipeline.
STATUS: scratch, 2026-11-17. Only the argv of the kf3 device changes (the harness, image, clone, clock, offline network are the same):
  no-guest-driver   drop guest-driver=580.65.06
  uuid-auto         drop gpu-uuid= (the device default is `auto`, a hash of -uuid); the seed-UUID rows are informational then
  plain             drop both: the property set a plain `win_vm.sh run` builds on next today (no guest-driver=, no gpu-uuid=)
  baseline          unchanged (control)
usage: w_check_variant.py VARIANT -- <propimage_check.py arguments>   (runs ON the pc as root, GPU lock held by the caller)"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401

CK = load_check()
variant = sys.argv[1]
assert variant in ("baseline", "no-guest-driver", "uuid-auto", "plain"), variant
rest = sys.argv[sys.argv.index("--") + 1:]
orig = CK.PI.build_argv


def build_argv(*a, **k):
    argv = orig(*a, **k)
    i = next(n for n, v in enumerate(argv) if v == "-device" and argv[n + 1].startswith("kf3-gpu"))
    props = argv[i + 1].split(",")
    if variant in ("no-guest-driver", "plain"):
        props = [p for p in props if not p.startswith("guest-driver=")]
    if variant in ("uuid-auto", "plain"):
        props = [p for p in props if not p.startswith("gpu-uuid=")]
    argv[i + 1] = ",".join(props)
    return argv


CK.PI.build_argv = build_argv
if variant in ("uuid-auto", "plain"):
    def gpu_fix(g, expected):
        r = g.ps("$x=@(& nvidia-smi --query-gpu=uuid,name,driver_version --format=csv,noheader 2>&1); $x -join '|'")
        txt = re.sub(r"GPU-[0-9a-f-]{20,}", "GPU-<uuid>", r.stdout)
        return (r.returncode == 0 and "GPU-<uuid>" in txt, "uuid-auto variant: nvidia-smi answered: " + txt + r.stderr)
    CK.gpu_fix = gpu_fix
    cmp0 = CK.PI.compare
    CK.PI.compare = lambda st, f: [tuple("INFO" if (j == 1 and v == "MISMATCH" and "gpu" in str(r[0]).lower()) else v for j, v in enumerate(r)) for r in cmp0(st, f)]
sys.exit(CK.main(rest))
