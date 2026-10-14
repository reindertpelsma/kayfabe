#!/usr/bin/env python3
"""Print DxgKrnl CSV events whose id is in a list, inside a UTC window. usage: etwid.py etwwin.py csv HH:MM:SS HH:MM:SS id,id,..."""
import sys
src = open(sys.argv[1]).read().replace("main()\n", "")
m = {}; exec(compile(src, "etwwin", "exec"), m)
a, b = m["tod"](sys.argv[3]), m["tod"](sys.argv[4]); ids = {int(x) for x in sys.argv[5].split(",")}
for t, eid, typ, pid, tid, cpu, data in m["parse"](sys.argv[2]):
    x = t.hour * 3600 + t.minute * 60 + t.second + t.microsecond / 1e6
    if a <= x <= b and eid in ids:
        print(f"{t.strftime('%H:%M:%S.%f')} {eid:4d} {typ:5s} pid={pid} tid={tid} " + " ".join(data)[:170])
