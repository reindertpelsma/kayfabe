#!/usr/bin/env python3
"""usage: tdr_ctx.py <outdir(with polls.jsonl,summary.json)> <qemu.log copy> <sampler file>
For each attempt: first LiveKernelEvent (guest clock) -> host epoch (offset from the polls) -> qemu.log line (sampler) -> host-side
activity in the 5 s before it (heartbeat counter deltas and any apply/map/refusal/error lines)."""
import json, re, sys, datetime, bisect, pathlib
out = pathlib.Path(sys.argv[1]); L = open(sys.argv[2], errors="replace").read().splitlines()
samp = [tuple(map(float, l.split())) for l in open(sys.argv[3]) if len(l.split()) == 2]
ts = [s[0] for s in samp]; ln = [s[1] for s in samp]
def line_at(t):
    i = bisect.bisect(ts, t); return int(ln[min(max(i - 1, 0), len(ln) - 1)])
def iso(s): return datetime.datetime.fromisoformat(s.split(" ")[0]).timestamp()
polls = [json.loads(l) for l in open(out / "polls.jsonl")]
def hbs(lines): return [l for l in lines if l.startswith("kf3: family=")]
def g(l, k):
    m = re.search(k + r"=(\d+)", l); return int(m.group(1)) if m else 0
for att in sorted({p["att"] for p in polls}):
    P = [p for p in polls if p["att"] == att]
    base = P[0]["nlke"]; first = next((p for p in P if p["nlke"] > base), None)
    print(f"== attempt {att}: polls={len(P)} baseline_lke={base}")
    if not first: print("   no LiveKernelEvent seen"); continue
    gt = min(iso(x) for x in first["lke"][: first["nlke"] - base]) if first["lke"] else None
    offs = [p["host"] - iso(p["guest"]) for p in P if p.get("guest")]
    off = sum(offs) / len(offs)
    # LKE list is newest-first; the oldest of the new ones is the first event
    new = first["lke"][: first["nlke"] - base]; gt = min(iso(x) for x in new)
    ht = gt + off; Lend = line_at(ht); Lbeg = line_at(ht - 5)
    seg = L[Lbeg:Lend]
    print(f"   first LKE guest={gt:.1f} -> host epoch {ht:.1f} (offset spread {max(offs)-min(offs):.1f} s) -> qemu.log lines {Lbeg}..{Lend} (5 s window, {len(seg)} lines)")
    h = hbs(L[max(0, Lbeg - 400):Lend + 50])
    if len(h) >= 2:
        print(f"   heartbeats around: first trapped={g(h[0],'trapped')} applied={g(h[0],'applied')} refused={g(h[0],'refused')} -> last trapped={g(h[-1],'trapped')} applied={g(h[-1],'applied')} refused={g(h[-1],'refused')}")
    kw = re.compile(r"mem large apply|refus|error|fail|abort|walk capacity|UNSERVICED|DISABLE_CHANNELS|panic|timed out|NOT complete|late", re.I)
    c = {}
    for l in seg:
        if l.startswith(("kf3: family=", "kf3: display fps")): continue
        k = kw.search(l)
        if k: c[k.group(0).lower()] = c.get(k.group(0).lower(), 0) + 1
    print("   event counts in window:", c)
    ap = [int(m.group(1)) for l in seg for m in [re.search(r"in (\d+) ms", l)] if m and "large apply" in l]
    print("   large apply ms in window:", sorted(ap)[-5:] if ap else "none")
    print("   sample non-heartbeat lines:")
    for l in [x for x in seg if not x.startswith(("kf3: family=", "kf3: display fps"))][-8:]: print("     ", l[:170])
