#!/usr/bin/env python3
"""s3b_table.py : fps of the mode-3 (read and drop), stream-load and control boots, per API."""
import re, os, statistics as st
EV = "/data/claude-perf2/ev/heaven"
def fps(label):
    p = f"{EV}/{label}.out"
    if not os.path.exists(p): return None
    t = re.sub(r"\d\d:\d\d:\d\d", " ", open(p, errors="replace").read().replace("&nbsp;", " ").replace("\n", " "))
    m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+)", t)
    return float(m.group(2)) if m else None
groups = {
 "ctl (28a0ee9c, counters)": ["m2ctl1", "m2ctl2", "m2ctl3"],
 "ctl (db8817ac, counters)": ["m3ctl"],
 "ctl (stream binary, counters)": ["m4ctl"],
 "mode 2: scan off, no reads": ["m2exp1", "m2exp2", "m2exp3"],
 "mode 3: BAR1/RAM read, dropped unscanned": ["m3a", "m3b"],
 "stream loads (KF3_BAR1_STREAM_LOAD=1), scan on": ["m4a", "m4b"],
}
for api in ("d3d11", "gl"):
    print("==", api)
    for g, tags in groups.items():
        v = [(t, fps(f"{t}_{api}")) for t in tags]
        xs = [x for _, x in v if x]
        if xs:
            print("  %-46s %s  median %.2f" % (g, ", ".join(f"{t}={x}" for t, x in v), st.median(xs)))
