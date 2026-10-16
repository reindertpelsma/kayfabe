#!/usr/bin/env python3
"""kayfabe_rows.py DIR : metrics for kayfabe Heaven runs. DIR holds NAME.dmon (host `nvidia-smi dmon`, header line first) and NAME.fps
(Heaven's `FPS:` line, timestamps glued to the number as Heaven's log prints them). Prints JSON lines + median rows per API."""
import glob, json, os, re, statistics as st, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from analyze import analyze
d = sys.argv[1]
rows = []
for f in sorted(glob.glob(os.path.join(d, "*.dmon"))):
    n = os.path.basename(f)[:-5]
    try:
        t = open(f[:-5] + ".fps").read()
    except OSError:
        continue
    ts = re.search(r"HLOG (\d\d:\d\d:\d\d)", t)
    m = ts and re.search(r"(?<!Min )(?<!Max )FPS:\s+([\d.]+?)" + ts.group(1), t)  # Heaven glues the line's timestamp to each number
    if not m:
        continue
    r = analyze(f, float(m.group(1)), n)
    rows.append(r)
    print(json.dumps(r))
