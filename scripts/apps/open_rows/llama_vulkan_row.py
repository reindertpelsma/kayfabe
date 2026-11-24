#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""llama.cpp Vulkan row: llama-bench on the Vulkan backend (cooperative-matrix compute, pipeline creation at load), run on this machine (stdlib only).

STATUS: LIVE, 2026-11-23 (design: docs/design/V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md; the decisions are pure functions here and are GPU-free tested by
scripts/ci/test_open_lanes.py). The native run on a Vast box is in traces/open_lanes_20261123/; the row has not run in a kayfabe guest.

    llama_vulkan_row.py run --out DIR [--bin PATH] [--model PATH] [--bundle DIR] [--data DIR] [--ngl N] [--emit]

`run` starts the llama-bench that provision.sh built from the pinned llama.cpp commit with -DGGML_VULKAN=ON (the same commit as the CUDA rows), on the
pinned GGUF the CUDA rows use, at `-ngl 99 -p 512 -n 64 -r 2`, and judges its own output:

  * ggml_vulkan found a device and the device is an NVIDIA GPU, not llvmpipe, lavapipe or SwiftShader (a Vulkan loader that lists a CPU driver first
    would run the whole benchmark on the CPU and still print a table);
  * every table row ran on the Vulkan backend with ngl >= 1, and both the prompt (pp512) and the generation (tg64) rows have a positive t/s.

The die's cooperative-matrix support is recorded, never expected: the `matrix cores:` field of the device line (`KHR_coopmat`, `NV_coopmat2`, `none`) is
what the driver told ggml, and a die without the extension still passes with `coopmat=none` (derive, never capture).

GPU use: the application proof above is a guest self-report; in a native run the row also samples the machine's nvidia-smi (census.py) and records
whether memory.used rose by the model's weights. scripts/bench/gpu_use.py judges the same log (`llama_vulkan`), and the negative control is the same row
with the NVIDIA ICD hidden from the Vulkan loader and `-ngl 0`: no Vulkan device, so the row FAILs.
Last line: `LLAMA_VULKAN_SUMMARY status=PASS|FAIL|NOT_RUN ...`; exit 0 on PASS, and on NOT_RUN after an `APPNOTRUN <reason>` line (run_apps.sh makes that a NOTRUN row).
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import census  # noqa: E402
import rowio  # noqa: E402

MODEL_FILE = "qwen2.5-1.5b-instruct-q4_k_m.gguf"
BENCH_ARGS = ("-p", "512", "-n", "64", "-r", "2")
RUN_TIMEOUT_S = 900
SOFT_RX = re.compile(r"llvmpipe|lavapipe|swiftshader|software|softpipe", re.I)
NVIDIA_RX = re.compile(r"nvidia|geforce|rtx|quadro|tesla", re.I)
FOUND_RX = re.compile(r"ggml_vulkan: Found (\d+) Vulkan devices?")
DEVICE_RX = re.compile(r"^ggml_vulkan: (\d+) = (.+?) \((.*?)\) \| (.*)$", re.M)
COOPMAT_RX = re.compile(r"matrix cores: *([A-Za-z0-9_]+)")
TABLE_RX = re.compile(r"^\|[^|]*\|[^|]*\|[^|]*\|\s*([^|\s][^|]*?)\s*\|\s*(\d+)\s*\|\s*((?:pp|tg)\d+)\s*\|\s*([0-9.]+)", re.M)


def parse_devices(text):
    """-> list of (index, name, driver, fields) of the `ggml_vulkan: N = ...` lines (the device list ggml printed while loading)."""
    return [(int(i), name.strip(), drv.strip(), rest) for i, name, drv, rest in DEVICE_RX.findall(text or "")]


def parse_table(text):
    """-> list of (backend, ngl, test, t/s) of the llama-bench markdown table."""
    return [(b, int(n), t, float(v)) for b, n, t, v in TABLE_RX.findall(text or "")]


def coopmat(devices):
    """What the first device told ggml about matrix cores: KHR_coopmat, NV_coopmat2, none, or unknown (no such field)."""
    if not devices:
        return "unknown"
    m = COOPMAT_RX.search(devices[0][3])
    return m.group(1) if m else "unknown"


