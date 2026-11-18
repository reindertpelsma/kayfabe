#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-PV gap probes: run the case table on this machine, or compare a host run with a guest run (stdlib only).

STATUS: RESEARCH, 2026-11-18 (design: docs/design/V3_GPUPV_GAP_MATRIX.md section 3; decisions are GPU-free tested in scripts/ci/test_gap_probes.py). Not yet run on a GPU.

    gap_probes.py run --side host|guest --out DIR [--only GLOB,GLOB] [--probe PATH] [--egl PATH] [--destructive] [--terminal] [--emit]
    gap_probes.py compare HOST GUEST [--xid FILE]      HOST and GUEST are a run directory or a row log with --emit blocks; prints only the differing cases
    gap_probes.py list
    gap_probes.py hoststate watch --out FILE           (on the HOST while a guest runs smi_setters) stream the host's GPU control state until SIGTERM
    gap_probes.py hoststate verdict --csv FILE         HOST_STATE_LEAK=1 when the state changed during the window

`run` writes DIR/results.tsv (id group status rc secs t0 t1 reason canary), DIR/kv.tsv (case key value), DIR/summary.txt, DIR/logs/. Each `c` case is one fresh process of
gap_probe (built from gap_probe.c with gcc when no binary is given). The last line printed is `GAP_PROBES_SUMMARY ...`. Exit status 0 when nothing FAILED or MISMATCHED.
"""
import argparse
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "open_rows"))
import diffrun  # noqa: E402
import gap_lib as L  # noqa: E402
import gap_scripts as S  # noqa: E402
import rowio  # noqa: E402

CASES = HERE / "gap_cases.tsv"
LIMITS = HERE / "gap_known_limits.tsv"
RESULT_COLS = ("id", "group", "status", "rc", "secs", "t0", "t1", "reason", "canary")


def build_probe(out):
    """-> path of a usable gap_probe, or None. Only gcc and the source in this directory: no toolkit, no header."""
    exe = out / "gap_probe"
    if shutil.which("gcc") is None:
        return None
    rc, text, _ = S.run(["gcc", "-O2", "-o", str(exe), str(HERE / "gap_probe.c"), "-ldl"], 120)
    return str(exe) if rc == 0 else None


def run_c_case(probe, case, log_dir):
    env = dict(os.environ, **case["env"])
    t0 = time.time()
    rc, out, to = S.run([probe, case["prog"]], case["timeout_s"], env=env)
    (log_dir / (case["id"] + ".log")).write_text(out[-8000:])
    parsed = L.parse_probe_output(out, case["prog"])
    status, reason = L.classify_process(rc, to, parsed)
    return {"status": status, "reason": reason, "kv": parsed["kv"] if parsed else {}, "rc": parsed["rc"] if parsed else rc, "secs": time.time() - t0}


def run_all(a):
    out = Path(a.out)
    (out / "logs").mkdir(parents=True, exist_ok=True)
    cases = L.load_cases(CASES.read_text())
    run, skipped = L.select(cases, a.side, a.only, a.destructive, a.terminal)
    probe = a.probe or build_probe(out)
    ctx = {"probe": probe, "out": out, "egl": a.egl}
    rows, kvs, dead = [], [], 0
    stop_reason = None
    for case in run:
        t0 = time.time()
        if stop_reason:
            res = {"status": "NOT_RUN", "reason": stop_reason, "kv": {}, "rc": 0, "secs": 0}
        elif probe is None:
            res = {"status": "NOT_RUN", "reason": "no_gap_probe_binary_and_no_gcc", "kv": {}, "rc": 0, "secs": 0}
        elif case["kind"] == "c":
            res = run_c_case(probe, case, out / "logs")
        else:
            try:
                r = S.CASES[case["prog"]](ctx)
            except Exception as e:   # a harness bug is a result too, named as such
                r = S.result("FAIL", "harness_exception_%s" % type(e).__name__)
            res = dict(r, rc=0, secs=time.time() - t0)
        t1 = time.time()
        canary = "-"
        if probe and res["status"] in ("FAIL", "MISMATCH") and case["id"] != "canary":
            run_it, stop = L.canary_policy(res["status"], res["reason"], dead)
            if run_it:
                ok = S.canary(ctx)
                canary = "PASS" if ok else "FAIL"
                dead = 0 if ok else dead + 1
                if stop and not ok:
                    stop_reason = "device_dead_after_" + case["id"]
        rows.append({"id": case["id"], "group": case["group"], "status": res["status"], "rc": res["rc"], "secs": "%.1f" % res["secs"], "t0": "%.3f" % t0,
                     "t1": "%.3f" % t1, "reason": res["reason"], "canary": canary})
        kvs += [(case["id"], k, v) for k, v in sorted(res["kv"].items())]
        print("GAPCASE %s %s %s" % (case["id"], res["status"], res["reason"]), flush=True)
    for case, why in skipped:
        rows.append({"id": case["id"], "group": case["group"], "status": "NOT_RUN", "rc": 0, "secs": "0.0", "t0": "0", "t1": "0", "reason": why, "canary": "-"})
    results = "\t".join(RESULT_COLS) + "\n" + "".join("\t".join(str(r[c]) for c in RESULT_COLS) + "\n" for r in rows)
    kvtxt = "".join("%s\t%s\t%s\n" % k for k in kvs)
    count = {s: sum(1 for r in rows if r["status"] == s) for s in L.STATUSES}
    summary = "GAP_PROBES_SUMMARY side=%s pass=%d not_supported=%d fail=%d mismatch=%d not_run=%d" % (
        a.side, count["PASS"], count["NOT_SUPPORTED"], count["FAIL"], count["MISMATCH"], count["NOT_RUN"])
    for name, text in (("results.tsv", results), ("kv.tsv", kvtxt), ("summary.txt", summary + "\n")):
        (out / name).write_text(text)
        if a.emit:
            sys.stdout.write(rowio.emit(name, text))
    print(summary)
    return 0 if count["FAIL"] == 0 and count["MISMATCH"] == 0 else 1


def compare(a):
    hr, gr = rowio.load(a.host, "results.tsv"), rowio.load(a.guest, "results.tsv")
    if hr is None or gr is None:
        print("GAP_COMPARE error=missing_results_tsv host=%s guest=%s" % (hr is not None, gr is not None))
        return 2
    host, guest = diffrun.parse_gap_results(hr), diffrun.parse_gap_results(gr)
    diffrun.parse_gap_kv(rowio.load(a.host, "kv.tsv") or "", host)
    diffrun.parse_gap_kv(rowio.load(a.guest, "kv.tsv") or "", guest)
    limits = diffrun.parse_limits(LIMITS.read_text()) if LIMITS.exists() else []
    xids = diffrun.parse_xids(Path(a.xid).read_text()) if a.xid else []
    diffs = diffrun.compare(host, guest, limits, xids)
    print("\n".join(diffrun.diff_lines(diffs)))
    hv, gv = rowio.load(a.host, "vk_leaves.tsv"), rowio.load(a.guest, "vk_leaves.tsv")
    if hv and gv:
        vd = L.vulkan_diff(hv.split(), gv.split())
        for side in ("only_host", "only_guest"):
            for leaf in vd[side][:20]:
                print("VKDIFF %s %s" % (side, leaf))
        print("VKDIFF_SUMMARY only_host=%d only_guest=%d" % (vd["n_only_host"], vd["n_only_guest"]))
    line = diffrun.summary_line(host, guest, diffs)
    print(line.replace("DIFFRUN_SUMMARY", "GAP_COMPARE"))
    return 1 if any(d["bucket"] == "kayfabe_gap_candidate" for d in diffs) else 0


def hoststate(a):
    if a.sub == "verdict":
        samples = [tuple(l.split(";")[1:]) for l in Path(a.csv).read_text().splitlines() if ";" in l]
        leaked, distinct = L.host_state_changed(samples)
        print("HOST_STATE_LEAK=%d samples=%d distinct=%d" % (leaked, len(samples), len(distinct)))
        return 1 if leaked else 0
    argv = ["nvidia-smi", "--query-gpu=" + S.SMI_FIELDS, "--format=csv,noheader,nounits", "-i", "0", "-lms", "500"]
    rows, stop = [], threading.Event()
    p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())

    def reader():
        for line in p.stdout:
            rows.append("%.3f;%s" % (time.time(), ";".join(x.strip() for x in line.strip().split(","))))
    threading.Thread(target=reader, daemon=True).start()
    stop.wait()
    p.terminate()
    time.sleep(0.2)
    Path(a.out).write_text("\n".join(rows) + "\n")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--side", required=True, choices=("host", "guest"))
    r.add_argument("--out", required=True)
    r.add_argument("--only")
    r.add_argument("--probe")
    r.add_argument("--egl")
    r.add_argument("--destructive", action="store_true")
    r.add_argument("--terminal", action="store_true")
    r.add_argument("--emit", action="store_true")
    c = sub.add_parser("compare")
    c.add_argument("host")
    c.add_argument("guest")
    c.add_argument("--xid")
    sub.add_parser("list")
    h = sub.add_parser("hoststate")
    h.add_argument("sub", choices=("watch", "verdict"))
    h.add_argument("--out")
    h.add_argument("--csv")
    a = ap.parse_args(argv)
    if a.cmd == "list":
        for case in L.load_cases(CASES.read_text()):
            print(case["id"])
        return 0
    return {"run": run_all, "compare": compare, "hoststate": hoststate}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
