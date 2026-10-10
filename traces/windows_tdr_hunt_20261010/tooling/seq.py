#!/usr/bin/env python3
"""Compact sequence of disable(+)/enable(-)/suspend(S)/schedule events in a qemu.log, with the last seen time stamp."""
import re, sys
t = None
tpat = re.compile(r"(?:WTRACE t=|mem t=|t=)([0-9]+\.[0-9]+)")
out = []
for i, ln in enumerate(open(sys.argv[1], errors="replace")):
    m = tpat.search(ln)
    if m:
        t = m.group(1)
    if "DISABLE_CHANNELS(bDisable=true" in ln:
        n = re.search(r"over (\d+) twin", ln); k = "D+"
    elif "DISABLE_CHANNELS(bDisable=false" in ln:
        n = re.search(r"over (\d+) twin", ln); k = "D-"
    elif "Running -> Suspending" in ln:
        k = "SUSPEND"; n = None
    elif "DISABLE_CHANNELS" in ln and "REFUSE" in ln.upper():
        k = "DREF"; n = None
    else:
        continue
    out.append((i + 1, t, k))
# compress consecutive same-kind
cur = None
for ln, t, k in out:
    if cur and cur[2] == k:
        cur[3] += 1; cur[4] = ln
    else:
        if cur: print(f"line {cur[0]}-{cur[4]} t~{cur[1]} {cur[2]} x{cur[3]}")
        cur = [ln, t, k, 1, ln]
if cur: print(f"line {cur[0]}-{cur[4]} t~{cur[1]} {cur[2]} x{cur[3]}")