def judge(text):
    """-> (status, reason, facts). status PASS | FAIL; facts: device, coopmat, pp512, tg64. The reasons are the first failed requirement."""
    devices, rows = parse_devices(text), parse_table(text)
    facts = {"device": devices[0][1] if devices else "", "coopmat": coopmat(devices), "pp512": 0.0, "tg64": 0.0}
    if not devices:
        return "FAIL", "no_vulkan_device", facts
    gpu = [d for d in devices if NVIDIA_RX.search(d[1]) and not SOFT_RX.search(d[1] + " " + d[2])]
    if not gpu:
        return "FAIL", "device_is_not_nvidia:" + devices[0][1][:40].replace(" ", "_"), facts
    facts["device"] = gpu[0][1]
    if not rows:
        return "FAIL", "no_table_row", facts
    for backend, ngl, test, tps in rows:
        if "vulkan" not in backend.lower() or "cpu" in backend.lower():
            return "FAIL", "backend_%s" % re.sub(r"[^A-Za-z0-9]+", "_", backend)[:20], facts
        if ngl < 1:
            return "FAIL", "ngl_%d" % ngl, facts
        if tps <= 0:
            return "FAIL", "no_throughput_" + test, facts
        if test in ("pp512", "tg64"):
            facts[test] = tps
    if not (facts["pp512"] > 0 and facts["tg64"] > 0):
        return "FAIL", "missing_pp512_or_tg64", facts
    return "PASS", "ok", facts


def run_cmd(argv, timeout, env=None):
    """-> (rc, output, seconds, timed_out). Own process group, killed whole on timeout; never a shell."""
    t0 = time.time()
    try:
        p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", start_new_session=True,
                             stdin=subprocess.DEVNULL, env=env)
    except OSError as e:
        return None, "cannot execute %s: %s" % (argv[0], e), 0.0, False
    try:
        out, _ = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return 124, out or "", time.time() - t0, True
    return p.returncode, out or "", time.time() - t0, False


def bench_argv(binary, model, ngl):
    return [binary, "-m", model, "-ngl", str(int(ngl))] + list(BENCH_ARGS)


def find_binary(explicit, bundle):
    if explicit:
        return explicit if os.access(explicit, os.X_OK) else None
    for c in (Path(bundle) / "llama_vk" / "llama-bench" if bundle else None,):
        if c and os.access(c, os.X_OK):
            return str(c)
    return shutil.which("llama-bench-vulkan")


def find_model(explicit, data):
    for c in (explicit, str(Path(data) / MODEL_FILE) if data else None):
        if c and os.path.isfile(c):
            return c
    return None


def cmd_run(a):
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    binary, model = find_binary(a.bin, a.bundle), find_model(a.model, a.data)
    stream = census.SmiStream().start()
    note = "binary_missing" if not binary else ("model_missing" if not model else "")
    text, secs, rc, timed_out = "", 0.0, None, False
    if not note:
        # the bench libraries sit beside the binary (the static ggml shared objects of the Vulkan build)
        env = dict(os.environ)
        env["LD_LIBRARY_PATH"] = os.pathsep.join([str(Path(binary).parent)] + ([env["LD_LIBRARY_PATH"]] if env.get("LD_LIBRARY_PATH") else []))
        rc, text, secs, timed_out = run_cmd(bench_argv(binary, model, a.ngl), RUN_TIMEOUT_S, env)
        print(text, end="" if text.endswith("\n") else "\n")
    samples = stream.stop()
    if note:
        status, reason, facts = "NOT_RUN", note, {"device": "", "coopmat": "unknown", "pp512": 0.0, "tg64": 0.0}
    elif timed_out:
        status, reason, facts = "FAIL", "timeout", judge(text)[2]
    elif rc != 0:
        status, reason, facts = "FAIL", "rc_%s" % rc, judge(text)[2]
    else:
        status, reason, facts = judge(text)
    use, use_why = census.gpu_use_verdict(census.peaks(samples), "mem") if status == "PASS" else ("NA", "the row did not pass, so there was no work to prove")
    summary = ("LLAMA_VULKAN_SUMMARY status=%s reason=%s device=%s coopmat=%s pp512_tps=%.2f tg64_tps=%.2f ngl=%d secs=%.0f gpu_use=%s" % (
        status, reason, (facts["device"] or "none").replace(" ", "_"), facts["coopmat"], facts["pp512"], facts["tg64"], a.ngl, secs, use))
    (out / "bench.log").write_text(census.scrub(text[-20000:]))
    (out / "hostsmi.csv").write_text(census.samples_to_csv(samples))
    (out / "summary.txt").write_text(census.scrub("# gpu_use_why=%s\n%s\n" % (use_why, summary)))
    if a.emit:
        sys.stdout.write(rowio.emit("summary.txt", (out / "summary.txt").read_text()))
    print(summary)
    if status == "NOT_RUN":                   # run_apps.sh turns this line (exit 0) into a NOTRUN row: our own configuration, never a result about the GPU
        print("APPNOTRUN llama_vulkan:%s" % reason)
        return 0
    return 0 if status == "PASS" else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--bin")
    r.add_argument("--model")
    r.add_argument("--bundle")
    r.add_argument("--data")
    r.add_argument("--ngl", type=int, default=99)
    r.add_argument("--emit", action="store_true")
    a = ap.parse_args(argv)
    return cmd_run(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
