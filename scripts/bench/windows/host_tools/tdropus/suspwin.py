#!/usr/bin/env python3
"""For each FLUSHSCHEDULER_SUSPEND->RESUME window in a DxgKrnl CSV: duration, paging-op starts (324), op stops (325),
paging DMA starts (175 PAGING) and fence CPU/GPU signals (551/552) inside the window."""
import sys, importlib.util
src = open(sys.argv[1]).read().replace("main()\n", "")
m = {}; exec(compile(src, "etwwin", "exec"), m)
ev = list(m["parse"](sys.argv[2]))
open_ = None
for t, eid, typ, pid, tid, cpu, data in ev:
    if eid == 360 and data and "SUSPEND" in data[1]:
        open_ = (t, pid, tid, data[1]); cnt = {324: 0, 325: 0, 175: 0, 551: 0, 552: 0}
        continue
    if open_ is not None:
        if eid in cnt: cnt[eid] += 1
        if eid == 360 and data and "RESUME" in data[1]:
            d = (t - open_[0]).total_seconds() * 1000
            print(f"{open_[0].strftime('%H:%M:%S.%f')} pid={open_[1]} {d:9.2f} ms  opStart={cnt[324]} opStop={cnt[325]} pagingDMA={cnt[175]} sig551={cnt[551]} sig552={cnt[552]}")
            open_ = None
if open_: print("OPEN at end:", open_)
