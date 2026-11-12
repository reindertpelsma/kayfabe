"""python3 -m kfml list | check | run | setup | fetch-data | bundle | build-wheels. Stdlib only."""
from __future__ import annotations

import argparse
import json
import os
import sys

from . import buildwheels, bundle, datasets, gating, registry, runner, setup_venv, versions
from . import env as envmod
from .spec import MID_TOTAL_LIMIT_S, guest_estimate_s, tier_problems


def _list(args, out):
    specs = registry.all_specs()
    timings = registry.load_timings()
    if args.tier:
        specs = [s for s in specs if s.tier == args.tier]
    rows = []
    for s in specs:
        rows.append({"name": s.name, "tier": s.tier, "gate": s.gate, "launch_bound": s.launch_bound,
                     "budget_s": s.budget_s, "min_vram_gb": s.min_vram_gb, "min_cc": versions.cc_str(s.min_cc),
                     "max_cc": versions.cc_str(s.max_cc), "min_torch": s.min_torch,
                     "host_runtime_s": timings.get(s.name), "guest_estimate_s": guest_estimate_s(s, timings),
                     "download_note": s.download_note})
    known = [r["guest_estimate_s"] for r in rows if r["guest_estimate_s"] is not None]
    summary = {"count": len(rows), "estimate_total_s": sum(known), "estimate_known": len(known),
               "estimate_unknown": len(rows) - len(known), "mid_total_limit_s": MID_TOTAL_LIMIT_S,
               "problems": tier_problems(registry.all_specs(), timings)}
    if args.json:
        out.write(json.dumps({"tests": rows, "summary": summary}, indent=1) + "\n")
    else:
        for r in rows:
            out.write("%-18s tier=%-5s gate=%-5s budget=%5.0fs vram>=%.0fGB cc>=%s est=%s%s\n" % (
                r["name"], r["tier"], r["gate"], r["budget_s"], r["min_vram_gb"], r["min_cc"],
                "%.0fs" % r["guest_estimate_s"] if r["guest_estimate_s"] is not None else "unmeasured",
                "  launch_bound" if r["launch_bound"] else ""))
        out.write("total guest estimate: %.0f s over %d measured of %d (%d unmeasured)\n" % (
            summary["estimate_total_s"], summary["estimate_known"], summary["count"], summary["estimate_unknown"]))
        for p in summary["problems"]:
            out.write("PROBLEM: %s\n" % p)
    return 0


def _check(args, out, probe=envmod.probe_env):
    e = probe()
    root = envmod.venv_root()
    lines, bad = [], 0

    def item(ok, what, fix=""):
        nonlocal bad
        lines.append("%-4s %s%s" % ("ok" if ok else "MISSING", what, "" if ok else "\n       -> " + fix))
        bad += 0 if ok else 1

    item(sys.version_info >= (3, 8), "python %s" % e.python, "install python >= 3.8")
    item(e.gpu_visible or e.cuda_available, "NVIDIA GPU: %s, cc %s, %s GB, driver %s" % (
        e.gpu_name, versions.cc_str(e.cc), e.vram_total_gb and round(e.vram_total_gb, 1), e.driver),
        "no GPU visible: need an NVIDIA GPU and its driver (the driver is never installed by kfml)")
    item(bool(e.vram_total_gb and e.vram_total_gb >= 8), "VRAM >= 8 GB (documented floor)",
         "tests need >= 8 GB total VRAM; smaller cards get NOT_RUN")
    item(e.torch is not None, "torch %s (cuda %s, cudnn %s, triton %s)" % (e.torch, e.cuda, e.cudnn, e.triton),
         "python3 -m kfml setup --venv DIR   (then run kfml from DIR/bin/python)")
    if e.torch and not e.cuda_available:
        item(False, "torch.cuda usable (%s)" % e.cuda_error,
             "driver too old for this CUDA wheel -> NOT_RUN; try `setup --flavor cu126`, or update the driver")
    item(bool(e.torch and versions.ge(e.torch, "2.7")), "torch >= 2.7", "install torch >= 2.7 (cu128 index)")
    nv_ok = bool(e.nvcc) and (not e.cuda or tuple(versions.parse(e.nvcc)[:2]) == tuple((versions.parse(e.cuda) or ())[:2]))
    item(nv_ok, "nvcc %s (must equal torch cuda %s major.minor) %s" % (e.nvcc, e.cuda, e.nvcc_path or ""),
         "python3 -m kfml setup --venv DIR --with-nvcc  (venv-contained, UNVERIFIED) or install the matching toolkit")
    item(e.cxx, "C compiler", "apt-get install gcc g++")
    item(e.python_h, "Python.h (python3-dev)", "apt-get install python3-dev")
    item(e.disk_free_gb is None or e.disk_free_gb >= 20, "disk free %s GB (>= 20 recommended: CUDA wheels are several GB)" % (
        e.disk_free_gb and round(e.disk_free_gb, 1)), "free disk space")
    item(root is not None, "venv/KF_ML_HOME: %s" % root, "run inside the venv from `kfml setup --venv DIR` or set KF_ML_HOME")
    item(bool(e.lock_sha256), "venv lock sha256: %s" % e.lock_sha256, "created by `kfml setup`")
    if root:
        lines.append("cache paths (delete the venv dir = full cleanup):")
        for k, v in sorted(envmod.cache_env(root).items()):
            lines.append("       %s=%s" % (k, v))
    out.write("\n".join(lines) + "\n")
    # per-test verdicts (what run would do)
    for s in registry.all_specs():
        v = gating.evaluate(s, e)
        out.write("test %-18s %s%s\n" % (s.name, v.kind.upper(), (": " + v.reason) if v.reason else ""))
    return 1 if bad else 0


