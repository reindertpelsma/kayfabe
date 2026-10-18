#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""align.py RUNDIR: join the guest's per-iteration QPC stamps (raw.csv), the host's fence-word observer (fence.txt) and the
bpftrace trace (trace.txt) on ONE clock (host CLOCK_MONOTONIC). Guest QPC -> host clock by matching iteration PERIODS to doorbell
periods (shift search), then offset = median(D0_host - mid(Execute call)). Prints per-hop median/p90/max (us, relative to the first doorbell D0)."""
import sys, os, statistics as st, bisect
d = sys.argv[1]
G = []   # guest rows
for l in open(os.path.join(d, 'raw.csv')):
    p = l.strip().split(',')
    if p and p[0].isdigit(): G.append(tuple(int(x) for x in p))
F = []
if os.path.exists(os.path.join(d, 'fence.txt')):
    for l in open(os.path.join(d, 'fence.txt')):
        if l.startswith('F '):
            p = l.split()
            if len(p) == 4: F.append((int(p[1]), int(p[2]), int(p[3], 16)))
M = []; W = []; Z = []; U = []
for l in open(os.path.join(d, 'trace.txt')):
    p = l.split()
    if not p: continue
    if p[0] == 'M' and p[3] == '2': M.append((int(p[1]), int(p[2]), p[4][-6:], p[5]))
    elif p[0] == 'W': W.append((int(p[1]), p[3], int(p[2])))
    elif p[0] == 'Z': Z.append((int(p[1]), p[3], p[4], int(p[5][2:])))
    elif p[0] == 'U': U.append((int(p[1]), int(p[2]), p[3], p[4]))
DB = 'bb0090'
D0 = []; last = -10**12
for m in M:
    if m[2] == DB:
        if m[0] - last > 300_000: D0.append(m[0])
        last = m[0]
n = len(G)
Pg = [(G[i + 1][1] - G[i][1]) / 10.0 for i in range(n - 1)]       # us
Ph = [(D0[j + 1] - D0[j]) / 1e3 for j in range(len(D0) - 1)]
best = None
for s in range(0, len(Ph) - 300):
    e = sum(abs(Ph[s + k] - Pg[k]) for k in range(100, 300))
    if best is None or e < best[0]: best = (e, s)
sh = best[1]
print("iterations guest=%d host_D0=%d; period-match shift=%d mean|dP|=%.2f us over 200 iterations" % (n, len(D0), sh, best[0] / 200))
off = []
for i in range(100, min(n, len(D0) - sh) - 1):
    mid = (G[i][1] + G[i][2]) / 2 / 10.0                           # us (guest clock)
    off.append(D0[sh + i] / 1e3 - mid)
o0 = st.median(off)
def g2h(q): return None
# linear drift fit guest->host on the matched D0s
xs = [G[i][1] / 10.0 for i in range(100, min(n, len(D0) - sh) - 1)]; ys = [D0[sh + i] / 1e3 for i in range(100, min(n, len(D0) - sh) - 1)]
mx = st.mean(xs); my = st.mean(ys)
slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)
print("clock fit: slope=%.8f  (D0 - exec-midpoint) offset spread: p10=%.1f med=%.1f p90=%.1f us" % (slope, sorted(off)[len(off) // 10] - o0, 0, sorted(off)[9 * len(off) // 10] - o0))
def hostus(q): return my + slope * (q / 10.0 - mx)               # guest QPC ticks -> host us
Ft = sorted((t / 1e3, c, v) for t, c, v in F)
Mt = [(t / 1e3, tid, o, v) for t, tid, o, v in M]
Mts = [m[0] for m in Mt]
rows = {}
def add(k, x): rows.setdefault(k, []).append(x)
def q(v, f): v = sorted(v); return v[min(len(v) - 1, int(f * len(v)))]
for i in range(150, min(n, len(D0) - sh) - 2):
    t0 = D0[sh + i] / 1e3; t1 = D0[sh + i + 1] / 1e3
    a = hostus(G[i][1]) - t0; b = hostus(G[i][2]) - t0; c = hostus(G[i][3]) - t0; dd = hostus(G[i][4]) - t0
    add('guest Execute call start (rel D0)', a); add('guest Execute call end (rel D0)', b); add('guest fence seen (rel D0)', dd)
    add('period (D0 -> next D0)', t1 - t0)
    lo = bisect.bisect_left(Mts, t0); hi = bisect.bisect_left(Mts, t1)
    seg = Mt[lo:hi]
    db = [m for m in seg if m[2] == DB]
    add('doorbells per submit', len(db)); add('last doorbell (rel D0)', db[-1][0] - t0)
    add('trapped writes per submit', len(seg))
    isr = [m for m in seg if m[2].startswith('b81')]
    add('interrupt-tree writes per submit', len(isr))
    if isr:
        add('first interrupt-tree write (rel D0)', isr[0][0] - t0); add('last interrupt-tree write (rel D0)', isr[-1][0] - t0)
    fl = bisect.bisect_left([f[0] for f in Ft], t0 - 50);
    fs = [f for f in Ft[fl:fl + 6] if f[0] < t1]
    if fs: add('fence word first change (rel D0)', fs[0][0] - t0); add('fence word last change (rel D0)', fs[-1][0] - t0)
    for (tw, comm, tid) in [(x[0] / 1e3, x[1], x[2]) for x in W]:
        pass
    wl = bisect.bisect_left([x[0] / 1e3 for x in W], t0)
    wk = [x for x in W[wl:wl + 400] if x[0] / 1e3 < t1 and x[1].startswith('kf3-worker')]
    if wk: add('first worker wake (rel D0)', wk[0][0] / 1e3 - t0); add('worker wakes per submit', len(wk))
    zl = bisect.bisect_left([x[0] / 1e3 for x in Z], t0)
    zz = [x for x in Z[zl:zl + 400] if x[0] / 1e3 < t1 and x[3] == 8]
    add('worker eventfd writes per submit', len(zz))
for k, v in rows.items():
    print("%-42s n=%5d med=%9.1f p90=%9.1f max=%9.1f" % (k, len(v), st.median(v), q(v, .9), max(v)))
