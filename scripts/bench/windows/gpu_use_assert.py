#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Assert that an application operation really used the GPU (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-16. GPU-free tested (scripts/ci/test_resolve_gpu.py); the sampling commands have run on
one real guest only as recorded in traces/resolve_gpu_20261116 (when that directory exists). INFERRED: thresholds.

Interface (kept small so it can be merged with the equivalent helper of the image-B lane):

    samples = parse_nvidia_smi_csv(text)                     # list of Sample
    verdict = assert_gpu_used(samples, Need(gpu_peak=30.0, encoder=True), window=(t0, t1))
    verdict, samples = run_with_sampling(operation, sampler, need)   # operation() runs while `sampler` samples

`Sampler` is any object with start() and stop() -> csv text (HostSampler below runs nvidia-smi on this host; the guest
sampler is a PowerShell fragment, guest_sampler_start/stop, that a guest script embeds so the sampler lives in the
interactive session). The CSV format is exactly what these commands print:

    nvidia-smi --query-gpu=timestamp,utilization.gpu,utilization.memory,utilization.encoder,utilization.decoder,memory.used
               --format=csv,noheader,nounits -lms 500

A verdict never passes on missing data: no samples, an unparsable file, or a sampler that saw only N/A values is a
failure with a stated reason (a guest whose driver reports no utilisation cannot prove GPU use; that is a finding about
the platform, not a pass). Thresholds are arguments, never silently relaxed.
"""

from __future__ import annotations

import dataclasses
import datetime
import re
import signal
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Callable, Iterable, Optional

QUERY_FIELDS = "timestamp,utilization.gpu,utilization.memory,utilization.encoder,utilization.decoder,memory.used"
SMI_ARGS = ["--query-gpu=" + QUERY_FIELDS, "--format=csv,noheader,nounits", "-lms", "500"]

_TS = re.compile(r"^\d{4}/\d\d/\d\d \d\d:\d\d:\d\d(\.\d+)?$")


@dataclasses.dataclass(frozen=True)
class Sample:
    t: float  # seconds since the epoch, as read from the sampler's own timestamp (host or guest clock)
    gpu: Optional[float]  # utilization.gpu, percent; None when the driver printed N/A
    mem_ctrl: Optional[float]  # utilization.memory
    encoder: Optional[float]
    decoder: Optional[float]
    mem_used: Optional[float]  # MiB


@dataclasses.dataclass(frozen=True)
class Need:
    """What 'the GPU was used' means for one operation. A None threshold is not checked."""

    gpu_peak: Optional[float] = 30.0  # utilization.gpu must reach this at least once ...
    gpu_active_samples: int = 3  # ... and be >= gpu_active_floor in this many samples (>= 1.5 s at 500 ms)
    gpu_active_floor: float = 10.0
    mem_growth_mib: Optional[float] = 100.0  # memory.used peak minus the pre-operation baseline
    encoder: bool = False  # NVENC must show utilization.encoder > 0 in >= engine_samples samples
    decoder: bool = False  # NVDEC must show utilization.decoder > 0 in >= engine_samples samples
    engine_samples: int = 2
    min_samples: int = 4  # fewer samples inside the window is a failed measurement


def _num(s: str) -> Optional[float]:
    s = s.strip()
    if not s or s.upper() in ("N/A", "[N/A]", "[NOT SUPPORTED]", "NOT SUPPORTED", "[UNKNOWN ERROR]"):
        return None
    try:
        return float(s)
    except ValueError:
        return None


def _epoch(stamp: str) -> float:
    return datetime.datetime.strptime(stamp.split(".")[0], "%Y/%m/%d %H:%M:%S").replace(
        tzinfo=datetime.timezone.utc
    ).timestamp() + (float("0." + stamp.split(".")[1]) if "." in stamp else 0.0)


def parse_nvidia_smi_csv(text: str, utc_offset_s: float = 0.0) -> list[Sample]:
    """Parse the CSV printed by SMI_ARGS. Lines that are not six fields with a timestamp are skipped and counted by the
    caller through len() against the line count; the timestamp is the sampler host's local time, so a guest whose clock
    is not UTC passes utc_offset_s (local minus UTC)."""
    out = []
    for line in text.splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) != 6 or not _TS.match(parts[0]):
            continue
        out.append(Sample(_epoch(parts[0]) - utc_offset_s, *(_num(p) for p in parts[1:])))
    return out


def summarize(samples: Iterable[Sample], window: Optional[tuple[float, float]] = None) -> dict:
    """Numbers only, for the evidence file. window=(t0, t1) selects the operation; samples before t0 give the baseline."""
    allv = list(samples)
    inside = [s for s in allv if window is None or window[0] <= s.t <= window[1]]
    before = [s for s in allv if window is not None and s.t < window[0]]

    def vals(rows, attr):
        return [getattr(r, attr) for r in rows if getattr(r, attr) is not None]

    def stat(attr):
        v = vals(inside, attr)
        return {"n": len(v), "peak": max(v) if v else None, "mean": round(sum(v) / len(v), 2) if v else None,
                "nonzero": sum(1 for x in v if x > 0)}

    base_mem = vals(before, "mem_used")
    in_mem = vals(inside, "mem_used")
    baseline = (min(base_mem[-6:]) if base_mem else (in_mem[0] if in_mem else None))
    return {
        "samples_total": len(allv),
        "samples_in_window": len(inside),
        "window_s": round(window[1] - window[0], 1) if window else (round(allv[-1].t - allv[0].t, 1) if len(allv) > 1 else 0),
        "gpu": stat("gpu"),
        "mem_ctrl": stat("mem_ctrl"),
        "encoder": stat("encoder"),
        "decoder": stat("decoder"),
        "mem_used": {"baseline": baseline, "peak": max(in_mem) if in_mem else None,
                     "growth": (max(in_mem) - baseline) if in_mem and baseline is not None else None},
    }


def assert_gpu_used(samples: Iterable[Sample], need: Need, window: Optional[tuple[float, float]] = None) -> dict:
    """Return {'ok': bool, 'reasons': [...], 'summary': {...}}. Every unmet requirement is listed; none is relaxed."""
    allv = list(samples)
    sm = summarize(allv, window)
    reasons = []
    n = sm["samples_in_window"]
    if n < need.min_samples:
        reasons.append(f"only {n} samples inside the operation window (need {need.min_samples}); the sampler saw too little to judge")
    g = sm["gpu"]
    if need.gpu_peak is not None:
        if g["n"] == 0:
            reasons.append("utilization.gpu was N/A in every sample (the driver reports no utilisation: cannot show GPU use)")
        else:
            if g["peak"] < need.gpu_peak:
                reasons.append(f"utilization.gpu peaked at {g['peak']:g}% (need >= {need.gpu_peak:g}%)")
            active = sum(1 for s in allv if (window is None or window[0] <= s.t <= window[1]) and s.gpu is not None and s.gpu >= need.gpu_active_floor)
            if active < need.gpu_active_samples:
                reasons.append(f"only {active} samples with utilization.gpu >= {need.gpu_active_floor:g}% (need {need.gpu_active_samples})")
    m = sm["mem_used"]
    if need.mem_growth_mib is not None:
        if m["growth"] is None:
            reasons.append("memory.used unreadable")
        elif m["growth"] < need.mem_growth_mib:
            reasons.append(f"memory.used grew {m['growth']:g} MiB over its baseline (need >= {need.mem_growth_mib:g})")
    for flag, key in ((need.encoder, "encoder"), (need.decoder, "decoder")):
        if flag:
            st = sm[key]
            if st["n"] == 0:
                reasons.append(f"utilization.{key} was N/A in every sample")
            elif st["nonzero"] < need.engine_samples:
                reasons.append(f"utilization.{key} was nonzero in {st['nonzero']} samples (need {need.engine_samples}); peak {st['peak']:g}%")
    return {"ok": not reasons, "reasons": reasons, "summary": sm}


class HostSampler:
    """nvidia-smi on this host, started and stopped as our own child process (never a name-based kill)."""

    def __init__(self, extra_args: Optional[list] = None, smi: str = "nvidia-smi"):
        self.cmd = [smi, *SMI_ARGS, *(extra_args or [])]
        self.proc = None
        self.tmp = None

    def start(self):
        self.tmp = tempfile.NamedTemporaryFile("w+", prefix="kf-smi-", suffix=".csv", delete=False)
        self.proc = subprocess.Popen(self.cmd, stdout=self.tmp, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, text=True)

    def stop(self) -> str:
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self.tmp.close()
        text = Path(self.tmp.name).read_text(errors="replace")
        Path(self.tmp.name).unlink(missing_ok=True)
        return text


def run_with_sampling(operation: Callable[[], object], sampler, need: Need, settle_s: float = 3.0,
                      clock: Callable[[], float] = time.time, sleep: Callable[[float], None] = time.sleep,
                      utc_offset_s: float = 0.0):
    """Start the sampler, wait settle_s for a baseline, run operation(), stop, and judge the samples inside the operation's
    own start/end (read from `clock`, the same clock the sampler's timestamps use only when sampler and clock share a host:
    for a guest sampler pass a guest-clock function or use the window the guest script itself recorded)."""
    sampler.start()
    sleep(settle_s)
    t0 = clock()
    err = None
    result = None
    try:
        result = operation()
    except BaseException as exc:  # the sampler is always stopped; the exception is re-raised after judging
        err = exc
    t1 = clock()
    sleep(1.0)
    samples = parse_nvidia_smi_csv(sampler.stop(), utc_offset_s)
    verdict = assert_gpu_used(samples, need, (t0, t1))
    verdict["operation_result"] = result if isinstance(result, (str, int, float, bool, type(None))) else repr(type(result))
    if err is not None:
        verdict["ok"] = False
        verdict["reasons"].append(f"operation raised {type(err).__name__}: {err}")
    return verdict, samples


# --- guest side: PowerShell fragments embedded in a script that runs in the interactive session ---------------------

def guest_sampler_start(csv_path: str) -> str:
    r"""PowerShell that starts nvidia-smi into csv_path and keeps the process in $smi. The path must be a guest path
    like C:\kfapps\<run>\resolve\smi.csv (single-quote safe)."""
    if "'" in csv_path or "\n" in csv_path:
        raise ValueError("unsafe guest path")
    args = " ".join(SMI_ARGS).replace("'", "''")
    return (
        f"$smiCsv='{csv_path}'\n"
        f"$smi=Start-Process -FilePath nvidia-smi -ArgumentList '{args}' -RedirectStandardOutput $smiCsv "
        "-WindowStyle Hidden -PassThru\n"
    )


def guest_sampler_stop() -> str:
    return "try {Start-Sleep 1; if($smi -and -not $smi.HasExited){Stop-Process -Id $smi.Id -Force}} catch {}\n"


def guest_clock_offset_ps() -> str:
    """PowerShell expression of the guest's local-minus-UTC offset in seconds (nvidia-smi stamps are local time)."""
    return "[int]([TimeZoneInfo]::Local.GetUtcOffset((Get-Date)).TotalSeconds)"
