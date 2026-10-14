#!/usr/bin/env python3
"""Pair DxgKrnl 450 (submit to hw, hContext, fenceId) with 451 (completed, hContext, fenceId); list unpaired before T."""
import sys, importlib.util
spec = importlib.util.spec_from_file_location("etwwin", sys.argv[1]); m = importlib.util.module_from_spec(spec)
src = open(sys.argv[1]).read().replace("main()\n", "")
exec(compile(src, "etwwin", "exec"), m.__dict__)
path = sys.argv[2]
sub = {}
done = {}
for t, eid, typ, pid, tid, cpu, data in m.parse(path):
    if eid == 450:
        sub[(data[0], data[1])] = (t, pid, data)
    elif eid == 451:
        done[(data[0], data[1])] = t
lat = []
for k, (t, pid, data) in sorted(sub.items(), key=lambda kv: kv[1][0]):
    d = done.get(k)
    if d is None:
        print("UNCOMPLETED", t.strftime("%H:%M:%S.%f"), pid, " ".join(data))
    else:
        dt = (d - t).total_seconds()
        lat.append(dt)
        if dt > 0.2:
            print(f"SLOW {dt*1000:8.1f} ms", t.strftime("%H:%M:%S.%f"), d.strftime("%H:%M:%S.%f"), pid, " ".join(data))
lat.sort()
print("n", len(lat), "p50", lat[len(lat)//2] if lat else None, "p99", lat[int(len(lat)*0.99)] if lat else None)
