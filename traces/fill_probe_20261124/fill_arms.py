#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""fill_arms.py -- runs the kf-fill-probe arms of PREREGISTRATION.md (Experiment 1) in order, on a rented box, as root.
Evidence tool of traces/fill_probe_20261124/, not a gate.

  fill_arms.py <kf-fill-probe binary> <out dir> [arm ...] [--leave-free-mib=N]

Each arm is one process. After each arm the new host dmesg lines are kept (NVRM, mmuWalk, Xid), and the run stops at the
first Xid. Box output is data, only parsed.
"""
import json
import re
import subprocess
import sys
import time
from pathlib import Path

ALIAS = "efa00000"
ARMS = {
    "A1": ["zero", "floor", ALIAS, "0", "0"],
    "A2": ["fault", "floor", ALIAS, "0", "0"],
    "A3": ["zero", "default", ALIAS, "0", "0"],
    "A4": ["fault", "floor", ALIAS, "65536", "4"],
    "A5": ["zero", "floor", ALIAS, "65536", "4"],
    "A6": ["zero", "floor", ALIAS, "0", "0", "32"],
    "A7": ["zero", "floor", ALIAS, "131072", "2", "0", "low"],
    "A8": ["fault", "floor", ALIAS, "131072", "2", "0", "low"],
    "A9": ["zero", "floor", ALIAS, "160000", "3", "0", "low"],
    "A10": ["fault", "floor", ALIAS, "160000", "3", "0", "low"],
    "P1": ["zero", "floor", ALIAS, "131072", "2", "0", "low"],
    "P2": ["fault", "floor", ALIAS, "131072", "2", "0", "low"],
    "P3": ["zero", "floor", ALIAS, "0", "0"],
    "S1": ["zero", "floor", "4000000", "0", "0"],
    "S2": ["zero", "floor", "20000000", "0", "0"],
    "S3": ["zero", "floor", "40000000", "0", "0"],
    "S4": ["zero", "floor", "80000000", "0", "0"],
    "S5": ["zero", "floor", "1efc00000", "0", "0"],
    "S6": ["zero", "default", "1efc00000", "0", "0"],
    "S7": ["fault", "floor", "1efc00000", "0", "0"],
}
KEEP = re.compile(r"NVRM|Xid|mmuWalk|NO_MEMORY|nvidia", re.I)


def dmesg_lines():
    r = subprocess.run(["dmesg"], capture_output=True, text=True)
    return r.stdout.splitlines()


LEAVE_FREE = None


def main():
    global LEAVE_FREE
    args = [a for a in sys.argv[1:] if not a.startswith("--leave-free-mib=")]
    for a in sys.argv[1:]:
        if a.startswith("--leave-free-mib="):
            LEAVE_FREE = int(a.split("=", 1)[1])
    probe, out = args[0], Path(args[1])
    order = args[2:] or ["A1", "A2", "A3", "A4", "A5", "A6"]
    out.mkdir(parents=True, exist_ok=True)
    summary = []
    for arm in order:
        argv = [probe] + ARMS[arm] + ([f"--leave-free-mib={LEAVE_FREE}"] if LEAVE_FREE else [])
        d0 = len(dmesg_lines())
        t = time.time()
        print(f"ARM_START {arm} {' '.join(ARMS[arm])}", flush=True)
        r = subprocess.run(argv, capture_output=True, text=True, timeout=3600)
        secs = round(time.time() - t, 1)
        (out / f"{arm}.out").write_text(r.stdout + ("\n--- stderr ---\n" + r.stderr if r.stderr else ""))
        new = [ln for ln in dmesg_lines()[d0:] if KEEP.search(ln)]
        (out / f"{arm}_dmesg.txt").write_text("\n".join(new) + "\n")
        res = next((ln for ln in r.stdout.splitlines() if ln.startswith(("FILL_PROBE_RESULT", "FILL_PROBE_ERROR"))), "none")
        row = {"arm": arm, "args": ARMS[arm], "leave_free_mib": LEAVE_FREE, "rc": r.returncode, "secs": secs, "result": res,
               "fb_used_mib": [ln.split(" ", 2)[2] for ln in r.stdout.splitlines() if ln.startswith("MEASURE fb_used_mib")],
               "dmesg_mmuwalk": sum("mmuWalk" in ln for ln in new), "dmesg_xid": sum("Xid" in ln for ln in new),
               "dmesg_kept": len(new)}
        summary.append(row)
        print(f"ARM_EXIT {arm} rc={r.returncode} secs={secs} mmuwalk={row['dmesg_mmuwalk']} xid={row['dmesg_xid']} {res}",
              flush=True)
        (out / "summary.json").write_text(json.dumps(summary, indent=1))
        if row["dmesg_xid"]:
            print(f"STOP first Xid in arm {arm}", flush=True)
            break
        time.sleep(3)
    print("FILL_ARMS_EXIT", flush=True)


if __name__ == "__main__":
    main()
