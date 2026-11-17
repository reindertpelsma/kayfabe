"""python3 -m kfml list | check | run | setup | fetch-data | bundle | build-wheels. Stdlib only."""
from __future__ import annotations

import argparse
import json
import os
import sys

from . import buildwheels, bundle, datasets, flavor, gating, registry, runner, setup_venv, versions
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


def flavor_cuda(name):
    return datasets.load_pins()["flavors"][name]["cuda"]


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
    lines.append("flavor: %s%s" % (e.flavor or "NONE", "  (" + e.flavor_reason + ")" if e.flavor_reason else ""))
    lines.append("       driver %s supports CUDA %s; GPU sm_%s" % (
        e.driver, e.driver_cuda, ("%d%d" % tuple(e.cc)) if e.cc else "?"))
    if e.flavor_blocked:
        lines.append("NOT_RUN: no pinned flavour can run here; every test row will be NOT_RUN (not FAIL)")
    elif e.flavor and e.torch and e.cuda and e.cuda != flavor_cuda(e.flavor):
        lines.append("note: installed torch is CUDA %s, the derived flavour is %s (CUDA %s); re-run setup with "
                     "--flavor auto into a fresh venv if it does not run" % (e.cuda, e.flavor, flavor_cuda(e.flavor)))
    item(bool(e.vram_total_gb and e.vram_total_gb >= 8), "VRAM >= 8 GB (documented floor)",
         "tests need >= 8 GB total VRAM; smaller cards get NOT_RUN")
    item(e.torch is not None, "torch %s (cuda %s, cudnn %s, triton %s)" % (e.torch, e.cuda, e.cudnn, e.triton),
         "python3 -m kfml setup --venv DIR   (flavour is derived from the box; then run kfml from DIR/bin/python)")
    if e.torch and not e.cuda_available:
        item(False, "torch.cuda usable (%s)" % e.cuda_error,
             "driver too old for this CUDA wheel -> NOT_RUN; `setup --flavor auto` picks the flavour the driver can run")
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
        sel = runner.select(specs, args.only.split(",") if args.only else None, args.all, args.tier)
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
    if args.plan:  # the flavour decision only: nothing is created, nothing is downloaded (the stage uses it to start downloads only when a flavour fits)
        try:
            f, why = setup_venv.resolve_flavor(args.flavor)
        except setup_venv.NoFlavor as e:
            out.write(json.dumps({"status": "NOT_RUN", "reason": str(e)}) + "\n")
            return 3
        except setup_venv.SetupError as e:
            sys.stderr.write("error: %s\n" % e)
            return 2
        out.write(json.dumps({"flavor": f, "flavor_reason": why}) + "\n")
        return 0
    try:
        r = setup_venv.setup(args.venv, args.flavor, args.python, args.with_nvcc, args.allow_unhashed, args.installer,
                             nvcc_if_needed=args.nvcc_if_needed, wheelhouse=args.wheelhouse)
    except setup_venv.NoFlavor as e:
        sys.stderr.write("NOT_RUN: %s\n" % e)
        out.write(json.dumps({"status": "NOT_RUN", "reason": str(e)}) + "\n")
        return 3
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
            out.write(json.dumps(dict(name=name, **datasets.fetch(name, data, pins, source_dir=args.source_dir))) + "\n")
        except (datasets.PinMe, datasets.HashMismatch, FileNotFoundError) as e:
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


