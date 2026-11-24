#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""storm_deadlines.py RUN_DIR -- per cycle, from the first 30000 trace lines after the wake (c*_wake_trace_head.txt): the
guest-written one-shot stimer counts (100 ns, absolute) against KVM's reference time, and the expiry count (STATUS: RESEARCH,
2026-11-24; evidence tool of traces/vfio_s3_control_20261124/). Data only."""
import re
import sys
from pathlib import Path


def one(path):
    after, counts, now, exp_ok, exp_all, first_ts, last_ts = False, [], None, 0, 0, None, None
    for ln in path.read_text(errors="replace").splitlines():
        if "KF_S3_WAKE" in ln:
            after = True
            continue
        if not after:
            continue
        m = re.search(r"\s(\d+\.\d+): (kvm_hv_\w+): (.*)$", ln)
        if not m:
            continue
        ts, ev, body = float(m[1]), m[2], m[3]
        first_ts = first_ts or ts
        last_ts = ts
        if ev == "kvm_hv_stimer_set_count" and body.endswith("host 0"):
            c = int(re.search(r"count (\d+)", body)[1])
            if c > 100000:
                counts.append(c)
        elif ev in ("kvm_hv_stimer_start_periodic", "kvm_hv_stimer_start_one_shot") and now is None:
            now = int(re.search(r"time_now (\d+)", body)[1])
        elif ev == "kvm_hv_stimer_expiration":
            exp_all += 1
            exp_ok += bool(re.search(r"send result 0\b", body))
    span = (last_ts - first_ts) if first_ts else 0
    return counts, now, exp_ok, exp_all, span


def main():
    print("| cycle | guest one-shot counts written (s, min..max) | KVM time_now at first start (s) | expiries (send result 0 / all) in trace span (s) |")
    print("|---|---|---|---|")
    for p in sorted(Path(sys.argv[1]).glob("*_b*/c*_wake_trace_head.txt")):
        counts, now, ok, allx, span = one(p)
        cs = "%.3f..%.3f (%d writes)" % (min(counts) / 1e7, max(counts) / 1e7, len(counts)) if counts else "none"
        print("| %s | %s | %s | %d / %d in %.2f |" % (p.parent.name + "/" + p.name.split("_")[0], cs,
              "%.3f" % (now / 1e7) if now else "-", ok, allx, span))


if __name__ == "__main__":
    main()
