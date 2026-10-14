#!/usr/bin/env python3
"""For every window LATCH in a kf3 display write trace: the VSync raise before it, the guest's ack of that VSync
(W1C 0x611800), and their order. Flags the last latch before a >1 s flip gap (the flip that never completed)."""
import re, sys
ev = []
for ln in open(sys.argv[1], errors="replace"):
    m = re.search(r"WTRACE t=([0-9.]+) (VSYNC|WRITE 0x611800|LATCH window 0|WRITE 0x690000)", ln)
    if m: ev.append((float(m.group(1)), m.group(2)))
puts = [t for t, k in ev if k == "WRITE 0x690000"]
rows = []
last_vs = None; last_ack = None
pending = []
for i, (t, k) in enumerate(ev):
    if k == "VSYNC": last_vs = t; last_ack = None
    elif k == "WRITE 0x611800":
        if last_ack is None: last_ack = t
    elif k == "LATCH window 0":
        # the ack of this VSync may come after the latch: look ahead to the first ack after last_vs
        ack = None
        for t2, k2 in ev[i - 10 if i >= 10 else 0:i + 10]:
            if k2 == "WRITE 0x611800" and last_vs is not None and t2 >= last_vs:
                ack = t2; break
        nxt = [p for p in puts if p > t]
        gap = (nxt[0] - t) if nxt else 99
        rows.append((t, last_vs, ack, gap))
def fmt(x, base): return "None" if x is None else f"{(x - base) * 1000:+.3f}"
ok = [r for r in rows if r[3] < 1.0 and r[2] is not None]
d = sorted((r[2] - r[0]) * 1000 for r in ok)
print(f"latches={len(rows)}  ack-minus-latch ms over completed-looking flips: n={len(d)} min={d[0]:.3f} p10={d[len(d)//10]:.3f} p50={d[len(d)//2]:.3f} p90={d[9*len(d)//10]:.3f} max={d[-1]:.3f}")
print("ack before latch (ack-latch<0):", sum(1 for x in d if x < 0), "of", len(d))
for r in rows:
    if r[3] >= 1.0:
        print(f"STUCK? latch {r[0]:.6f}  vsync {fmt(r[1], r[0])} ms  guest-ack {fmt(r[2], r[0])} ms  next PUT after {r[3]:.2f} s")
