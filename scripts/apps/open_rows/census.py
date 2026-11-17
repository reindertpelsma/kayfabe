#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host census for the open rows: did the GPU do the work, and how many guest RM calls did kf3 leave unanswered (stdlib only).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md section 6; GPU-free tests: scripts/ci/test_open_rows.py).
The sampling command has run natively on Vast boxes (traces/open_rows_ffdcgm_20261117/); the kf3 half has not run on a guest.

Two things, both read on the HOST of a guest run (or on the machine itself in a native run):

  1. GPU use. `nvidia-smi --query-gpu=... -lms 250` is streamed while a row runs; the peaks of the session counter, the encoder, decoder
     and graphics utilisation and the memory rise are what proves the work reached the GPU. A guest's own nvidia-smi cannot prove it
     until the utilisation flag serves the counters (docs/design/V3_UTILIZATION_FLAG.md), so the proof is read on the host. Missing
     data is never a pass: no samples, or only N/A values, is UNPROVEN with the reason.
  2. The unserviced-RPC census: the number of `GSP rpc UNSERVICED` lines in the kf3 QEMU log up to a line number (BEFORE the row) and to
     the end (AFTER it), counted by scripts/bench/windows/kf3_census.py, the census the other rows read.

    census.py sample --out FILE            stream nvidia-smi into FILE until SIGTERM, then print one JSON line of peaks
    census.py verdict --csv FILE --need enc|dec|gpu|mem|session|info
    census.py kf3 --log QLOG --before N    print `unserviced_before=.. unserviced_after=.. unserviced_in_row=..` (N = log lines before the row)
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading
import time

SMI_FIELDS = ("encoder.stats.sessionCount", "utilization.encoder", "utilization.decoder", "utilization.gpu", "memory.used")
SMI_ARGV = ["nvidia-smi", "--query-gpu=" + ",".join(SMI_FIELDS), "--format=csv,noheader,nounits", "-lms", "250"]
NEED_FIELD = {"enc": "enc", "dec": "dec", "gpu": "gpu", "mem": "mem_rise", "session": "sess", "info": "info"}
# thresholds (INFERRED, not measured on a guest): an engine counts as used when the host saw this much, in this many samples
ENGINE_FLOOR = 1.0
ENGINE_SAMPLES = 2

UUID = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b|GPU-[0-9a-fA-F-]{8,}|MIG-[0-9a-fA-F-]{8,}")
IPV4 = re.compile(r"(?<!\w)(?<!\d\.)(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?!\w)(?!\.\d)")
PCI = re.compile(r"\b[0-9a-fA-F]{4,8}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]\b")
MAC = re.compile(r"\b(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b")


def scrub(text, ips=True):
    """Evidence leaves a box only after this: GPU UUIDs, MACs, PCI bus addresses and IPv4 addresses are replaced. `ips=False` is for
    structured values (driver, NVML and VBIOS versions look like addresses: 13.580.126.20) that come from a query, not from a log."""
    text = UUID.sub("<uuid>", text)
    text = MAC.sub("<mac>", text)
    text = PCI.sub("<pci>", text)
    return IPV4.sub("<ip>", text) if ips else text


def _num(cell):
    cell = cell.strip()
    return float(cell) if re.fullmatch(r"\d+(\.\d+)?", cell) else None  # [N/A], N/A, Not Supported, empty


def parse_smi_line(line):
    """One `--format=csv,noheader,nounits` line of SMI_FIELDS -> dict(sess, enc, dec, gpu, mem) with None for non-numbers, or None."""
    cells = line.strip().split(",")
    if len(cells) != len(SMI_FIELDS):
        return None
    vals = [_num(c) for c in cells]
    return dict(zip(("sess", "enc", "dec", "gpu", "mem"), vals))


def peaks(samples):
    """samples: list of (t, dict). -> {n, sess, enc, dec, gpu (max or None), enc_n/dec_n/gpu_n (samples at or above the floor),
    mem_rise (max - first, MiB, or None)}. A field that is None in every sample stays None: it is not zero."""
    out = {"n": len(samples)}
    for k in ("sess", "enc", "dec", "gpu"):
        vals = [s[k] for _, s in samples if s[k] is not None]
        out[k] = max(vals) if vals else None
        out[k + "_n"] = sum(1 for v in vals if v >= (1 if k == "sess" else ENGINE_FLOOR))
    mem = [s["mem"] for _, s in samples if s["mem"] is not None]
    out["mem_rise"] = (max(mem) - mem[0]) if mem else None
    return out


