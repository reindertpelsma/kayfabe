"""Recorded native-host references, kept per die class (never pinned to one die).

Layout: <dir>/<die>/<test>.json, die = slug of the GPU name (e.g. `nvidia-geforce-rtx-4070`); every file records the die,
compute capability, driver, CUDA runtime and torch it was measured on, the test's `data` (what a workload compares
against) and the row's `wall_s` and `metrics` as measured. A flat <dir>/<test>.json (the old layout) is still read.

Comparison rule (owner 2026-11-16: the comparison must tolerate other dies):
  * `data` (loss windows, accuracy: properties of the algorithm, not of the card) is compared against the best
    available reference: the same die first, then the same compute capability, then any other die. The tolerance in
    the workload does not change with the die. `reference_from` in the row says which one was used.
  * wall time and throughput are NEVER a pass criterion. `wall_ratio` (row wall_s / reference wall_s) is reported only when
    the reference is from the same die (same GPU name); against another die it is null, because a ratio between two
    different cards measures the cards, not the stack.
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path


def die_slug(name) -> str:
    return re.sub(r"[^a-z0-9]+", "-", (name or "unknown").lower()).strip("-") or "unknown"


def _cc(env):
    return "%d.%d" % tuple(env.cc) if getattr(env, "cc", None) else None


def write(dirpath, name, env, data, wall_s=None, metrics=None, run_id=None):
    d = Path(dirpath) / die_slug(env.gpu_name)
    d.mkdir(parents=True, exist_ok=True)
    doc = {"test": name, "measured_on": {"gpu_name": env.gpu_name, "compute_cap": _cc(env), "driver": env.driver,
                                         "driver_cuda": getattr(env, "driver_cuda", None), "torch": env.torch,
                                         "cuda": env.cuda, "flavor": getattr(env, "flavor", None), "run_id": run_id},
           "gpu_name": env.gpu_name, "compute_cap": _cc(env), "torch": env.torch, "cuda": env.cuda,
           "driver": env.driver, "data": data, "wall_s": wall_s, "metrics": metrics or {}}
    p = d / (name + ".json")
    tmp = str(p) + ".tmp"
    with open(tmp, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
    os.replace(tmp, p)
    return str(p)


def _read(p):
    try:
        with open(p) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def find(dirpath, name, env=None):
    """Best reference doc for this test and box: same die, else same compute capability, else any die, else flat file.
    Returns (doc, how) with how in same_die | same_cc | other_die | flat, or (None, None)."""
    if not dirpath:
        return None, None
    base = Path(dirpath)
    if not base.is_dir():
        return None, None
    want_slug = die_slug(getattr(env, "gpu_name", None))
    want_cc = _cc(env) if env is not None else None
    docs = []
    for sub in sorted(p for p in base.iterdir() if p.is_dir()):
        doc = _read(sub / (name + ".json"))
        if doc is not None and "data" in doc:
            docs.append((sub.name, doc))
    for slug, doc in docs:
        if slug == want_slug:
            return doc, "same_die"
    for slug, doc in docs:
        if want_cc and doc.get("compute_cap") == want_cc:
            return doc, "same_cc"
    if docs:
        return docs[0][1], "other_die"
    doc = _read(base / (name + ".json"))
    if doc is not None and "data" in doc:
        return doc, "flat"
    return None, None


def load(dirpath, name, env=None):
    doc, _ = find(dirpath, name, env)
    return doc.get("data") if doc else None


def describe(doc, how):
    """Compact, row-safe description of the reference used."""
    m = doc.get("measured_on") or {"gpu_name": doc.get("gpu_name"), "compute_cap": doc.get("compute_cap"),
                                    "driver": doc.get("driver"), "cuda": doc.get("cuda")}
    return {"how": how, "gpu_name": m.get("gpu_name"), "compute_cap": m.get("compute_cap"), "driver": m.get("driver"),
            "cuda": m.get("cuda"), "run_id": m.get("run_id")}


def wall_ratio(doc, how, wall_s):
    """wall_s / reference wall_s, only against the same die; None otherwise (never a pass criterion)."""
    ref = (doc or {}).get("wall_s")
    if how != "same_die" or not ref or not wall_s:
        return None
    return round(float(wall_s) / float(ref), 3)
