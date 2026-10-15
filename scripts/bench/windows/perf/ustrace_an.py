#!/usr/bin/env python3
"""ustrace_an.py TRACE [FPS] : analyse KF3_USTORE_TRACE lines: P id n=<k> off,off.. | A id off ns | C id at n ns"""
import sys, collections
fps = float(sys.argv[2]) if len(sys.argv) > 2 else None
P = []; C = []; A = []; t0 = tl = None
for l in open(sys.argv[1]):
    f = l.split()
    if len(f) < 3: continue
    t = int(f[0]); k = f[1]
    if t0 is None: t0 = t
    tl = t
    if k == "P": P.append((t, int(f[2]), [int(x, 16) for x in f[4].split(",")] if len(f) > 4 else []))
    elif k == "C": C.append((t, int(f[2]), int(f[3], 16), int(f[4]), int(f[5])))
    elif k == "A": A.append((t, int(f[2]), int(f[3], 16), int(f[4])))
dur = (tl - t0) / 1e9
print("window %.2f s, lines P=%d C=%d A=%d" % (dur, len(P), len(C), len(A)))
print("prepares/s %.0f, copies/s %.0f, arms/s %.0f (arm time avg %.1f us)" % (len(P)/dur, len(C)/dur, len(A)/dur, (sum(a[3] for a in A)/max(1,len(A)))/1e3))
sizes = [c[3] for c in C]; ns = [c[4] for c in C]
print("copy sizes: n=%d total %.1f KB/s (%.0f B/s)  min %d  max %d" % (len(sizes), sum(sizes)/dur/1e3, sum(sizes)/dur, min(sizes), max(sizes)))
b = collections.Counter()
for s in sizes: b["<=8" if s <= 8 else "<=64" if s <= 64 else "<=256" if s <= 256 else "<=1K" if s <= 1024 else "<=4K" if s <= 4096 else "<=16K" if s <= 16384 else ">16K"] += 1
print("copy size histogram:", dict(b))
tot_ns = sum(ns); print("copy time total %.3f s of %.2f s window (%.2f %%); avg %.2f us, per-byte %.1f ns" % (tot_ns/1e9, dur, 100*tot_ns/1e9/dur, tot_ns/len(ns)/1e3, tot_ns/max(1,sum(sizes))))
by = collections.defaultdict(list)
for s, n in zip(sizes, ns): by["<=256" if s <= 256 else "<=4K" if s <= 4096 else ">4K"].append(n)
for k2, v in by.items(): v.sort(); print("  %s: n=%d p50 %.1f us p99 %.1f us max %.1f us" % (k2, len(v), v[len(v)//2]/1e3, v[int(len(v)*.99)]/1e3, v[-1]/1e3))
big = [c for c in C if c[3] > 256]
print("reads > 256 B: %d (%.1f %%)" % (len(big), 100*len(big)/len(C)))
# windows
chans = sorted(set(p[1] for p in P)); print("channels (cache ids) in window:", len(chans), chans[:12])
per = collections.defaultdict(set); allw = collections.Counter()
for t, k, ws in P:
    pass
ids = {}
for l in open(sys.argv[1]):
    f = l.split()
    if len(f) > 4 and f[1] == "P":
        cid = int(f[2]); 
        for w in f[4].split(","): per[cid].add(int(w, 16)); allw[int(w, 16)] += 1
print("distinct windows per channel:", {c: len(v) for c, v in sorted(per.items())})
union = set().union(*per.values()) if per else set()
print("distinct windows overall: %d; summed per channel: %d -> shared slices (touched by >1 channel): %d" % (len(union), sum(len(v) for v in per.values()), sum(1 for w in union if sum(1 for v in per.values() if w in v) > 1)))
req = [len(p[2]) for p in P]; print("windows required per prepare: mean %.2f max %d" % (sum(req)/len(req), max(req)))
if fps: print("per frame at %.1f fps: prepares %.0f, copies %.0f, bytes %.0f" % (fps, len(P)/dur/fps, len(C)/dur/fps, sum(sizes)/dur/fps))
# replay: per-channel LRU of size R, and shared LRU of size S over (window) keys
def replay_channel(R):
    caches = collections.defaultdict(collections.OrderedDict); hit = miss = 0
    for t, cid, ws in P:
        pass
    return None
seq = []
for l in open(sys.argv[1]):
    f = l.split()
    if len(f) > 4 and f[1] == "P": seq.append((int(f[2]), [int(w, 16) for w in f[4].split(",")]))
def lru(cap, key):
    od = collections.OrderedDict(); h = m = 0
    for cid, ws in seq:
        for w in ws:
            k = key(cid, w)
            if k in od: od.move_to_end(k); h += 1
            else:
                m += 1; od[k] = 1
                if len(od) > cap: od.popitem(last=False)
    return h, m
def lru_pc(R):
    cs = collections.defaultdict(collections.OrderedDict); h = m = 0
    for cid, ws in seq:
        od = cs[cid]
        for w in ws:
            if w in od: od.move_to_end(w); h += 1
            else:
                m += 1; od[w] = 1
                if len(od) > R: od.popitem(last=False)
    return h, m
tot = sum(len(ws) for _, ws in seq)
print("replay (window accesses %d):" % tot)
for R in (1, 2, 4, 8, 16): h, m = lru_pc(R); print("  per-channel LRU %2d views: hit %.2f %%  misses/s %.0f" % (R, 100*h/tot, m/dur))
for S in (16, 64, 256): 
    h, m = lru(S, lambda c, w: w); print("  shared LRU keyed by store offset %3d views: hit %.2f %%  misses/s %.0f" % (S, 100*h/tot, m/dur))
    h, m = lru(S, lambda c, w: (c, w)); print("  shared LRU keyed by (channel,offset) %3d views: hit %.2f %%  misses/s %.0f" % (S, 100*h/tot, m/dur))
