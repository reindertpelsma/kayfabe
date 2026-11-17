#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The Linux app matrix, GPU-use side (stdlib only): which rows have a CPU path, what each must prove, and the three verbs the shell glue calls.

STATUS: LIVE, 2026-11-17 (owner requirement of 2026-11-17; design and audit: docs/design/V3_GPU_USE_ASSERTION.md; judge: scripts/bench/gpu_use.py).
GPU-free tested (scripts/ci/test_gpu_use.py, mutants scripts/ci/gpu_use_mutants.py); NOT run on hardware when this line was written.

    gpu_use_rows.py list                                           the class of every row of run_apps.sh and what a class (b) row must prove
    gpu_use_rows.py sample --out DIR --app A [--interval-ms N] -- CMD...    run CMD while the BOX's nvidia-smi is sampled (host-authored), exit with CMD's rc
    gpu_use_rows.py annotate --side guest|host --app A --line "APPRES ..." --app-log F [--kf3-log F --kf3-base F] [--smi-dir D] [--negctl]
    gpu_use_rows.py annotate-res --side host|guest --res FILE --logs DIR [--suffix .log] [--negctl]        every APPRES line of a .res file

Classes (every row of `run_apps.sh list` must have one; test_every_row_is_classified keeps the table complete when a row is added):
  a       cannot run without the GPU: a hard CUDA / driver call with no CPU path. GPU use is implied; the census is RECORDED, never judged.
  a-info  identity probes (nvidia-smi, deviceQuery, vulkaninfo, clinfo): they prove which device exists, not that work ran on it.
  a+proof class (a) by construction, wired anyway: the script also prints a device proof from the framework's own allocator (defence in depth: a later
          edit to `device = "cuda" if ... else "cpu"` or a software encoder would be caught).
  b       CAN silently fall back (Cycles to CPU, llama.cpp -ngl 0, hashcat or clpeak on a CPU OpenCL device, vkpeak/EGL on a software rasteriser, ffmpeg
          software decode): the row's PASS needs the app proof AND (guest side) the host census AND the host engine counters named in ROWS.
  own     the open discovery rows of PRs 471 and 474 (nvcodec_matrix, nvml_dcgm, vkcts): GPU use is asserted inside their own driver from the host's counters;
          recorded here, never judged here (verdict NA).
  b-skip  class (b) and not run by policy (geekbench_gpu): the proof is required before the policy skip is ever lifted.
