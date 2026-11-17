#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kfopen.py -- run the open Windows rows (VK-GL-CTS, DirectX-Graphics-Samples extras) from the repo alone.

STATUS: DESIGN-ONLY, 2026-11-17 (docs/design/V3_WINDOWS_OPEN_ROWS.md). GPU-free logic, tested with fakes; nothing here ran on a Windows guest or on
native Windows. A standalone user trusts git only: no private object, no account, no secret, no licence, no cache. Run it on the Windows machine
(a kayfabe guest or native Windows) with any Python 3.9+; the same file runs inside the matrix guest (apps.json rows `vkglcts_*`, `dxsamples_*`).

  kfopen.py list                                 the rows
  kfopen.py check                                pins, caselists and rows are consistent (CI; needs no Windows)
  kfopen.py plan ROW                             what the row needs and how each download is anchored
  kfopen.py prepare ROW [ROW...]                 toolchain, pinned sources, build (online; a kayfabe gate guest is offline)
  kfopen.py run ROW [--control FILE] [--role guest|native] [--out DIR]     prepare if needed, run, print KFOPEN/CHECK lines
  kfopen.py diff --native FILE --guest FILE      the native-vs-guest comparison of a CTS row

Exit codes: 0 PASS, 1 FAIL, 3 NOT_RUN (a `KFNOTRUN <row> reason=...` line says why), 2 usage or refusal.
"""
import argparse
import json
import os
import socket
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import cts_caselist  # noqa: E402
import cts_results as R  # noqa: E402
import cts_run  # noqa: E402
import dxs_harness as D  # noqa: E402
import gl_soak  # noqa: E402
import kfopen_build as B  # noqa: E402
import kfopen_pins as P  # noqa: E402
from kfopen_common import OpenRowError, Runner, Trust, add_trust_args, check_line, load_json, say, write_json  # noqa: E402

EXIT_NOTRUN = 3


def notrun(row_id, reason):
    print("KFNOTRUN %s reason=%s" % (row_id, reason), flush=True)
    return EXIT_NOTRUN


def default_root():
    return os.environ.get("KFOPEN_ROOT") or (B.DEFAULT_ROOT if os.name == "nt" else os.path.join(os.path.expanduser("~"), ".kfopen"))


def can_reach(host="github.com", port=443, timeout=8):
    try:
        socket.create_connection((host, port), timeout).close()
        return True
    except OSError:
        return False


def check_all():
    """Everything that can be checked without Windows: pins policy, caselists, rows against the pins."""
    problems = []
    try:
        pins = P.load_pins()
    except (OpenRowError, KeyError, ValueError) as e:
        return ["PINS %s" % e]
    problems += cts_caselist.check()
    spec = load_json(os.path.join(HERE, "caselist_spec.json"))
    if spec["cts_commit"] != pins["sources"]["cts"]["commit"]:
        problems.append("CASELIST_COMMIT caselist_spec.json names another CTS commit than open_rows.json")
    for rid, row in pins["rows"].items():
        if row["kind"] in ("cts-gl", "cts-vk"):
            path = os.path.join(HERE, *row["caselist"].split("/"))
            if not os.path.exists(path):
                problems.append("ROW %s: caselist %s is missing" % (rid, row["caselist"]))
            if row["caselist"] not in [l["out"] for l in spec["lists"].values()]:
                problems.append("ROW %s: caselist %s is not in caselist_spec.json" % (rid, row["caselist"]))
    return problems


def tools_for(ctx, row):
    return B.ensure_toolchain(ctx, B.ROW_NEEDS[row["kind"]])


def prepare(ctx, row_id):
    """Toolchain, sources, build for one row. Returns what run needs: {exe, cwd}."""
    row = ctx.pins["rows"][row_id]
    kind = row["kind"]
    if kind == "not-run":
        return {}
    if ctx.trust.prebuilt:                                   # the user's own tree: trusted, hash recorded, nothing built here
        return prebuilt_paths(ctx, row)
    tools = tools_for(ctx, row)
    if kind in ("cts-gl", "cts-vk"):
        src = B.fetch_source(ctx, "cts", tools["git"])
        if not os.path.isdir(os.path.join(src, "external", "glslang", "src")):
            B.fetch_cts_externals(ctx, src, tools["python"])
        built = B.build_cts(ctx, [row["binary"]], dict(tools, python=tools["python"]), src)
        exe, cwd = built[row["binary"]]
        B.write_receipt(ctx)
        return {"exe": exe, "cwd": cwd}
    if kind == "gl-soak":
        exe = B.build_soak(ctx, tools)
        B.write_receipt(ctx)
        return {"exe": exe}
    src = B.fetch_source(ctx, "dxs", tools["git"])
    exe = B.build_dxs(ctx, row["sample"], tools, src)
    B.write_receipt(ctx)
    return {"exe": exe, "cwd": os.path.dirname(exe)}


def prebuilt_paths(ctx, row):
    root = ctx.trust.prebuilt
    if row["kind"] in ("cts-gl", "cts-vk"):
        spec = ctx.pins["cts_build"]["targets"][row["binary"]]
        exe = B.locate(root, spec["exe"])
        cwd = os.path.join(root, *spec["cwd"].split("/"))
    elif row["kind"] == "gl-soak":
        exe, cwd = B.locate(root, "kf_gldual.exe"), root
    else:
        exe = B.locate(root, os.path.basename(ctx.pins["dxs_samples"][row["sample"]]["exe"]))
        cwd = os.path.dirname(exe) if exe else root
    if not exe:
        raise OpenRowError("PREBUILT_MISSING no binary for this row below %s" % root)
    from kfopen_common import sha256_file
    ctx.receipt["builds"]["prebuilt:" + os.path.basename(exe)] = {"exe": exe, "sha256": sha256_file(exe), "verified": "trusted-by-user"}
    return {"exe": exe, "cwd": cwd}


def preflight(ctx, row_id, dry_run=False):
    """A named NOT_RUN reason, or None. The offline gate guest cannot build: without a prepared tree the row is NOT_RUN, never a pass."""
    row = ctx.pins["rows"][row_id]
    if row["kind"] == "not-run":
        return "%s (%s)" % (row["reason"], row["detail"])
    if not sys.platform.startswith("win") and not dry_run:
        return "not-windows (the rows run in a Windows guest or on native Windows)"
    if not ctx.trust.prebuilt and not os.path.isdir(ctx.p("build")) and not os.path.isdir(ctx.p("src")) and not can_reach():
        return "offline-guest-without-build-tree (a gate guest is offline: build with `kfopen.py prepare` on an online LTSC guest, or pass --prebuilt DIR)"
    return None


def run_row(ctx, row_id, args):
    row = ctx.pins["rows"][row_id]
    kind = row["kind"]
    why = preflight(ctx, row_id)
    if why:
        return notrun(row_id, why)
    out = args.out or ctx.p("results", row_id)
    os.makedirs(out, exist_ok=True)
    try:
        paths = prepare(ctx, row_id)
    except OpenRowError as e:
        say("PREPARE_FAILED", str(e))
        check_line("prepare_" + row_id, False, str(e))
        return 1
    runner = ctx.runner
    if kind in ("cts-gl", "cts-vk"):
        caselist = os.path.join(HERE, *row["caselist"].split("/"))
        control = args.control or default_control(ctx, row_id)
        return cts_run.execute_row(row_id, row, ctx.pins["sources"]["cts"]["commit"] if not ctx.trust.commit_overrides.get("cts") else ctx.trust.commit_overrides["cts"],
                                   caselist, paths["exe"], paths["cwd"], runner, out, role=args.role, control_path=control,
                                   allow_different_gpu=args.allow_different_gpu)
    if kind == "gl-soak":
        return gl_soak.run_soak(row_id, row, runner, paths["exe"], smoke=args.smoke, share=args.share)
    if kind == "dx-window":
        try:
            popen = subprocess.Popen([paths["exe"]], cwd=paths["cwd"])
        except OSError as e:
            check_line("sample_started_window", False, "could not start %s: %s" % (paths["exe"], e))
            say("VERDICT", "FAIL row=%s failed=sample_started_window" % row_id)
            return 1
        facts = D.run_window_sample(row, popen, D.Win32())
        write_json(os.path.join(out, row_id + "." + args.role + ".json"), facts)
        checks = D.judge_window(row, facts)
        if args.control:
            ctl = load_json(args.control)
            same = [t for _, t in ctl.get("titles", [])] == [t for _, t in facts.get("titles", [])]
            checks.append(("equals_control", same, "window titles differ from the native control"))
        return D.report(row_id, row, checks, facts)
    if kind == "dx-console":
        res = runner.run([paths["exe"]], cwd=paths["cwd"], timeout=row["timeout_s"])
        control = load_json(args.control)["uav"] if args.control else None
        checks, status, uav = D.judge_console(row, res.rc, res.out, control)
        write_json(os.path.join(out, row_id + "." + args.role + ".json"), {"uav": uav, "rc": res.rc})
        if status == "notrun":
            return notrun(row_id, "work-graphs-tier-not-supported-by-this-adapter (the sample reports it and exits)")
        for n, ok, w in checks:
            check_line(n, ok, w)
        say("VERDICT", ("PASS" if status == "ok" else "FAIL") + " row=%s uav=%d" % (row_id, len(uav)))
        return 0 if status == "ok" else 1
    return 2


def default_control(ctx, row_id):
    p = ctx.p("control", row_id + ".tsv")
    return p if os.path.exists(p) else None


def build_parser():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    sub.add_parser("check")
    for name in ("plan", "prepare", "run"):
        s = sub.add_parser(name)
        s.add_argument("rows", nargs="+" if name != "plan" else 1)
        s.add_argument("--root", default=None, help="working root (tools, sources, builds, results); default %s" % B.DEFAULT_ROOT)
        add_trust_args(s)
        if name == "run":
            s.add_argument("--role", choices=("guest", "native"), default="guest", help="native: write the control the guest run is compared with")
            s.add_argument("--control", help="the native control (results file of a CTS row, JSON of a sample); default <root>/control/<row>.tsv")
            s.add_argument("--out", help="result directory (default <root>/results/<row>)")
            s.add_argument("--smoke", action="store_true", help="the short soak (60 s)")
            s.add_argument("--share", action="store_true", help="soak: let the two GL contexts share objects")
            s.add_argument("--allow-different-gpu", action="store_true", help="compare with a control from another GPU model (recorded)")
    d = sub.add_parser("diff")
    d.add_argument("--native", required=True)
    d.add_argument("--guest", required=True)
    d.add_argument("--out")
    return ap


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        if args.cmd == "list":
            pins = P.load_pins()
            for rid, row in pins["rows"].items():
                print("%-30s %-10s %s" % (rid, row["kind"], row.get("sample") or row.get("binary") or ""))
            return 0
        if args.cmd == "check":
            problems = check_all()
            for p in problems:
                print("PROBLEM", p)
            print("OPEN_ROWS_OK" if not problems else "OPEN_ROWS_FAILED problems=%d" % len(problems))
            return 1 if problems else 0
        if args.cmd == "diff":
            nh, n = R.read_results(open(args.native, encoding="utf-8").read())
            gh, g = R.read_results(open(args.guest, encoding="utf-8").read())
            d = R.diff_results(n, g)
            text = R.format_diff(d)
            if args.out:
                open(args.out, "w", encoding="utf-8", newline="\n").write(text)
            say("DIFF", "verdict=%s cases=%d candidates=%d %s" % (d["verdict"], d["cases"], len(d["candidates"]), " ".join("%s=%d" % kv for kv in sorted(d["counts"].items()))))
            return {"PASS": 0, "FAIL": 1}.get(d["verdict"], EXIT_NOTRUN)
        pins = P.load_pins()
        trust = Trust.from_args(args)
        ctx = B.Ctx(pins, root=args.root or default_root(), runner=Runner(), trust=trust)
        if args.cmd == "plan":
            print(json.dumps(B.plan(pins, args.rows[0]), indent=1))
            return 0
        rc = 0
        for rid in args.rows:
            if rid not in pins["rows"]:
                raise OpenRowError("unknown row %s (kfopen.py list)" % rid)
            if args.cmd == "prepare":
                why = preflight(ctx, rid)
                if why:
                    rc = max(rc, notrun(rid, why))
                    continue
                prepare(ctx, rid)
                say("PREPARED", rid)
            else:
                rc = max(rc, run_row(ctx, rid, args))
        return rc
    except (OpenRowError, R.OpenRowError) as e:
        print("KFOPEN REFUSED %s" % e, file=sys.stderr, flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
