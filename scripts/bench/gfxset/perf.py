#!/usr/bin/env python3
"""perf.py <resdir> — the RECORDED numbers (GSET_VAL), guest vs bare metal, as ratios. Never a grade.
⊘ A guest FASTER than bare metal is flagged, not celebrated: a refused MC_SERVICE_INTERRUPTS once ended
GPU waits early (v3-mapfix 56032c46) and made vkpeak read faster in the guest than on bare metal.
Rows: numeric values present on both sides; 'higher is better' unless the key says ns/us/ms/secs/time.
The ratio is always oriented so that > 1 means the guest did BETTER than bare metal."""
import re, sys, collections
R = sys.argv[1]
def vals(paths):
    d = collections.OrderedDict()
    for p in paths:
        try:
            for l in open(p, errors="replace"):
                m = re.match(r"GSET_VAL side=\S+ item=(\S+) key=(\S+) val=(\S+)", l)
                if m: d[(m.group(1), m.group(2))] = m.group(3)
        except OSError:
            pass
    return d
def num(v):
    m = re.match(r"^([0-9]+(?:\.[0-9]+)?)", v or "")
    return float(m.group(1)) if m else None
h = vals([f"{R}/host.dig"]); g = vals([f"{R}/guest.dig", f"{R}/iso/guest.dig"])
print("| item | value | bare metal | guest | guest/bare | note |\n|---|---|---|---|---|---|")
for (it, k), hv in h.items():
    gv = g.get((it, k))
    if it == "blender_opendata":   # no_sync=..,total=..,spm=.. → samples per minute is the throughput
        hm = re.search(r"spm=([0-9.]+)", hv); gm = re.search(r"spm=([0-9.]+)", gv or "")
        a, b = (float(hm.group(1)) if hm else None), (float(gm.group(1)) if gm else None); k = k + " (samples/min)"
    else:
        a, b = num(hv), num(gv)
    if a is None or b is None or a == 0:
        continue
    lower_better = bool(re.search(r"(_ns|_us|_ms|secs|time|wall)", k))
    r = b / a if not lower_better else a / b
    note = "⚠ FASTER than bare metal — check for early-ended waits" if r > 1.03 else ""
    print(f"| {it} | {k} | {a:g} | {b:g} | {r:.2f} | {note} |")
