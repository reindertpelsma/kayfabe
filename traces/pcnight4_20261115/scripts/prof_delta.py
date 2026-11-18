#!/usr/bin/env python3
"""usage: prof_delta.py <qemu.log copy> <summary.json> : per attempt, the delta of the cumulative `PROF t=.. bar0_handler n= sum_ms=` between the
CONFIRM line (qlog[1]) and the END line (qlog[2]); also vamgr busy_ms delta, and the display fps line near the end."""
import json, re, sys
L = open(sys.argv[1], errors="replace").read().splitlines()
S = json.load(open(sys.argv[2]))
def last(pat, lo, hi):
    r = None
    for l in L[lo:hi]:
        m = re.search(pat, l)
        if m: r = (l, m)
    return r
for a in S:
    if a.get("relaunch"): continue
    c, e = a["qlog"][1], a["qlog"][2]
    b0 = last(r"bar0_handler n=(\d+) sum_ms=([\d.]+) avg_us=([\d.]+) p50_us=([\d.]+) p90_us=([\d.]+) p99_us=([\d.]+) max_us=([\d.]+)", 0, c)
    b1 = last(r"bar0_handler n=(\d+) sum_ms=([\d.]+) avg_us=([\d.]+) p50_us=([\d.]+) p90_us=([\d.]+) p99_us=([\d.]+) max_us=([\d.]+)", 0, e)
    v0 = last(r"PROF vamgr busy_ms=([\d.]+)", 0, c); v1 = last(r"PROF vamgr busy_ms=([\d.]+)", 0, e)
    d = last(r"display fps\[cap=(\S+) h0\[armed=([\d.]+) tick=([\d.]+)", c, e)
    t = last(r"PROF t=([\d.]+)s", 0, c), last(r"PROF t=([\d.]+)s", 0, e)
    secs = float(t[1][1].group(1)) - float(t[0][1].group(1)) if t[0] and t[1] else None
    n = int(b1[1].group(1)) - int(b0[1].group(1)); ms = float(b1[1].group(2)) - float(b0[1].group(2))
    print(f"attempt {a['attempt']}: window {secs:.0f} s: BAR0 traps {n} ({n/secs:.0f}/s), handler {ms:.0f} ms total = {ms*1000/max(n,1):.2f} us/trap = {100*ms/1000/secs:.2f} % of wall; cumulative p99 {b1[1].group(6)} us max {b1[1].group(7)} us; vamgr busy delta {float(v1[1].group(1))-float(v0[1].group(1)):.0f} ms; display fps line: cap={d[1].group(1) if d else None} tick={d[1].group(3) if d else None}")
