#!/usr/bin/env python3
"""Print qemu.log lines a..b (or a time window) prefixed with the last WTRACE t= stamp; drop VSync/ack lines."""
import re, sys
path = sys.argv[1]; t0 = float(sys.argv[2]); t1 = float(sys.argv[3])
pat = re.compile(r"WTRACE t=([0-9.]+)")
t = 0.0
skip = re.compile(sys.argv[4]) if len(sys.argv) > 4 else None
for ln in open(path, errors="replace"):
    m = pat.search(ln)
    if m:
        t = float(m.group(1))
        if "VSYNC" in ln or "WRITE 0x611800" in ln:
            continue
    if t < t0 or t > t1:
        continue
    if skip and skip.search(ln):
        continue
    print(f"{t:.3f} {ln.rstrip()[:240]}")
