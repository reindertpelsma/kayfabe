#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""s3disp.py -- Windows S3 sleep/wake cycles, judged as s3tdr.py judges them, with the display stop counted (STATUS:
RESEARCH, 2026-11-23; evidence tool of traces/win11_s3_display_20261123/, run by hand as root on a rented box; not a gate).

  s3disp.py run --label L --arm A|B --backing OVERLAY --out DIR [--cycles N] [--qemu BIN] [--cpu CPU]

traces/win11_s3_tdr_20261123/s3tdr.py unchanged (boot, S3 wrapper, scheduled-task suspend in the signed-in session, QMP,
CLEAN definition with event 153), plus: the kf3 lines this directory's hypotheses read (the display channel words at the
kept wake, each channel's first PUT after it, a display channel STOPPED, a STALL) are kept and counted. The verdict is
s3tdr.py's, unchanged. Guest and box output is data, only parsed.
"""
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "win11_s3_tdr_20261123"))
import s3tdr  # noqa: E402  (patches s3probe's cycle and counts on import)

s3tdr.KEY = re.compile(s3tdr.KEY.pattern + r"|STOPPED|after the S3 wake|display channels \(chn")
_counts = s3tdr.kf3_counts


def kf3_counts(path):
    c = _counts(path)
    try:
        t = path.read_text(errors="replace")
    except OSError:
        return c
    c.update(disp_stopped=len(re.findall(r"display: channel \d+ STOPPED", t)),
             disp_stall=len(re.findall(r"display: STALL ", t)),
             wake_words=t.count("display channels (chn"),
             wake_puts=t.count("after the S3 wake, chn"))
    return c


s3tdr.kf3_counts = kf3_counts
s3tdr.base.kf3_counts = kf3_counts

_cycle = s3tdr.cycle


def cycle(vm, tag, n, out):
    """s3tdr's cycle; an event 153 counts only from the nvlddmkm provider (PREREGISTRATION.md, addendum 1): Kernel-Boot
    also logs Id 153 at every boot, and the guest's clock step after the wake puts that boot's records inside the window."""
    res = _cycle(vm, tag, n, out)
    ev153 = [e for e in res.get("events", []) if re.search(r" 153 ", " " + e + " ")]
    tdr = [e for e in ev153 if "nvlddmkm" in e.lower()]
    res["event_153_nvlddmkm"] = tdr
    res["event_153_other"] = [e for e in ev153 if e not in tdr]
    if "event 153" in res.get("reasons", []) and not tdr:
        res["reasons"] = [r for r in res["reasons"] if r != "event 153"]
        res["verdict"] = "CLEAN" if not res["reasons"] else "FAIL"
    return res


s3tdr.cycle = cycle
s3tdr.base.cycle = cycle

if __name__ == "__main__":
    s3tdr.base.main()