A PASS is rewritten to FAIL (reason named) when the judge says FAIL and to NOTRUN when it says NOT_RUN: a row never passes without its proof. A row that
was not PASS stays as it was. Negative controls: KF_NEGCTL=1 makes run_apps.sh run the CPU form of each class (b) row; `--negctl` judges that run and
prints CONTROL_OK when it was judged FAIL, CONTROL_BROKEN when the judge passed a CPU run.
"""
import argparse
import importlib.util
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("kf_gpu_use", HERE.parent / "bench" / "gpu_use.py")
gu = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gu)

MAX_FILE = 1 << 21
APPRES_KEYS = re.compile(r"(\w+)=((?:(?! \w+=).)*)")

# row -> (class, proof kind, proof params, host engines the sampler must show busy, sampler interval ms, min busy samples, app_missing)
# Engines and min_samples are UNCALIBRATED (no hardware run when written): the validation request is section 6 of the design doc. A real passing GPU row
# that is judged FAIL means a named constant here is wrong, to be changed here with the measurement beside it; never the judge relaxed.
SHORT = {"interval_ms": 100, "min_samples": 1}           # rows that keep the GPU busy for a second or less: sample fast, one busy sample is enough
LONG = {"interval_ms": 250, "min_samples": 2}
ROWS = {
    "blender_cycles": ("b", "blender", {"devs": ("CUDA", "OPTIX")}, ("gpu", "mem"), dict(SHORT), "FAIL"),
    "llama_cpp_gen": ("b", "llama", {"mode": "gen"}, ("gpu",), dict(SHORT), "FAIL"),
    "llama_bench": ("b", "llama", {"mode": "bench"}, ("gpu",), dict(SHORT), "FAIL"),
    "hashcat": ("b", "hashcat", {}, (), dict(SHORT), "FAIL"),
    "clpeak": ("b", "opencl_bench", {}, ("gpu",), dict(LONG), "FAIL"),
    "vkpeak": ("b", "vkpeak", {}, ("gpu",), dict(LONG), "FAIL"),
    "egl_offscreen": ("b", "egl", {}, (), dict(SHORT), "FAIL"),
    "nvdec_h264": ("b", "nvdec", {}, (), dict(SHORT), "NOT_RUN"),
    "geekbench_gpu": ("b-skip", None, {}, (), dict(LONG), "FAIL"),
    "nvenc_h264": ("a+proof", "nvenc", {"codec": "h264"}, ("enc",), dict(SHORT), "FAIL"),
    "nvenc_hevc": ("a+proof", "nvenc", {"codec": "hevc"}, ("enc",), dict(SHORT), "FAIL"),
    "torch_correct": ("a+proof", "framework", {"framework": "torch", "min_mib": 1000}, ("gpu",), dict(LONG), "FAIL"),
    "torch_ai_bench": ("a+proof", "framework", {"framework": "torch", "min_mib": 500}, ("gpu",), dict(LONG), "FAIL"),
    "hf_generate": ("a+proof", "framework", {"framework": "torch", "min_mib": 400}, ("gpu",), dict(SHORT), "FAIL"),
    "cupy": ("a+proof", "framework", {"framework": "cupy", "min_mib": 4}, (), dict(SHORT), "FAIL"),
}
INFO = ("nvidia_smi", "deviceQuery", "vulkaninfo", "clinfo")
# Open discovery rows of PRs 471 and 474 that assert GPU use INSIDE their own driver, from the host's counters (`open_rows/census.py`: `host_gpu_use=PROVEN|UNPROVEN|FAILED`;
# `vkcts.py`: `VKCTS_GPU`, exit 4 on an idle GPU). They are recorded here (census, never judged: verdict NA) and are not rewritten by this module; promoting one into ROWS is
# the follow-up that gives it a proof kind and calibrated engines.
OWN = ("nvcodec_matrix", "nvml_dcgm", "vkcts")
WIRED = tuple(r for r, v in ROWS.items() if v[0] in ("b", "a+proof"))      # rows whose verdict the judge may rewrite


def row_class(app):
    if app in ROWS:
        return ROWS[app][0]
    return "a-info" if app in INFO else "own" if app in OWN else "a"


def host_engines(app, side):
    """The host observables a row must show: `mem` (host video memory rises by a floor) is required of a BARE-METAL run only. MEASURED 2026-11-17 (first guest run,
    RTX 3090, blender_cycles): the guest's device memory is the store the host reserves at boot (8 GiB), so a guest allocation never moves host VRAM (8761 -> 8792 MiB
    with 9512 submissions and the GPU 53 percent busy). The census and the other engines stay required of a guest."""
    engines = ROWS[app][3]
    return tuple(e for e in engines if not (side == "guest" and e == "mem"))


def policy(app):
    cls, proof, params, engines, sampler, missing = ROWS[app]
    return {"class": cls, "proof": proof, "params": params, "engines": engines, "interval_ms": sampler["interval_ms"], "min_samples": sampler["min_samples"],
            "app_missing": missing}


def read_text(path):
    try:
        with open(path, "rb") as f:
            return f.read(MAX_FILE).decode("utf-8", "replace")
    except OSError:
        return None


def clean(text, limit=120):
    """A reason for an APPRES note: one line, no `key=` token (the matrix parsers split a line at ` key=`), no pipe (the table separator)."""
    return re.sub(r"\s+", " ", re.sub(r"[=|]", ":", str(text)))[:limit].strip()


def evaluate(app, side, app_text, kf3_lines=None, kf3_base=None, smi=None):
    """The judge's record for one row. Rows of class (a) / (a-info) get the census recorded and are never judged (verdict NA)."""
    cls = row_class(app)
    census = gu.window_census(kf3_lines, kf3_base) if kf3_lines is not None else None
    if cls not in ("b", "a+proof"):
        return {"row": app, "class": cls, "verdict": "NA", "reasons": [], "authority": "none", "census": census, "smi": None, "app": None}
    p = policy(app)
    appp = None if app_text is None else gu.PROOFS[p["proof"]](app_text, **p["params"])
    engines = host_engines(app, side) if (side == "guest" or smi is not None) else ()      # a bare-metal baseline is not sampled unless a sampler record is given
    rec = gu.judge(app, census=census, smi=smi, app=appp, need_census=(side == "guest"), need_engines=engines, side=side, app_missing=p["app_missing"])
    rec["class"], rec["thresholds"] = cls, {"min_samples": p["min_samples"], "interval_ms": p["interval_ms"]}
    return rec


