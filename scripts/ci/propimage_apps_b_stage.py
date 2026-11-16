#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The second proprietary image's application rows as a payload stage: same runner and image format as `propimage_apps.py` (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-16. GPU-free tests only (test_propimage_apps_b_stage.py); not run on a box. It refuses until its three pins
(image archive, signed boot GOP, open-asset data volume) are set (all three are, as of 2026-11-16). Wiring into vast_payload.py / vast_report.py is NOT done here: those files
are being changed by the release-run work (#434 stack); see docs/design/V3_PROPIMAGE_B_APPS.md for the exact additions.

    propimage_apps_b_stage.py run --sha SHA --checkout ROOT --prefix DIR --out FILE --urls FILE [--bench DIR] [--deadline-epoch N]
    propimage_apps_b_stage.py plan [--sha SHA]

It reuses `propimage_apps.py` for everything that does not depend on the application list (preflight rows, pins and manifest, swtpm,
the verified unpack, the offline proof, the host kernel log windows, cleanup) and replaces three things: the application list and its
timeouts (`scripts/bench/windows/propimage_apps_b.py`, selected with `propimage_check.py --app-set b`), the boot command (adds
`--data-iso`), and the grading (PASS or PASS_LIMITED per row; a PASS row names the GPU engines it was asserted on and carries the sampled
peaks). One-GPU boxes: the image B leg is a separate box with the same instruction shape (a leg selector A|B).
A row that only proves a launch is PASS_LIMITED and the stage says so; the stage verdict is PASS when no row failed.
The record is printed as `KF_PROPIMAGE_APPS_B_RESULT {json}` (scores and sampled GPU numbers are written to the record, not to the log).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import propimage_apps as pa  # noqa: E402

PINS_PATH = HERE / "propimage_apps_b_pins.json"
SCHEMA = 1
APP_VERDICTS = ("PASS", "PASS_LIMITED", "FAIL", "INCONCLUSIVE", "HARNESS", "NOT_RUN", "MISSING")
BOOT_BUDGET_S = pa.BOOT_BUDGET_S
DATA_ROLE = "data"


def apps_module(root):
    """scripts/bench/windows/propimage_apps_b.py as a module (the application table and the grading)."""
    return pa.load_module(Path(root) / "scripts/bench/windows/propimage_apps_b.py", "kf_propimage_apps_b_for_stage")


def plan_boots(timeouts, order, budget=BOOT_BUDGET_S):
    """Greedy groups in table order whose timeouts sum to at most `budget` (the evaluation timer is per boot)."""
    boots, cur, total = [], [], 0
    for app in order:
        t = timeouts[app]
        if cur and total + t > budget:
            boots.append(cur)
            cur, total = [], 0
        cur.append(app)
        total += t
    if cur:
        boots.append(cur)
    return boots


def boot_command(check_py, a):
    cmd = [sys.executable, "-B", "-I", str(check_py), "--dir", a["dir"], "--work", a["work"], "--out", a["out"], "--qemu", a["qemu"],
           "--flags", a["flags"], "--broker", a["broker"], "--gop-efi", a["gop"], "--display", "broker", "--app-set", "b",
           "--data-iso", a["data"], "--apps", ",".join(a["apps"])]
    return cmd


def collect_app(out, app, rows, xids, expected_gpu):
    """One row's record from the harness rows and its verdict.json; both must agree for a PASS or PASS_LIMITED."""
    rec = {"app": app, "verdict": "MISSING", "kind": "NONE", "limited": False, "secs": 0, "detail": "", "evidence": [], "gpu": {},
           "host_xid": {"count": len(xids), "lines": [t for _, t in xids[:pa.MAX_XID_LINES]]}}
    step = next((r for r in rows if type(r) is dict and r.get("step") == "app:" + app), None)
    dirs = pa.run_dirs(out, app)
    doc = None
    if len(dirs) == 1:
        try:
            doc = pa.read_json(dirs[0] / "verdict.json")
            rec["evidence"] = pa.evidence_files(dirs[0])
        except (OSError, ValueError):
            doc = None
    if step is None:
        rec["detail"] = "no app row from the harness"
        return rec
    said = pa.clean(step.get("verdict"), 20)
    rec["verdict"] = said if said in APP_VERDICTS else "FAIL"
    rec["detail"] = pa.clean(step.get("detail"), 200)
    kind = pa.clean(step.get("evidence"), 8)
    rec["kind"] = kind if kind in ("FILE", "UIA", "NONE") else "NONE"
    if type(doc) is dict:
        secs = doc.get("secs")
        rec["secs"] = secs if type(secs) is int and 0 <= secs < 1 << 20 else 0
        g = doc.get("gpu") if type(doc.get("gpu")) is dict else {}
        rec["gpu"] = {"required": [pa.clean(x, 8) for x in (g.get("required") or [])][:4], "verdict": pa.clean(g.get("verdict"), 8),
                      "peak": {pa.clean(k, 16): v for k, v in list((g.get("peak") or {}).items())[:12]
                               if type(v) in (int, float) or v is None}}
    if rec["verdict"] in ("PASS", "PASS_LIMITED"):
        limited = rec["verdict"] == "PASS_LIMITED"
        proven = (type(doc) is dict and doc.get("app") == app and doc.get("ok") is True and len(dirs) == 1
                  and bool(doc.get("limited")) == limited and doc.get("evidence") == rec["kind"] and rec["kind"] in ("FILE", "UIA"))
        if proven and expected_gpu:
            # an asserted row must carry a PASS GPU record that names every engine it was required to use
            proven = rec["gpu"].get("verdict") == "PASS" and set(expected_gpu) <= set(rec["gpu"].get("required", []))
        if not proven:
            rec["verdict"], rec["detail"] = "FAIL", "UNPROVEN_PASS: " + rec["detail"][:150]
        rec["limited"] = limited
    return rec


def grade(apps, order):
    """(verdict, reason, failed). PASS needs every row PASS or PASS_LIMITED."""
    by = {a["app"]: a for a in apps}
    failed = [n for n in order if by.get(n, {}).get("verdict") not in ("PASS", "PASS_LIMITED")]
    if not failed:
        return "PASS", "", []
    app_class = [n for n in failed if by.get(n, {}).get("verdict") in ("FAIL", "INCONCLUSIVE")]
    if app_class:
        return "FAIL", "APP_FAIL:" + app_class[0], failed
    return "RETRY", "HARNESS:" + failed[0], failed


def new_record(ctx):
    return {"schema": SCHEMA, "image_set": "b", "verdict": "RETRY", "sha": ctx.sha, "reason": "INTERRUPTED", "family": None, "cpu_vendor": None,
            "preflight": [], "image": {"name": None, "sha256": None, "size": None}, "swtpm": {"version": None, "built": False},
            "boots": [], "apps": [], "failed": [], "limited": []}


def build_manifest(pins, build_swtpm):
    """pa.build_manifest plus the data volume (role `data`, a signed or public ref like the image)."""
    unset = pa.unset_roles(pins, (DATA_ROLE,))
    if unset:
        raise pa.Refused("PIN_UNSET:" + DATA_ROLE)
    m = pa.build_manifest(pins, build_swtpm)
    it = next(i for i in pins["items"] if i["role"] == DATA_ROLE)
    source = {"kind": "signed", "ref": it["ref"]} if "ref" in it else {"kind": "mirror", "url": it["mirror"]}
    m["items"].append({"name": it["name"], "dest": it["dest"], "sha256": it["sha256"], "size": it["size"], "needed_by": "image", "sources": [source]})
    return m


def run(ctx, evidence_dir=None, apps=None):
    rec = new_record(ctx)
    klog, started, out_of_time = None, False, False
    try:
        pa.check_prefix(ctx.prefix)
        ctx.pins_path = Path(PINS_PATH if ctx.pins_path == pa.PINS_PATH else ctx.pins_path)
        rows, family, build, pins, vendor = pa.preflight(ctx)
        rec["preflight"], rec["family"], rec["cpu_vendor"] = rows, family, vendor
        image = next(i for i in pins["items"] if i["role"] == "image")
        rec["image"] = {"name": image["name"], "sha256": image["sha256"], "size": image["size"]}
        refusal = pa.refusal_of(rows)
        if not refusal and pa.unset_roles(pins, (DATA_ROLE,)):
            refusal = ("REFUSED", "PIN_UNSET:" + DATA_ROLE)
        if refusal:
            rec["verdict"], rec["reason"] = refusal
            return rec
        pi_check = pa.harness(ctx.checkout)
        mod = apps_module(ctx.checkout)
        order = tuple(a for a in mod.CHECKS if not apps or a in apps)
        if not order:
            raise pa.Refused("APPS")
        started = True
        klog = pa.KernelLog(ctx.prefix / "kernel", ctx.klog_source)
        klog.start()
        timeouts = {a: mod.CHECKS[a]["timeout"] for a in order}
        plan = plan_boots(timeouts, order)
        rec["boots"] = [{"index": i + 1, "apps": b, "rc": None, "secs": 0, "offline": False} for i, b in enumerate(plan)]
        work, dl, imgdir, logs = ctx.prefix / "work", ctx.prefix / "dl", ctx.prefix / "unpacked", ctx.prefix / "logs"
        for d in (work, dl, imgdir, logs):
            d.mkdir(parents=True, exist_ok=True)
        manifest = build_manifest(pins, build)
        manifest_path = ctx.prefix / "manifest.json"
        manifest_path.write_text(json.dumps(manifest))
        pa.fetch(ctx, manifest_path, dl, logs)
        if build:
            env = pa.build_swtpm(ctx, dl, work, logs)
            ctx.environ.update(SWTPM=pa.SWTPM_PREFIX + "/bin/swtpm", LD_LIBRARY_PATH=env["LD_LIBRARY_PATH"])
            rec["swtpm"]["built"] = True
        have_version = pa.swtpm_version(pa.swtpm_binary(ctx.environ), ctx.runner, ctx.environ)
        rec["swtpm"]["version"] = ".".join(str(x) for x in have_version) if have_version else None
        broker = ctx.checkout / "broker/nvkvm-display-broker"
        if not broker.is_file() and pa.run_command(ctx, ["make", "-C", str(ctx.checkout / "broker")], logs / "broker_build.log", 900) != 0:
            raise pa.Refused("NO_BROKER_BUILD")
        try:
            (ctx.unpack or pi_check.load_image)(str(dl / "image/image.zip"), str(imgdir))
        except Exception as exc:  # noqa: BLE001 - same set as the first stage
            rec["error"] = pa.clean(exc, 120)
            raise pa.Refused("UNPACK")
        (dl / "image/image.zip").unlink(missing_ok=True)
        data_item = next(i for i in pins["items"] if i["role"] == DATA_ROLE)
        flags = ctx.checkout / "scripts/bench/windows/kf3_flags.sample.env"
        for boot in rec["boots"]:
            label = "boot%d" % boot["index"]
            need = sum(timeouts[a] for a in boot["apps"])
            if ctx.deadline_epoch and time.time() + need + 900 > ctx.deadline_epoch:
                boot["rc"] = 125
                out_of_time = True
                continue
            out, wk = ctx.prefix / "out" / label, work / label
            a = {"dir": str(imgdir), "work": str(wk), "out": str(out), "qemu": str(ctx.qemu), "flags": str(flags), "broker": str(broker),
                 "gop": str(dl / "image/kf-gop.signed.efi"), "data": str(dl / data_item["dest"]), "apps": boot["apps"]}
            klog.mark(label, "start")
            t0 = time.monotonic()
            mark = pa.Mark(klog, label, boot["apps"])
            (ctx.prefix / "out").mkdir(exist_ok=True)
            boot["rc"] = (ctx.boot_runner or pa.run_boot)(ctx, boot_command(ctx.check_py, a), logs / (label + ".log"), need + pa.BOOT_SLACK_S, mark)
            boot["secs"] = int(time.monotonic() - t0)
            klog.mark(label, "end")
            boot["offline"] = pa.offline_proof(wk)
            try:
                rows_json = pa.read_json(out / "result.json")
                rows_json = rows_json if type(rows_json) is list else []
            except (OSError, ValueError):
                rows_json = []
            boot["harness"] = [{"step": pa.clean(r.get("step"), 24), "verdict": pa.clean(r.get("verdict"), 12), "detail": pa.clean(r.get("detail"), 120)}
                               for r in rows_json if type(r) is dict and r.get("class") == "harness" and r.get("verdict") not in ("PASS", "OK")][:6]
            per, unattributed = pa.xid_by_window(klog, boot["apps"], label)
            for app in boot["apps"]:
                item = collect_app(out, app, rows_json, per[app], mod.CHECKS[app]["gpu"])
                if not boot["offline"] and item["verdict"] in ("PASS", "PASS_LIMITED"):
                    item["verdict"], item["detail"] = "FAIL", "OFFLINE_NOT_PROVEN: " + item["detail"][:150]
                rec["apps"].append(item)
            if unattributed:
                boot["host_xid_unattributed"] = {"count": len(unattributed), "lines": [t for _, t in unattributed[:pa.MAX_XID_LINES]]}
            copy_evidence(out, evidence_dir, order) if evidence_dir else None
            pa.write_record(ctx.out, rec)
        have = {a["app"] for a in rec["apps"]}
        for app in order:
            if app not in have:
                rec["apps"].append(dict(collect_app(ctx.prefix / "none", app, [], [], ()), verdict="NOT_RUN", detail="boot not started"))
        rec["apps"].sort(key=lambda a: order.index(a["app"]))
        rec["verdict"], rec["reason"], rec["failed"] = grade(rec["apps"], order)
        rec["limited"] = [a["app"] for a in rec["apps"] if a["verdict"] == "PASS_LIMITED"]
        if rec["verdict"] == "PASS" and any(not b["offline"] for b in rec["boots"]):
            rec["verdict"], rec["reason"], rec["failed"] = "FAIL", "OFFLINE_NOT_PROVEN", []
        if out_of_time and rec["verdict"] != "PASS":
            rec["verdict"], rec["reason"] = "RETRY", "TIME"
    except pa.Refused as r:
        rec["verdict"], rec["reason"] = "REFUSED", r.reason
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as exc:
        rec["verdict"], rec["reason"], rec["error"] = "REFUSED", "INTERNAL", pa.clean(exc, 120)
    finally:
        if klog is not None:
            klog.stop()
        if started:
            pa.cleanup(ctx)
    return rec


# text evidence only: the verdicts, the sampled GPU numbers and the diagnostic output; never a PNG, PDF, PSD or video
EVIDENCE_COPY = ("verdict.json", "result.json", "steps.log")
EVIDENCE_GLOB = re.compile(r"[a-z0-9]+_gpu(\.json|_query\.csv|_dmon\.txt)|sniffer(_[a-z]+)?\.txt")


def copy_evidence(out, evidence_dir, order):
    for app in order:
        for d in pa.run_dirs(out, app):
            for p in sorted(d.iterdir()):
                if p.is_symlink() or not p.is_file() or p.stat().st_size > 1 << 20:
                    continue
                if p.name in EVIDENCE_COPY or EVIDENCE_GLOB.fullmatch(p.name):
                    dst = Path(evidence_dir) / app
                    dst.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(p, dst / p.name)


def finish(rec, out):
    data = pa.write_record(out, rec) if out else json.dumps(rec, separators=(",", ":"), sort_keys=True)
    print("KF_PROPIMAGE_APPS_B_RESULT " + data, flush=True)
    return 0 if rec["verdict"] in ("PASS", "NOT_RUN") else (3 if rec["verdict"] == "RETRY" else 1)


def main(argv=None):
    p = argparse.ArgumentParser(prog="propimage_apps_b_stage.py")
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--sha", required=True)
    r.add_argument("--checkout", default=str(pa.ROOT))
    r.add_argument("--prefix", default=pa.DEFAULT_PREFIX + "-b")
    r.add_argument("--out", required=True)
    r.add_argument("--urls", default=pa.DEFAULT_URLS)
    r.add_argument("--bench", default="/workspace/bench")
    r.add_argument("--deadline-epoch", type=float, default=0.0)
    r.add_argument("--evidence-dir")
    r.add_argument("--apps", help="comma list: run only these rows (default: all)")
    q = sub.add_parser("plan")
    q.add_argument("--sha", default="0" * 40)
    a = p.parse_args(argv)
    try:
        if a.cmd == "plan":
            mod = apps_module(pa.ROOT)
            order = tuple(mod.CHECKS)
            print(json.dumps({"apps": order, "boots": plan_boots({n: mod.CHECKS[n]["timeout"] for n in order}, order)}, indent=1))
            return 0
        if not pa.SHA_RE.fullmatch(a.sha):
            return 2
        ctx = pa.Context(a.sha, a.checkout, a.prefix, a.out, a.urls, bench=Path(a.bench), deadline_epoch=a.deadline_epoch, hardware=pa.hardware_now())
        ctx.pins_path = PINS_PATH
        return finish(run(ctx, a.evidence_dir, a.apps.split(",") if a.apps else None), a.out)
    except pa.Refused as exc:
        print("KF_PROPIMAGE_APPS_B_REFUSED " + pa.clean(exc.reason, 80), flush=True)
        return 2
    except (ValueError, OSError) as exc:
        print("KF_PROPIMAGE_APPS_B_USAGE " + pa.clean(exc, 100), flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