def _run(args, out, probe=envmod.probe_env):
    specs = registry.all_specs()
    try:
        sel = runner.select(specs, args.only.split(",") if args.only else None, args.all)
    except runner.RunnerError as e:
        sys.stderr.write("error: %s\n" % e)
        return 2
    e = probe()
    done = []

    def on_row(row):
        out.write(json.dumps(row, sort_keys=True) + "\n")
        out.flush()
        sys.stderr.write("[%s] %s %s\n" % (row["status"], row["name"], row["reason"]))
        done.append(row)

    try:
        runner.run_all(sel, e, record_dir=args.record_reference, reference_dir=args.reference,
                       log_dir=args.log_dir, budget_scale=float(args.budget_scale or os.environ.get("KF_ML_BUDGET_SCALE", 1)),
                       timings=registry.load_timings(), on_row=on_row)
    except runner.RunnerError as ex:
        sys.stderr.write("error: %s\n" % ex)
        return 2
    if args.json:
        with open(args.json, "w") as f:
            json.dump(done, f, indent=1, sort_keys=True)
    return 1 if any(r["status"] == "FAIL" for r in done) else 0


def _setup(args, out):
    try:
        r = setup_venv.setup(args.venv, args.flavor, args.python, args.with_nvcc, args.allow_unhashed, args.installer)
    except setup_venv.SetupError as e:
        sys.stderr.write("error: %s\n" % e)
        return 2
    out.write(json.dumps(r) + "\n")
    return 0


def _fetch(args, out):
    root = envmod.venv_root()
    if root is None:
        sys.stderr.write("error: run inside the venv (or set KF_ML_HOME)\n")
        return 2
    pins = datasets.load_pins()
    data = envmod.cache_env(root)["KF_ML_DATA"]
    rc = 0
    for name in sorted(pins["datasets"]):
        try:
            out.write(json.dumps(dict(name=name, **datasets.fetch(name, data, pins))) + "\n")
        except (datasets.PinMe, datasets.HashMismatch) as e:
            sys.stderr.write("%s\n" % e)
            rc = 1
    return rc


def _bundle(args, out):
    pins = datasets.load_pins()
    ups = [u for s in registry.all_specs() for u in s.upstreams]
    r = bundle.build(args.out, args.wheels, pins, ups)
    out.write(json.dumps({"path": r["path"], "sha256": r["sha256"]}) + "\n")
    return 0


def _build_wheels(args, out):
    pins = datasets.load_pins()
    builds = buildwheels.load_builds(pins)
    if not builds:
        sys.stderr.write("nothing to build: pins.json wheel_builds is empty\n")
        return 0
    e = envmod.probe_env()
    recs = buildwheels.build_all(args.out, builds, {"python": e.python, "torch": e.torch, "cuda": e.cuda})
    out.write(json.dumps(recs, indent=1) + "\n")
    return 1 if any("error" in r or r.get("refused") for r in recs) else 0


def build_parser():
    p = argparse.ArgumentParser(prog="kfml")
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("list")
    s.add_argument("--tier", choices=("mid", "large"))
    s.add_argument("--json", action="store_true")
    sub.add_parser("check")
    s = sub.add_parser("run")
    s.add_argument("--only")
    s.add_argument("--all", action="store_true", help="include tests that are not part of the gate")
    s.add_argument("--json")
    s.add_argument("--record-reference")
    s.add_argument("--reference")
    s.add_argument("--log-dir")
    s.add_argument("--budget-scale")
    s = sub.add_parser("setup")
    s.add_argument("--venv", required=True)
    s.add_argument("--flavor", choices=tuple(setup_venv.FLAVORS), default="cu128")
    s.add_argument("--python")
    s.add_argument("--with-nvcc", action="store_true")
    s.add_argument("--allow-unhashed", action="store_true")
    s.add_argument("--installer", choices=("pip", "uv"), default="pip")
    sub.add_parser("fetch-data")
    s = sub.add_parser("bundle")
    s.add_argument("--out", required=True)
    s.add_argument("--wheels")
    s = sub.add_parser("build-wheels")
    s.add_argument("--out", required=True)
    return p


def main(argv=None, out=None):
    out = out or sys.stdout
    args = build_parser().parse_args(argv)
    return {"list": _list, "check": _check, "run": _run, "setup": _setup, "fetch-data": _fetch,
            "bundle": _bundle, "build-wheels": _build_wheels}[args.cmd](args, out)