def smi_from_dir(app, directory, side="guest"):
    """The sampler's files of one row -> the gpu_assert record, or None when the row was not sampled (no directory / no files)."""
    if not directory:
        return None
    d = Path(directory)
    q, m = read_text(d / "host_query.csv"), read_text(d / "host_dmon.txt")
    if q is None and m is None:
        return None
    p = policy(app) if app in ROWS else {"engines": (), "min_samples": 1}
    engines = host_engines(app, side) if app in ROWS else ()
    return gu.smi_record(q or "", m or "", engines, min_samples=p["min_samples"]) if engines else None


def rewrite(line, verdict, note):
    """Replace verdict= and put a note in front of note= in one APPRES line (the line's other fields stay byte for byte)."""
    line = re.sub(r" verdict=\S+", " verdict=" + verdict, line, count=1)
    if " note=" not in line:
        return line + " note=" + note
    return re.sub(r" note=((?:(?! \w+=).)*)", lambda m: " note=" + note + ((" / " + m.group(1)) if m.group(1) and m.group(1) != "-" else ""), line, count=1)


def annotate_line(line, side, app_text, kf3_lines=None, kf3_base=None, smi=None, negctl=False):
    """-> (new APPRES line, record). A PASS becomes FAIL / NOTRUN when the judge says so; the verdict of any other row is left alone."""
    kv = {k: v.strip() for k, v in APPRES_KEYS.findall(line[len("APPRES "):])}
    app = kv.get("app", "")
    rec = evaluate(app, side, app_text, kf3_lines, kf3_base, smi)
    census = rec.get("census") or {}
    fields = " gpu_use=%s gpu_auth=%s gpu_subs=%s gpu_births=%s" % (rec["verdict"], rec["authority"], census.get("submissions", "-"), census.get("births", "-"))
    if negctl:
        return line, rec
    new = line
    if kv.get("verdict") == "PASS" and app in WIRED and rec["verdict"] in ("FAIL", "NOT_RUN"):
        why = clean(next((r for r in rec["reasons"] if ("] %s:" % rec["verdict"]) in r), rec["reasons"][0] if rec["reasons"] else "?"), 140)
        new = rewrite(line, "FAIL" if rec["verdict"] == "FAIL" else "NOTRUN", "gpu-use:%s:%s" % (rec["verdict"], why))
    return new + fields, rec


def control_line(app, rec):
    why = clean(next((r for r in rec["reasons"] if "] FAIL:" in r), rec["reasons"][0] if rec["reasons"] else "no reason"), 140)
    return "NEGCTL app=%s control=%s judged=%s why=%s" % (app, gu.control_verdict(rec) if rec["verdict"] != "NA" else "CONTROL_BROKEN", rec["verdict"], why)


def window_lines(path):
    text = read_text(path) if path else None
    return None if text is None else text.splitlines()


def cmd_annotate(a):
    try:
        app_text = read_text(a.app_log)
        kf3 = window_lines(a.kf3_log) if a.side == "guest" else None
        base_text = read_text(a.kf3_base) if a.kf3_base else None
        base = gu.last_status_line((base_text or "").splitlines()) if base_text else None
        new, rec = annotate_line(a.line, a.side, app_text, kf3, base, smi_from_dir(a.app, a.smi_dir, a.side), a.negctl)
        out = control_line(a.app, rec) if a.negctl else new
    except Exception as exc:  # noqa: BLE001 - fail closed: a judge that crashed must not leave a PASS of a wired row standing
        rec = None
        if a.negctl:
            out = "NEGCTL app=%s control=CONTROL_BROKEN judged=ERROR why=%s" % (a.app, clean(exc, 100))
        elif " verdict=PASS" in a.line and a.app in WIRED:
            out = rewrite(a.line, "NOTRUN", "gpu-use:NOT_RUN:judge-error " + clean(exc, 100)) + " gpu_use=NOT_RUN gpu_auth=none gpu_subs=- gpu_births=-"
        else:
            out = a.line
    print(out)
    if a.record_dir and rec is not None:
        d = Path(a.record_dir)
        d.mkdir(parents=True, exist_ok=True)
        (d / (a.app + ".gpu_use.json")).write_text(json.dumps(rec, indent=1, sort_keys=True, default=str)[:200000] + "\n")
    return 0


