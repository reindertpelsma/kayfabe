#!/usr/bin/env python3
"""gate.py TAG... : the correctness gate of the optimisation loop, per boot (D3D11 run of that boot):
 Heaven completed with a score; host Xid delta; guest 153/4101 events beyond the boot ones (startmarker 'events:' line); mid-run screenshot statistics
 (mean luma, std, distinct 4-bit colours; sane = std > 20 and colours > 50, i.e. not black/uniform); USER_RING_REFUSED / REFUSED lines in the boot's qemu log;
 BAR1 read budget per frame (private-USERD loads, guest PUT reads, GET write-backs, BAR1 pushbuffer copies) from the last two ticks of the benchmark window."""
import re, sys, gzip, os, datetime
EV = os.environ.get("GAPEV", "/data/claude-gap/ev")
for tag in sys.argv[1:]:
    lab = tag.lower() + "_d3d11"
    out = {"tag": tag}
    try:
        t = open(f"{EV}/heaven/{lab}.out", errors="replace").read().replace("&nbsp;", " ")
        tt = re.sub(r"\d\d:\d\d:\d\d", " ", t.replace("\n", " ")); m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+).*?Score:\s+([\d.]+)", tt)
        out["heaven"] = "frames=%s fps=%s score=%s" % m.groups() if m else "NO RESULT"
    except FileNotFoundError: out["heaven"] = "no file"
    try:
        sm = open(f"{EV}/heaven/{lab}/startmarker").read()
        out["events"] = " | ".join(l for l in sm.splitlines() if l.startswith("events:"))
    except FileNotFoundError: out["events"] = "-"
    try:
        sh = open(f"{EV}/heaven/shot_{tag}_d3d11.out", errors="replace").read(); m = re.search(r"SHOT screen=(\S+) mean_luma=([\d.]+) std_luma=([\d.]+) distinct_4bit_colours=(\d+)", sh)
        out["shot"] = ("screen=%s mean_luma=%s std=%s colours=%s sane=%s" % (m.group(1), m.group(2), m.group(3), m.group(4), float(m.group(3)) > 20 and int(m.group(4)) > 50)) if m else "no SHOT line"
    except FileNotFoundError: out["shot"] = "no shot hook"
    try:
        n = 0; last = {}
        for l in gzip.open(f"{EV}/qemulog/{tag}.qemu.log.gz", "rt", errors="replace"):
            if "REFUSED" in l and "USER_RING" in l: n += 1
            m = re.search(r"tick cw .*observes=(\d+) bar1_store_loads=(\d+) put_reads=(\d+) get_writes=(\d+)", l)
            if m: last = tuple(map(int, m.groups()))
        out["refused_lines"] = n; out["cw_totals"] = last
    except FileNotFoundError: out["refused_lines"] = "no log"
    print(" ".join(f"{k}={v}" for k, v in out.items()))
