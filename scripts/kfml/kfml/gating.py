"""Requirement gating: ok | not_run (hardware cannot) | env_missing (software; reported FAIL)."""
from __future__ import annotations

import os
from dataclasses import dataclass

from . import versions
from .env import DRIVER_TOO_OLD, MIN_DRIVER_FOR_CUDA


@dataclass
class Verdict:
    kind: str  # ok | not_run | env_missing
    reason: str = ""


def evaluate(spec, env, environ=None) -> Verdict:
    environ = os.environ if environ is None else environ
    if not env.gpu_visible and not env.cuda_available:
        return Verdict("env_missing", "no NVIDIA GPU visible (nvidia-smi found none)")
    if env.torch is None:
        return Verdict("env_missing", "torch is not importable (%s); run `kfml setup --venv DIR`" % (env.cuda_error or ""))
    if not env.cuda_available:
        err = env.cuda_error or ""
        if DRIVER_TOO_OLD.search(err):
            need = MIN_DRIVER_FOR_CUDA.get(".".join(str(x) for x in (versions.parse(env.cuda) or ())[:2]), "newer")
            return Verdict("not_run", "driver too old for this CUDA wheel (needs >= %s)" % need)
        return Verdict("env_missing", "torch cannot use the GPU: %s" % (err or "cuda unavailable"))
    if not versions.ge(env.torch, spec.min_torch):
        return Verdict("env_missing", "torch %s < required %s" % (env.torch, spec.min_torch))
    if env.cc is None:
        return Verdict("env_missing", "compute capability unknown")
    if tuple(env.cc) < tuple(spec.min_cc):
        return Verdict("not_run", "compute capability %s < min %s" % (versions.cc_str(env.cc), versions.cc_str(spec.min_cc)))
    if spec.max_cc is not None and tuple(env.cc) > tuple(spec.max_cc):
        return Verdict("not_run", "compute capability %s > max %s" % (versions.cc_str(env.cc), versions.cc_str(spec.max_cc)))
    if (env.vram_total_gb or 0) < spec.min_vram_gb:
        return Verdict("not_run", "vram %.1f GB < required %.1f GB" % (env.vram_total_gb or 0, spec.min_vram_gb))
    if spec.needs_triton and not (env.triton or env.modules.get("triton")):
        return Verdict("env_missing", "triton is not installed")
    if spec.needs_compiler and not (env.cxx and env.python_h):
        return Verdict("env_missing", "C compiler and/or Python.h missing (install gcc and python3-dev)")
    if spec.needs_nvcc:
        if not env.nvcc:
            return Verdict("env_missing", "nvcc not found (torch cuda %s); `kfml setup --with-nvcc` or install the matching toolkit" % env.cuda)
        t = tuple((versions.parse(env.cuda) or ())[:2])
        n = tuple((versions.parse(env.nvcc) or ())[:2])
        if t and n != t and environ.get("KF_ML_NVCC_LOOSE") != "1":
            return Verdict("env_missing", "nvcc %s does not match torch.version.cuda %s (major.minor must match)" % (env.nvcc, env.cuda))
    if spec.precheck is not None:
        r = spec.precheck(env, environ)
        if r:
            return Verdict("not_run", r)
    return Verdict("ok")
