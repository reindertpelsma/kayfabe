#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Shared GPU-use assertion for guest application checks (stdlib only).

STATUS: LIVE, 2026-11-16 (GPU-free parsing and verdict logic tested in scripts/ci/test_gpu_assert.py; the guest sampler has run on
no GPU guest yet when this line was written, see the evidence of the first GPU run for its measured behaviour).

    ok, result = assert_gpu_used(guest, operation, require=("gpu", "mem"), out_dir=Path, name="demo")

`guest` is a propimage_check.Guest (or anything with .ps(script, timeout) and .scp(src, dst, to_guest)). `operation` is a callable run
while the guest samples `nvidia-smi` (the NVIDIA tool inside the guest) about twice a second; it may be any host-side function that
drives the application. The helper returns (operation_result, record). The record is text only and is also written to
`<out_dir>/<name>_gpu.json`:

    {"required": [...], "verdict": "PASS"|"FAIL", "reasons": [...], "baseline": {...}, "peak": {...}, "samples": N, ...}

Rule (never weakened to pass): for each required engine the sampled values must show use; an engine that stayed idle FAILS the row.
    gpu   utilization.gpu reaches MIN_GPU_PCT in at least MIN_SAMPLES samples
    mem   memory.used rises by at least MIN_MEM_MB over the baseline taken before the operation
    enc   encoder utilisation (dmon `enc`) reaches MIN_ENG_PCT in at least one sample, or the encoder session count exceeds the baseline
    dec   decoder utilisation (dmon `dec`) reaches MIN_ENG_PCT in at least one sample
