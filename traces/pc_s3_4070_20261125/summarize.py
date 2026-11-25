#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""summarize.py -- per-cycle and per-boot tables of runs/<run>/ (STATUS: RESEARCH, 2026-11-25; evidence tool of
traces/pc_s3_4070_20261125/). usage: summarize.py RUNDIR [RUNDIR...]   Prints Markdown; reads only c*_result.json and boot_summary.json."""
import json
import sys
from pathlib import Path


def hms(ts):
    return ts[-9:-1] if ts else "-"


def main():
    print("| boot | cycle | verdict | cycle start (UTC) | guest Kernel-Power 42, entering sleep (UTC) | QMP suspended after request (s) | wake (UTC) | ssh back (s) | nvlddmkm 153 | kf3 KEPT/GSP-RESUME | explorer | nvidia-smi | reasons |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    boots = []
    for d in sys.argv[1:]:
        for b in sorted(Path(d).glob("*_b[0-9]*")):
            for f in sorted(b.glob("c[0-9]*_result.json"), key=lambda p: int(p.name[1:].split("_")[0])):
                r = json.loads(f.read_text())
                k = r.get("kf3_delta", {})
                kp42 = next((e.split("T")[1][:8] for e in r.get("events", []) if " 42 Microsoft-Windows-Kernel-Power" in e), "-")
                print("| %s | %s | %s | %s | %s | %s | %s | %s | %s | %s/%s | %s | %s | %s |" % (
                    b.name, r.get("cycle"), r.get("verdict"), r.get("cycle_start_utc", "-"), kp42, r.get("suspend_wait_s", "-"), hms(r.get("wake_utc")),
                    r.get("back_after_s", "-"), r.get("event153_nvlddmkm", "-"), k.get("kept"), k.get("gsp_resume"), r.get("explorer_after"),
                    (r.get("nvidia_smi") or "-").replace("NVIDIA GeForce ", ""), "; ".join(r.get("reasons", [])) or "-"))
            s = b / "boot_summary.json"
            if s.exists():
                j = json.loads(s.read_text())
                boots.append((b.name, j["host_mmuWalkMap_lines"], j["host_no_memory_lines"], j["qemu_log"].get("status_0x51"), " ".join(j["qmp_all"])))
    print()
    print("| boot | host dmesg mmuWalkMap lines | host NV_ERR_NO_MEMORY lines | QEMU log 0x51 lines | QMP events |")
    print("|---|---|---|---|---|")
    for b in boots:
        print("| %s | %s | %s | %s | %s |" % b)


main()
