#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""summarize_run.py RUN_DIR -- one table row per cycle from the c*_result.json files of s3vfio.py (STATUS: RESEARCH,
2026-11-24; evidence tool of traces/vfio_s3_control_20261124/). Reads data only, prints Markdown."""
import json
import re
import sys
from pathlib import Path


def offset_jump(rec):
    """The guest TSC offset change at the wake (cycles), from the first AFTER kvm_write_tsc_offset line of vCPU 0."""
    for line in rec.get("trace", {}).get("tsc_offset", []):
        m = re.search(r"AFTER vcpu=0 prev=(\d+) next=(\d+)", line)
        if m:
            d = (int(m[2]) - int(m[1])) % (1 << 64)
            return d - (1 << 64) if d >= 1 << 63 else d
    return None


def main():
    run = Path(sys.argv[1])
    print("| arm | boot | cycle | QEMU | verdict | ssh back (s) | exits/s 60 s after wake | TSC page unchanged | TSC offset jump (cycles) | nvlddmkm 153 | reasons |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for f in sorted(run.glob("*_b*/c*_result.json"), key=lambda p: (p.stat().st_mtime)):
        r = json.loads(f.read_text())
        q = Path(r.get("qemu", "")).parent.name or "?"
        q = {"bin": "distro 10.2.1", "build-patched": "10.2.4+hyperv-wake", "build-stock": "10.2.4"}.get(q, q)
        late = (r.get("rates_late") or {}).get("exits", "-")
        print("| %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s |" % (
            r.get("arm"), r.get("label"), r.get("cycle"), q, r.get("verdict"), r.get("back_after_s", "-"), late,
            r.get("tsc_page_unchanged") if r.get("tsc_page") else "no page (no hv-time)", offset_jump(r),
            r.get("event153_nvlddmkm", "-"), "; ".join(r.get("reasons") or []) or "-"))


if __name__ == "__main__":
    main()