Missing sampler output (no nvidia-smi, no GPU, empty file) is its own FAIL reason "no samples": an idle GPU and an unreadable GPU are
both failures, and the reason says which. A caller that only proves a launch must not call this helper and must label its row
LIMITED (say so in the row).
"""
import csv
import io
import json
import re
import time
from pathlib import Path

MIN_GPU_PCT = 3
MIN_ENG_PCT = 1
MIN_SAMPLES = 2
MIN_MEM_MB = 64
INTERVAL_MS = 500
ENGINES = ("gpu", "mem", "enc", "dec")
QUERY_FIELDS = ("timestamp", "utilization.gpu", "utilization.memory", "memory.used",
                "encoder.stats.sessionCount", "encoder.stats.averageFps")
SAFE_NAME = re.compile(r"[a-z0-9_]{1,40}")

# Starts both samplers detached and writes their PIDs; the files stay in the guest run directory until pulled.
START_PS = r"""
$d='{dir}'
New-Item -Force -ItemType Directory $d | Out-Null
$smi=(Get-Command nvidia-smi -ErrorAction SilentlyContinue).Source
if(-not $smi){{ $smi='C:\Windows\System32\nvidia-smi.exe' }}
Remove-Item "$d\gpu_query.csv","$d\gpu_dmon.txt","$d\gpu_pids.txt" -ErrorAction SilentlyContinue
$a=Start-Process $smi -ArgumentList '--query-gpu={fields}','--format=csv,noheader,nounits','-lms','{ms}','-f',"$d\gpu_query.csv" -PassThru -WindowStyle Hidden
$b=Start-Process cmd.exe -ArgumentList '/c',"`"$smi`" dmon -s um -d 1 > `"$d\gpu_dmon.txt`" 2>&1" -PassThru -WindowStyle Hidden
"$($a.Id) $($b.Id)" | Set-Content "$d\gpu_pids.txt"
'started'
"""

STOP_PS = r"""
$d='{dir}'
$ids=(Get-Content "$d\gpu_pids.txt" -ErrorAction SilentlyContinue) -split ' '
foreach($i in $ids){{ if($i){{ Get-CimInstance Win32_Process -Filter "ParentProcessId=$i" -ErrorAction SilentlyContinue | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}; Stop-Process -Id $i -Force -ErrorAction SilentlyContinue }} }}
Get-Process nvidia-smi -ErrorAction SilentlyContinue | Where-Object {{ $_.StartTime -gt (Get-Date).AddMinutes(-60) }} | Stop-Process -Force -ErrorAction SilentlyContinue
'stopped'
"""


def _num(text):
    m = re.match(r"\s*([0-9]+(?:\.[0-9]+)?)", str(text))
    return float(m.group(1)) if m else None


def parse_query_csv(text):
    """`nvidia-smi --query-gpu=... --format=csv,noheader,nounits` rows -> list of dicts (None for N/A or [Not Supported])."""
    rows = []
    for rec in csv.reader(io.StringIO(text)):
        if len(rec) != len(QUERY_FIELDS):
            continue
        rows.append({"t": rec[0].strip(), "gpu": _num(rec[1]), "memutil": _num(rec[2]), "mem_mb": _num(rec[3]),
                     "enc_sessions": _num(rec[4]), "enc_fps": _num(rec[5])})
    return rows


def parse_dmon(text):
    """`nvidia-smi dmon -s um` output -> list of dicts with the columns it prints (`sm`, `mem`, `enc`, `dec`, `fb`, ...).
    Header lines start with '#'; the first names the columns. A '-' value is None."""
    cols, rows = None, []
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("#"):
            names = s.lstrip("#").split()
            if names and names[0].lower() == "gpu":
                cols = [n.lower() for n in names]
            continue
        if not s or cols is None:
            continue
        parts = s.split()
        if len(parts) != len(cols):
            continue
        rows.append({c: _num(p) for c, p in zip(cols, parts)})
    return rows


def evaluate(query_rows, dmon_rows, require, min_gpu=MIN_GPU_PCT, min_eng=MIN_ENG_PCT, min_samples=MIN_SAMPLES, min_mem=MIN_MEM_MB,
             baseline_n=2):
    """Pure verdict. The baseline is the first `baseline_n` query samples (taken before the operation starts working)."""
    unknown = [e for e in require if e not in ENGINES]
    if unknown:
        raise ValueError("unknown engine %s" % unknown)
    reasons = []
    base = {}
    peak = {}
    q = [r for r in query_rows if r["gpu"] is not None or r["mem_mb"] is not None]
    if not q and not dmon_rows:
        return {"required": list(require), "verdict": "FAIL", "reasons": ["no samples: the GPU could not be read (nvidia-smi missing, no GPU or empty output)"],
                "baseline": {}, "peak": {}, "samples": 0, "dmon_samples": 0}
    head = q[:baseline_n]
    base["mem_mb"] = min((r["mem_mb"] for r in head if r["mem_mb"] is not None), default=None)
    base["gpu"] = max((r["gpu"] for r in head if r["gpu"] is not None), default=None)
    base["enc_sessions"] = max((r["enc_sessions"] for r in head if r["enc_sessions"] is not None), default=0.0)
    peak["gpu"] = max((r["gpu"] for r in q if r["gpu"] is not None), default=None)
    peak["mem_mb"] = max((r["mem_mb"] for r in q if r["mem_mb"] is not None), default=None)
    peak["enc_sessions"] = max((r["enc_sessions"] for r in q if r["enc_sessions"] is not None), default=0.0)
    peak["gpu_dmon_sm"] = max((r.get("sm") for r in dmon_rows if r.get("sm") is not None), default=None)
    peak["enc"] = max((r.get("enc") for r in dmon_rows if r.get("enc") is not None), default=None)
    peak["dec"] = max((r.get("dec") for r in dmon_rows if r.get("dec") is not None), default=None)
    busy_q = sum(1 for r in q if r["gpu"] is not None and r["gpu"] >= min_gpu)
    busy_d = sum(1 for r in dmon_rows if r.get("sm") is not None and r["sm"] >= min_gpu)
    peak["busy_samples"] = max(busy_q, busy_d)
    for e in require:
        if e == "gpu":
            if peak["busy_samples"] < min_samples:
                reasons.append("gpu engine idle: %d sample(s) at or above %d%% (need %d); peak %s" % (peak["busy_samples"], min_gpu, min_samples, peak["gpu"] if peak["gpu"] is not None else peak["gpu_dmon_sm"]))
        elif e == "mem":
            if base["mem_mb"] is None or peak["mem_mb"] is None or peak["mem_mb"] - base["mem_mb"] < min_mem:
                reasons.append("video memory did not rise by %d MiB (baseline %s, peak %s)" % (min_mem, base["mem_mb"], peak["mem_mb"]))
        elif e == "enc":
            if not ((peak["enc"] or 0) >= min_eng or peak["enc_sessions"] > base["enc_sessions"]):
                reasons.append("encoder idle: dmon enc peak %s, sessions %s -> %s" % (peak["enc"], base["enc_sessions"], peak["enc_sessions"]))
        elif e == "dec":
            if not ((peak["dec"] or 0) >= min_eng):
                reasons.append("decoder idle: dmon dec peak %s" % peak["dec"])
    return {"required": list(require), "verdict": "FAIL" if reasons else "PASS", "reasons": reasons, "baseline": base, "peak": peak,
            "samples": len(q), "dmon_samples": len(dmon_rows)}


def assert_gpu_used(guest, operation, require=("gpu",), out_dir=None, name="op", remote_dir=r"C:\kfapps\gpu", interval_ms=INTERVAL_MS,
                    settle_s=2.0, sleep=time.sleep, **thresholds):
    """Run `operation()` while the guest samples the GPU; returns (operation_result, record). The operation's exception, if any, is
    re-raised after the samplers are stopped and the record is written (verdict FAIL, reason 'operation raised')."""
    if not SAFE_NAME.fullmatch(name):
        raise ValueError("bad name")
    d = remote_dir.replace("/", "\\")
    p = guest.ps(START_PS.format(dir=d, fields=",".join(QUERY_FIELDS), ms=interval_ms))
    if p.returncode or "started" not in p.stdout:
        raise RuntimeError("GPU sampler did not start: " + (p.stderr or p.stdout)[:200])
    sleep(settle_s)                      # the baseline: samples before the operation does any work
    result, error = None, None
    try:
        result = operation()
    except Exception as exc:             # noqa: BLE001 - recorded, then re-raised below
        error = exc
    sleep(settle_s)
    guest.ps(STOP_PS.format(dir=d))
    texts = {}
    for f, key in (("gpu_query.csv", "query"), ("gpu_dmon.txt", "dmon")):
        got = guest.ps("if(Test-Path '%s\\%s'){Get-Content '%s\\%s' -Raw}" % (d, f, d, f))
        texts[key] = got.stdout if got.returncode == 0 else ""
    record = evaluate(parse_query_csv(texts["query"]), parse_dmon(texts["dmon"]), require, **thresholds)
    if error is not None:
        record["verdict"] = "FAIL"
        record["reasons"].append("operation raised: %s" % str(error)[:160])
    record["name"] = name
    if out_dir is not None:
        out = Path(out_dir)
        out.mkdir(parents=True, exist_ok=True)
        (out / (name + "_gpu.json")).write_text(json.dumps(record, indent=1))
        (out / (name + "_gpu_query.csv")).write_text(texts["query"][:200000])
        (out / (name + "_gpu_dmon.txt")).write_text(texts["dmon"][:200000])
    if error is not None:
        raise error
    return result, record