def cmd_annotate_res(a):
    src = Path(a.res)
    if not src.exists():
        return 0
    out = []
    for line in src.read_text(errors="replace").splitlines():
        if not line.startswith("APPRES ") or " gpu_use=" in line:
            out.append(line)
            continue
        app = dict((k, v.strip()) for k, v in APPRES_KEYS.findall(line[7:])).get("app", "")
        new, rec = annotate_line(line, a.side, read_text(os.path.join(a.logs, app + a.suffix)), negctl=a.negctl)
        out.append(control_line(app, rec) if a.negctl else new)
    tmp = src.with_suffix(src.suffix + ".tmp")
    tmp.write_text("\n".join(out) + "\n")
    os.replace(tmp, src)
    return 0


def cmd_sample(a):
    """Run the command with the box's nvidia-smi sampled around it. The probe is host-authored (the box, not the guest); no probe (no nvidia-smi) is not an
    error here: the judge reads the missing files as an unreadable sampler (NOT_RUN), never as an idle GPU."""
    probe = gu.gpu_assert().HostProbe(Path(a.out) / (a.app + ".smi"), a.interval_ms)
    started = probe.start()
    time.sleep(a.settle_s if started else 0)
    proc = subprocess.Popen(a.command)

    def stop(signum, frame):
        proc.terminate()
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    rc = proc.wait()
    time.sleep(a.settle_s if started else 0)
    if started:
        probe.stop()
    return rc


def run_apps_names():
    """The row names of run_apps.sh, read from its table (one `name|timeout|regex|command` line per row)."""
    return re.findall(r"^([A-Za-z0-9_]+)\|\d+\|", (HERE / "run_apps.sh").read_text(), re.M)


def cmd_list(_):
    for n in run_apps_names():
        cls = row_class(n)
        p = policy(n) if n in ROWS else None
        print("%-26s %-8s %s" % (n, cls, ("proof=%s engines=%s" % (p["proof"], "+".join(p["engines"]) or "-")) if p and p["proof"] else ""))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="verb", required=True)
    sub.add_parser("list")
    s = sub.add_parser("sample")
    s.add_argument("--out", required=True)
    s.add_argument("--app", required=True)
    s.add_argument("--interval-ms", type=int, default=250)
    s.add_argument("--settle-s", type=float, default=1.0)
    s.add_argument("command", nargs=argparse.REMAINDER)
    n = sub.add_parser("annotate")
    n.add_argument("--side", choices=("guest", "host"), required=True)
    n.add_argument("--app", required=True)
    n.add_argument("--line", required=True)
    n.add_argument("--app-log")
    n.add_argument("--kf3-log")
    n.add_argument("--kf3-base")
    n.add_argument("--smi-dir")
    n.add_argument("--record-dir")
    n.add_argument("--negctl", action="store_true")
    r = sub.add_parser("annotate-res")
    r.add_argument("--side", choices=("guest", "host"), required=True)
    r.add_argument("--res", required=True)
    r.add_argument("--logs", required=True)
    r.add_argument("--suffix", default=".log")
    r.add_argument("--negctl", action="store_true")
    a = ap.parse_args(argv)
    if a.verb == "sample":
        a.command = a.command[1:] if a.command and a.command[0] == "--" else a.command
        if not a.command:
            ap.error("sample needs a command after --")
        a.interval_ms = max(50, min(a.interval_ms, 2000))
    return {"list": cmd_list, "sample": cmd_sample, "annotate": cmd_annotate, "annotate-res": cmd_annotate_res}[a.verb](a)


if __name__ == "__main__":
    sys.exit(main())