def _wheelhouse(args, out):
    from . import wheelhouse as wh
    pins = datasets.load_pins()
    if args.verify:
        try:
            doc = wh.verify(args.out)
        except wh.WheelhouseError as e:
            sys.stderr.write("error: %s\n" % e)
            return 1
        out.write(json.dumps({"verified": len(doc["files"]), "total_bytes": doc["total_bytes"], "flavors": sorted(doc["flavors"])}) + "\n")
        return 0
    if args.flavor == "all":
        flavors = [n for n, _ in flavor.table(pins)]
    elif args.flavor == "auto":
        if args.flavor_for:
            dc, cc = args.flavor_for.split(":")
            c = flavor.choose(pins, dc, tuple(int(x) for x in cc.split(".")))
            if c.flavor is None:
                sys.stderr.write("NOT_RUN: %s\n" % c.reason)
                out.write(json.dumps({"status": "NOT_RUN", "reason": c.reason}) + "\n")
                return 3
            flavors = [c.flavor]
        else:
            try:
                f, _ = setup_venv.resolve_flavor("auto", pins)
            except setup_venv.NoFlavor as e:
                sys.stderr.write("NOT_RUN: %s\n" % e)
                out.write(json.dumps({"status": "NOT_RUN", "reason": str(e)}) + "\n")
                return 3
            flavors = [f]
    else:
        flavors = args.flavor.split(",")
    try:
        doc = wh.build(args.out, flavors, pins, guest_python=args.guest_python, guest_glibc=args.guest_glibc, with_redist=not args.no_redist, datasets=args.with_data)
    except (wh.WheelhouseError, setup_venv.SetupError) as e:
        sys.stderr.write("error: %s\n" % e)
        return 1
    out.write(json.dumps({"flavors": flavors, "files": len(doc["files"]), "total_bytes": doc["total_bytes"], "build_s": doc["build_s"]}) + "\n")
    return 0


def wheelhouse_default_glibc():
    from . import wheelhouse as wh
    return wh.DEFAULT_GLIBC


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
    s.add_argument("--tier", choices=("mid", "large"),
                   help="mid: the gated mid tests; large: every test (mid plus the non-gate ones)")
    s.add_argument("--json")
    s.add_argument("--record-reference")
    s.add_argument("--reference")
    s.add_argument("--log-dir")
    s.add_argument("--budget-scale")
    s = sub.add_parser("setup")
    s.add_argument("--venv", required=True)
    s.add_argument("--flavor", choices=("auto",) + tuple(setup_venv.FLAVORS), default="auto",
                   help="auto (default): derived from the driver's CUDA version and the GPU's compute capability")
    s.add_argument("--python")
    s.add_argument("--with-nvcc", action="store_true")
    s.add_argument("--nvcc-if-needed", action="store_true",
                   help="like --with-nvcc, but only when no nvcc with the flavour's CUDA major.minor is on the box")
    s.add_argument("--allow-unhashed", action="store_true")
    s.add_argument("--plan", action="store_true", help="print the flavour decision and exit (3 = no flavour fits); installs and creates nothing")
    s.add_argument("--wheelhouse", metavar="DIR", help="offline install from a wheelhouse (no index, hashes required)")
    s.add_argument("--installer", choices=("pip", "uv"), default="pip")
    s = sub.add_parser("fetch-data")
    s.add_argument("--from", dest="source_dir", metavar="DIR",
                   help="offline: take the datasets from DIR (a wheelhouse's data/); still verified against pins.json")
    s = sub.add_parser("wheelhouse", help="build or verify the offline wheelhouse (host side; the guest installs from it)")
    s.add_argument("--out", required=True)
    s.add_argument("--flavor", default="auto", help="auto (this box), all, or cu126|cu128 (comma separated)")
    s.add_argument("--guest-python", help="the CPython (3.12) of the machine that will install it; default: this interpreter")
    s.add_argument("--guest-glibc", default=wheelhouse_default_glibc(), help="glibc of the guest (2.39): the newest manylinux tag accepted")
    s.add_argument("--no-redist", action="store_true")
    s.add_argument("--with-data", action="store_true", help="also fetch the pinned datasets into OUT/data")
    s.add_argument("--verify", action="store_true", help="only re-hash OUT against its MANIFEST.json")
    s.add_argument("--flavor-for", metavar="DRIVER_CUDA:CC",
                   help="choose the flavour for these guest facts instead of this box (e.g. 12.6:8.9)")
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
            "bundle": _bundle, "build-wheels": _build_wheels, "wheelhouse": _wheelhouse}[args.cmd](args, out)
