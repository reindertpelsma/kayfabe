#!/usr/bin/env python3
"""matrix_table.py <results-dir> [--out table.tsv] [--gate-list gate_list.txt] — the per-app table of one
app-matrix run, with the cause class decided MECHANICALLY (no judgement) from the host baseline.

Inputs, in <results-dir> (written by scripts/apps/apps_matrix.sh): host.res, guest.res,
guest_isolated.res (optional), and the per-app logs <app>.host.log / <app>.guest.log /
<app>.guest_dmesg.log (optional; only used for the key output line and the Xid counts).
The classes are the ones pre-registered in traces/apps_linux_20261107/PREREG.md:

  GATE             host PASS and guest PASS (batched, or alone in a fresh boot)        -> required in the gate
  ORDER_SENSITIVE  host PASS, guest PASS alone, FAIL batched (a boot that degrades)     -> kayfabe finding;
                                                                                          runs in its own boot
  KAYFABE_DEFECT   host PASS, guest FAIL alone in a fresh boot                          -> never hidden
  HOST_LIMIT       host FAIL / TIMEOUT / NOTRUN on the same box (bare metal first)      -> documented exception
  KNOWN_UNSUPPORTED UVM: explicitly unsupported until the per-fault GR hold is measured and bounded
  POLICY_SKIP      not run by policy (docs/DOWNLOADS.md terms status)                   -> not a result
  UNATTRIBUTED     host PASS but the guest has no usable row (boot failure, guest dead,
                   no isolated re-run)                                                  -> re-run, never counted

Last row per (side, app) wins in each file. The guest verdict of record is the isolated re-run when there is
one. `--gate-list` writes the GATE apps (one per line, run order of `run_apps.sh list`).
"""

import argparse
import collections
import json
from pathlib import Path
import os
import re
import sys

POLICY_SKIP = {
    "geekbench_gpu": "Geekbench: docs/DOWNLOADS.md says not allowed for scripted use (Pro licence); the free mode uploads the result",
}
KNOWN_UNSUPPORTED = json.loads((Path(__file__).resolve().parent / "known_unsupported.json").read_text())["known_unsupported"]
PASS = "PASS"
KV = re.compile(r"(\w+)=((?:(?! \w+=).)*)")

# One regular expression per app for the line that carries its figure of merit, when it prints one.
# First match in the log wins; the generic fallback is tried last. Nothing is invented: no match = "-".
KEY_PATTERNS = [
    ("bandwidthTest", r"^\s*32000000\s+[0-9.]+"),
    ("cudaTensorCoreGemm", r"TFLOPS"),
    ("bf16TensorCoreGemm", r"TFLOPS"),
    ("matrixMul", r"Performance="),
    ("matrixMulDrv", r"Performance="),
    ("sgemm_cublas", r"GFLOPS|gflops"),
    ("stream_triad", r"GB/s"),
    ("nbody", r"GFLOP|interactions"),
    ("llama_bench", r"tg64"),
    ("vkpeak", r"fp32-scalar"),
    ("clpeak", r"Global memory bandwidth"),
    ("transpose", r"GB/s|Throughput"),
    ("UnifiedMemoryPerf", r"^16384"),
    ("gpu_burn", r"GPU 0: OK"),
    ("torch_ai_bench", r"CHECK bert_infer_seqs ok"),
    ("hf_generate", r"OUTSHA"),
    ("llama_cpp_gen", r"OUTSHA"),
    ("blender_cycles", r"BLENDER_OK"),
    ("hashcat", r"Speed\.#1|Recovered"),
    ("nvenc_h264", r"frame= *600"),
    ("nvenc_hevc", r"frame= *600"),
    ("nvdec_h264", r"frame="),
]
LAST_MATCH = {"nvenc_h264", "nvenc_hevc", "nvdec_h264"}  # ffmpeg prints progress lines: the last one is the result
GENERIC_KEY = re.compile(r"^METRIC |GFLOPS|TFLOPS|GB/s|MB/s|tokens per second| t/s|fps|ms\b", re.I)


def parse(path):
    """APPRES rows of one .res file: {app: {key: value}}; the last row of an app wins."""
    rows = collections.OrderedDict()
    if not os.path.exists(path):
        return rows
    with open(path, errors="replace") as handle:
        for line in handle:
            if not line.startswith("APPRES "):
                continue
            kv = {k: v.strip() for k, v in KV.findall(line[7:].strip())}
            if "app" in kv:
                rows[kv["app"]] = kv
    return rows


