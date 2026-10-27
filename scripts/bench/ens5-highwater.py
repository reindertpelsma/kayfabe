#!/usr/bin/env python3
"""Stream owned sparse counters; never infer per-space usage from session totals.

Manifest entries name a workload and inclusive first/last source-line numbers.
Old made-freed excludes initial roots: report that delta, not an invented exact count.
"""
import argparse
import json
import re
from pathlib import Path

FIELDS = re.compile(r"([a-z_]+)=(0x[0-9a-fA-F]+|[0-9]+)")


def scan(lines, manifest):
    out, last = {}, {}
    for index, line in enumerate(lines, 1):
        if "SPARSE space=" not in line:
            continue
        if len(line) > 16384:
            raise ValueError("counter line exceeds16KiB")
        f = {k: int(v, 0) for k, v in FIELDS.findall(line)}
        if not {"space", "made", "freed"} <= f.keys():
            continue
        workload = [m["workload"] for m in manifest
                    if m["first_line"] <= index <= m["last_line"]]
        if len(workload) != 1:
            continue
        space = f["space"]
        old = last.get(space)
        epoch = 0 if old is None else old[2] + int(f["made"] < old[0] or f["freed"] < old[1])
        last[space] = (f["made"], f["freed"], epoch)
        key = (workload[0], space, epoch)
        row = out.setdefault(key, {"workload": workload[0], "space": hex(space), "epoch": epoch,
                                   "samples": 0, "peak_made_minus_freed": None,
                                   "exact_peak_blocks": None, "coverage": "censored"})
        row["samples"] += 1
        net = f["made"] - f["freed"]
        prior = row["peak_made_minus_freed"]
        row["peak_made_minus_freed"] = net if prior is None else max(prior, net)
        if "peak_blocks" in f:
            row["exact_peak_blocks"] = max(row["exact_peak_blocks"] or 0, f["peak_blocks"])
    return list(out.values())


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--log", type=Path, required=True)
    p.add_argument("--manifest", type=Path, required=True)
    a = p.parse_args()
    if a.manifest.stat().st_size > 65536:
        p.error("manifest exceeds64KiB")
    manifest = json.loads(a.manifest.read_text())
    if len(manifest) > 256:
        p.error("manifest exceeds256 intervals")
    for m in manifest:
        if (not isinstance(m["workload"], str) or not 1 <= len(m["workload"]) <= 80
                or not 1 <= m["first_line"] <= m["last_line"]):
            p.error("invalid interval")
    # The capture manifest supplies the expected viewset names for its actual release.
    workloads = list(dict.fromkeys(m["workload"] for m in manifest))
    with a.log.open(errors="replace") as f:
        rows = scan(f, manifest)
    print(json.dumps({"spaces": rows, "unavailable_workloads": [w for w in workloads
          if not any(r["workload"] == w for r in rows)],
          "limits": "line samples may be print-capped; initial roots absent from old deltas"},
          indent=2))


if __name__ == "__main__":
    main()