def gpu_use_verdict(p, need):
    """-> (PROVEN | UNPROVEN | FAILED, reason). PROVEN: the needed counter was seen. UNPROVEN: the counters were N/A or there were no
    samples (a statement about the platform, never a pass). FAILED: counters were readable and stayed idle through the whole window."""
    if p["n"] == 0:
        return "UNPROVEN", "no samples (nvidia-smi absent or silent)"
    if need == "info":  # a row that may legitimately do no GPU work (the DCGM diagnostics skip on a GeForce): peaks are recorded, nothing is asserted
        return "INFO", " ".join("%s=%s" % (k, "-" if p[k] is None else "%g" % p[k]) for k in ("gpu", "enc", "dec", "sess", "mem_rise"))
    keys = ("sess", "enc") if need == "enc" else {"dec": ("dec",), "gpu": ("gpu",), "mem": ("mem_rise",), "session": ("sess",)}[need]
    readable = [k for k in keys if p[k] is not None]
    if not readable:
        return "UNPROVEN", "%s was N/A in every sample" % "/".join(keys)
    for k in readable:
        if k == "mem_rise":
            if p[k] >= 16:
                return "PROVEN", "memory.used rose %.0f MiB" % p[k]
        elif k == "sess":
            if p["sess_n"] >= 1:
                return "PROVEN", "encoder session counter reached %d" % p["sess"]
        elif p[k + "_n"] >= ENGINE_SAMPLES:
            return "PROVEN", "utilization.%s peaked at %.0f in %d samples" % ({"enc": "encoder", "dec": "decoder", "gpu": "gpu"}[k], p[k], p[k + "_n"])
    return "FAILED", "%s readable and idle (peak %s)" % ("/".join(readable), ", ".join("%s=%s" % (k, p[k]) for k in readable))


class SmiStream:
    """Runs `nvidia-smi -lms 250` in the background and keeps (time, values) samples; window(t0, t1) selects a case."""

    def __init__(self, argv=None):
        self.argv = argv or SMI_ARGV
        self.samples = []
        self.proc = None
        self.thread = None
        self.error = ""

    def start(self):
        try:
            self.proc = subprocess.Popen(self.argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        except OSError as e:
            self.error = "cannot start nvidia-smi: %s" % e
            return self
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()
        return self

    def _read(self):
        for line in self.proc.stdout:
            s = parse_smi_line(line)
            if s is not None:
                self.samples.append((time.time(), s))

    def window(self, t0, t1):
        return [(t, s) for t, s in list(self.samples) if t0 <= t <= t1]

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        if self.thread:
            self.thread.join(5)
        return self.samples


def samples_to_csv(samples):
    return "".join("%.3f,%s\n" % (t, ",".join("" if s[k] is None else "%g" % s[k] for k in ("sess", "enc", "dec", "gpu", "mem"))) for t, s in samples)


def csv_to_samples(text):
    out = []
    for line in text.splitlines():
        cells = line.split(",")
        if len(cells) != 6:
            continue
        try:
            t = float(cells[0])
        except ValueError:
            continue
        vals = [_num(c) for c in cells[1:]]
        out.append((t, dict(zip(("sess", "enc", "dec", "gpu", "mem"), vals))))
    return out


# ------------------------------------------------------------------------------------- the unserviced-RPC census
def _census_module():
    path = Path(__file__).resolve().parents[2] / "bench/windows/kf3_census.py"
    spec = importlib.util.spec_from_file_location("kf3_census", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def unserviced_counts(lines, before):
    """lines: the whole kf3 QEMU log; before: how many lines existed when the row started. -> dict(before, after, in_row, ids), counted by
    kf3_census.census, so the number is the one every other row reports."""
    mod = _census_module()
    lines = list(lines)
    pre = mod.census(lines[:before])
    allc = mod.census(lines)
    row = mod.census(lines[before:])
    return {"before": pre["counts"].get("unserviced", 0), "after": allc["counts"].get("unserviced", 0),
            "in_row": row["counts"].get("unserviced", 0), "ids": row["ids"].get("unserviced", [])}


def format_unserviced(c):
    return "unserviced_before=%d unserviced_after=%d unserviced_in_row=%d unserviced_ids_in_row=%s" % (
        c["before"], c["after"], c["in_row"], ",".join(c["ids"]).replace(" ", "") or "-")


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("sample")
    s.add_argument("--out", required=True)
    v = sub.add_parser("verdict")
    v.add_argument("--csv", required=True)
    v.add_argument("--need", required=True, choices=sorted(NEED_FIELD))
    k = sub.add_parser("kf3")
    k.add_argument("--log", required=True)
    k.add_argument("--before", type=int, required=True)
    a = ap.parse_args(argv)
    if a.cmd == "sample":
        stream = SmiStream().start()
        stop = threading.Event()
        signal.signal(signal.SIGTERM, lambda *_: stop.set())
        signal.signal(signal.SIGINT, lambda *_: stop.set())
        stop.wait()
        samples = stream.stop()
        Path(a.out).write_text(samples_to_csv(samples))
        print(json.dumps(dict(peaks(samples), error=stream.error)))
        return 0
    if a.cmd == "verdict":
        samples = csv_to_samples(Path(a.csv).read_text())
        res, why = gpu_use_verdict(peaks(samples), a.need)
        print("host_gpu_use=%s need=%s why=%s" % (res, a.need, why.replace(" ", "_")))
        return 0 if res in ("PROVEN", "INFO") else 1
    with open(a.log, "rb") as f:
        lines = [l.decode("utf-8", "replace").rstrip("\n") for l in f]
    print(format_unserviced(unserviced_counts(lines, a.before)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
