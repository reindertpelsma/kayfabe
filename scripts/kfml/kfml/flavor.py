"""Automatic wheel flavour selection (owner, 2026-11-16): derived from what the box has, never from a
hard-coded die or driver. Pure functions over (driver CUDA version, compute capability); stdlib only.

A flavour is runnable when (a) the driver supports at least the flavour's CUDA version (the CUDA
version `nvidia-smi` prints, i.e. the highest CUDA the driver can run) and (b) the flavour's wheels carry
kernels for the GPU's compute capability. The newest runnable flavour wins. The flavour table (CUDA version,
supported compute-capability range, lock file) lives in pins.json `flavors`, next to the wheel hash locks.
"""
from __future__ import annotations

import re
import shutil
import subprocess
from dataclasses import dataclass
from typing import Optional

from . import versions


@dataclass
class Choice:
    flavor: Optional[str]  # None: no pinned flavour can run on this box (NOT_RUN)
    reason: str  # one line: what was chosen and why, or why nothing can run
    considered: tuple = ()  # (flavor, ok, why) in preference order


def _cuda_tuple(v):
    t = versions.parse(v)
    return tuple(t[:2]) if t else None


def table(pins: dict) -> list:
    """Flavours newest CUDA first: [(name, meta)]."""
    fl = pins.get("flavors") or {}
    return sorted(fl.items(), key=lambda kv: _cuda_tuple(kv[1]["cuda"]), reverse=True)


def choose(pins: dict, driver_cuda, cc) -> Choice:
    """driver_cuda: highest CUDA version the driver supports ('12.6'), or None when unknown. cc: (major, minor).

    1. STRICT: the newest flavour whose CUDA version the driver supports and whose wheels cover the GPU's sm.
    2. MINOR-VERSION COMPATIBILITY (added 2026-11-17): none is strict, but the driver is of the same CUDA major as a flavour and at least its
       `compat_min_driver_cuda`, and the GPU is not newer than `compat_cc_max` (a driver generation cannot drive an architecture it predates): the
       flavour with the LOWEST CUDA version that covers the sm is chosen. MEASURED (run kfml_run_20261117_ampere_older_driver): the cu126 wheels ran all
       six rows on an Ampere card under a CUDA 12.2 driver, which the strict rule alone calls NOT_RUN; the cu128 wheels did the same under a CUDA 12.6
       driver (run kfml_run_20261116). The reason says which rule picked the flavour.
    Anything else (another CUDA major, a driver older than the compat minimum, a GPU outside every wheel range) is NOT_RUN with the named reason."""
    tried = []
    if not pins.get("flavors"):
        return Choice(None, "no flavours pinned in pins.json", ())
    if cc is None:
        return Choice(None, "compute capability unknown: cannot pick a flavour", ())
    dc = _cuda_tuple(driver_cuda)
    if dc is None:
        return Choice(None, "driver CUDA version unknown (nvidia-smi gave none): cannot pick a flavour", ())
    cc = tuple(cc)
    compat = []
    for name, meta in table(pins):
        need = _cuda_tuple(meta["cuda"])
        lo, hi = tuple(meta["cc_min"]), tuple(meta["cc_max"])
        if cc < lo or cc > hi:
            tried.append((name, False, "its wheels cover sm_%d%d..sm_%d%d, the GPU is sm_%d%d" % (lo + hi + cc)))
        elif dc >= need:
            tried.append((name, True, "driver CUDA %s.%s >= %s and sm_%d%d is inside sm_%d%d..sm_%d%d" % (
                (dc[0], dc[1], meta["cuda"]) + cc + lo + hi)))
            why = "; ".join("%s: %s" % (n, w) for n, ok, w in tried if not ok)
            return Choice(name, "chose %s (%s)%s" % (name, tried[-1][2], ("; rejected " + why) if why else ""), tuple(tried))
        else:
            tried.append((name, False, "driver supports CUDA %s.%s < %s" % (dc[0], dc[1], meta["cuda"])))
            cmin = _cuda_tuple(meta.get("compat_min_driver_cuda"))
            cmax = tuple(meta["compat_cc_max"]) if meta.get("compat_cc_max") else None
            if cmin and cmax and dc[0] == need[0] and dc >= cmin and cc <= cmax:
                compat.append((need, name, meta))
    if compat:
        need, name, meta = sorted(compat, key=lambda c: c[0])[0]
        why = "; ".join("%s: %s" % (n, w) for n, ok, w in tried if not ok and n != name)
        return Choice(name, "chose %s by minor-version compatibility (driver CUDA %s.%s < %s but the same CUDA major and >= %s, sm_%d%d <= sm_%d%d; "
                      "measured to run: kfml_run_20261117_ampere_older_driver)%s" % (
                          (name, dc[0], dc[1], meta["cuda"], meta["compat_min_driver_cuda"]) + cc + tuple(meta["compat_cc_max"]) +
                          (("; rejected " + why if why else ""),)),
                      tuple(tried) + ((name, True, "minor-version compatibility"),))
    return Choice(None, "no pinned flavour can run on this box: " + "; ".join("%s: %s" % (n, w) for n, ok, w in tried), tuple(tried))


SMI_CUDA = re.compile(r"CUDA Version:\s*(\d+\.\d+)")


def driver_cuda_from_smi_text(text):
    m = SMI_CUDA.search(text or "")
    return m.group(1) if m else None


def driver_cuda_from_driver(driver, min_driver_table: dict):
    """Fallback when nvidia-smi prints no 'CUDA Version': the highest CUDA whose documented minimum driver is <= driver.
    The table is NVIDIA's (env.MIN_DRIVER_FOR_CUDA, UNVERIFIED there); returns None for an older driver than the table."""
    best = None
    for cuda, mind in min_driver_table.items():
        if versions.ge(driver, mind) and (best is None or _cuda_tuple(cuda) > _cuda_tuple(best)):
            best = cuda
    return best


def smi_driver_cuda(run=subprocess.run, which=shutil.which):
    exe = which("nvidia-smi")
    if not exe:
        return None
    try:
        return driver_cuda_from_smi_text(run([exe], capture_output=True, text=True, timeout=30).stdout)
    except Exception:
        return None