def key_line(directory, app, side):
    path = os.path.join(directory, "%s.%s.log" % (app, side))
    if not os.path.exists(path):
        return "-"
    with open(path, errors="replace") as handle:
        lines = [x.rstrip("\n") for x in handle if not x.startswith("=== ")]
    patterns = [re.compile(rx) for name, rx in KEY_PATTERNS if name == app]
    for rx in patterns + [GENERIC_KEY]:
        hits = [line for line in lines if rx.search(line)]
        if hits:
            line = hits[-1] if app in LAST_MATCH else hits[0]
            return re.sub(r"\s+", " ", line).strip()[:160].replace("\t", " ")
    return "-"


def classify(app, host, guest, isolated):
    """(class, guest verdict of record, reason). Mechanical: the order of the checks IS the rule."""
    if app in KNOWN_UNSUPPORTED:
        return "KNOWN_UNSUPPORTED", "-", KNOWN_UNSUPPORTED[app]
    if app in POLICY_SKIP:
        return "POLICY_SKIP", "-", POLICY_SKIP[app]
    hv = host.get("verdict") if host else None
    if hv is None:
        return "UNATTRIBUTED", "-", "no host row: no bare-metal baseline for this app"
    if hv != PASS:
        return "HOST_LIMIT", (isolated or guest or {}).get("verdict", "-"), "host %s on the same box: not kayfabe's" % hv
    batched = guest.get("verdict") if guest else None
    alone = isolated.get("verdict") if isolated else None
    if batched == PASS:
        return "GATE", PASS, ""
    if alone == PASS:
        return "ORDER_SENSITIVE", PASS, "guest batched %s, alone PASS" % batched
    if alone is not None and alone not in ("BOOT_FAIL", "GUEST_DEAD"):
        return "KAYFABE_DEFECT", alone, "host PASS, guest %s alone in a fresh boot" % alone
    if batched is None and alone is None:
        return "UNATTRIBUTED", "-", "no guest row"
    return "UNATTRIBUTED", alone or batched, "guest %s and no clean isolated re-run" % (alone or batched)


def table(directory):
    host = parse(os.path.join(directory, "host.res"))
    guest = parse(os.path.join(directory, "guest.res"))
    isolated = parse(os.path.join(directory, "guest_isolated.res"))
    order = []
    for source in (host, guest, isolated):
        for app in source:
            if app not in order:
                order.append(app)
    for app in list(POLICY_SKIP) + list(KNOWN_UNSUPPORTED):
        if app not in order:
            order.append(app)
    rows = []
    for app in order:
        cls, final, reason = classify(app, host.get(app), guest.get(app), isolated.get(app))
        g = isolated.get(app) or guest.get(app) or {}
        rows.append(
            {
                "app": app,
                "host": (host.get(app) or {}).get("verdict", "-"),
                "host_secs": (host.get(app) or {}).get("secs", "-"),
                "guest_batched": (guest.get(app) or {}).get("verdict", "-"),
                "guest_alone": (isolated.get(app) or {}).get("verdict", "-"),
                "guest": final,
                "guest_secs": g.get("secs", "-"),
                "class": cls,
                "guest_xid": g.get("guest_xid", "-"),
                "kf3_refusals": g.get("kf3_refusals", "-"),
                "key_host": key_line(directory, app, "host"),
                "key_guest": key_line(directory, app, "guest"),
                "reason": reason or (g.get("note", "") if cls != "GATE" else ""),
            }
        )
    return rows


COLUMNS = [
    "app", "class", "host", "guest_batched", "guest_alone", "host_secs", "guest_secs",
    "guest_xid", "kf3_refusals", "key_host", "key_guest", "reason",
]


def tsv(rows):
    out = ["\t".join(COLUMNS)]
    for row in rows:
        out.append("\t".join(str(row[c]).replace("\t", " ").replace("\n", " ") for c in COLUMNS))
    return "\n".join(out) + "\n"


def totals(rows):
    count = collections.Counter(r["class"] for r in rows)
    return dict(count)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("directory")
    parser.add_argument("--out")
    parser.add_argument("--gate-list")
    args = parser.parse_args(argv)
    rows = table(args.directory)
    text = tsv(rows)
    if args.out:
        with open(args.out, "w") as handle:
            handle.write(text)
    else:
        sys.stdout.write(text)
    if args.gate_list:
        with open(args.gate_list, "w") as handle:
            handle.write("# known_unsupported (non-gating; listed in reports):\n")
            for app, reason in KNOWN_UNSUPPORTED.items():
                handle.write("# " + app + ": " + reason + "\n")
            handle.write("".join(r["app"] + "\n" for r in rows if r["class"] == "GATE"))
    print("MATRIX_TABLE rows=%d %s" % (len(rows), " ".join("%s=%d" % kv for kv in sorted(totals(rows).items()))), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
