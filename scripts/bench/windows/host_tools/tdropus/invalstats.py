#!/usr/bin/env python3
"""Per run: VA-thread invalidate statistics from kf3 status lines (2 s windows): total, window avg, cumulative max and the
window where each new maximum appeared, windows with avg > 12 ms, and the TDR cycle lines for alignment."""
import re, sys
for path in sys.argv[1:]:
    t = None; last_max = 0; jumps = []; slow_avg = []; tot = 0; cyc = []; n = 0
    for ln in open(path, errors="replace"):
        n += 1
        m = re.search(r"(?:WTRACE t=|RELAY-LAG t=|VCPU-MAX t=|GSPQ t=|mem t=)([0-9.]+)", ln)
        if m: t = m.group(1)
        if "Running -> Suspending" in ln: cyc.append(f"line{n}@{t}")
        if "phase=Running" in ln:
            v = re.search(r"vat\[invals=(\d+) arrive->clear_avg_us=(\d+) max_us=(\d+)", ln)
            i = re.search(r"inval=(\d+) walks=\S+ cleared=(\d+)", ln)
            if not v: continue
            w, avg, mx = int(v.group(1)), int(v.group(2)), int(v.group(3))
            if i: tot = int(i.group(1))
            if mx > last_max:
                jumps.append(f"{mx/1000:.1f}ms@line{n}(t~{t},win_invals={w})"); last_max = mx
            if w and avg > 12000: slow_avg.append(f"line{n}:avg{avg/1000:.1f}ms/{w}")
    print(f"{path.split('/')[-2]}: invals={tot} max={last_max/1000:.1f}ms new-max-at={jumps[-4:]} windows_avg>12ms={slow_avg} tdr={cyc[:4]}")
