#!/usr/bin/env python3
"""usage: stalls.py <qemu.log copy> <summary.json>  -> per attempt (qlog line ranges of summary.json): longest 'mem large apply' ms,
last VA_WORST apply_max/split_max (cumulative per boot), ceiling lines, M1/M2/DEAD markers."""
import json, re, sys
L = open(sys.argv[1], errors="replace").read().splitlines()
S = json.load(open(sys.argv[2]))
for a in S:
    if a.get("relaunch"): continue
    lo, hi = a["qlog"][0] or 0, a["qlog"][2] or len(L)
    seg = L[lo:hi]
    ap = [(int(m.group(1)), i + lo + 1) for i, l in enumerate(seg) for m in [re.search(r"mem large apply .* in (\d+) ms", l)] if m]
    vw = [l for l in seg if "VA_WORST" in l]
    last = re.search(r"apply_max_ms=(\d+) split_max_ms=(\d+) slow_applies=(\d+) slow_splits=(\d+)", vw[-1]) if vw else None
    nce = sum(1 for l in seg if "slot pool exhausted" in l)
    dead = [l[:160] for l in seg if re.search(r"DEAD", l)][:3]
    m1 = sum(1 for l in seg if "ForeignClass" in l)
    print(f"attempt {a['attempt']}: end={a['end']} confirm_to_end={a['confirm_to_end']} vram_peak={a['vram_peak']} lines {lo}-{hi} | longest large apply {max(ap) if ap else None} ms (line) | VA_WORST(cum) apply/split max/slow_applies/slow_splits {last.groups() if last else None} | ceiling_lines {nce} | M1 {m1} | DEAD lines {len(dead)} {dead}")
