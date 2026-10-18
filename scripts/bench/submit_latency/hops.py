#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""hops.py RUNDIR [--irqfd FD]: the per-hop table of one traced run (gen_bt.py `tp`/`pub` profile + fence.txt), one clock (host CLOCK_MONOTONIC).
Per submit (window = first doorbell D0 -> next D0, windows 600..1500 us only), times in us from D0 unless named a duration.
Hops are the differences of consecutive event medians' per-iteration values (computed per iteration, then summarised)."""
import sys, os, bisect, statistics as st, collections
d = sys.argv[1]
IRQFD = 130
WMIN, WMAX = 600_000, 1_500_000; PH = None; GAP = 300_000
if '--gap' in sys.argv: GAP = int(sys.argv[sys.argv.index('--gap') + 1]) * 1000
if '--win' in sys.argv: i = sys.argv.index('--win'); WMIN, WMAX = int(sys.argv[i+1])*1000, int(sys.argv[i+2])*1000
if '--phase' in sys.argv: PH = int(sys.argv[sys.argv.index('--phase') + 1])
if '--irqfd' in sys.argv: IRQFD = int(sys.argv[sys.argv.index('--irqfd') + 1])
M = []; W = []; Z = []; QI = []; U = collections.defaultdict(list); F = collections.defaultdict(list)
for l in open(d + '/trace.txt'):
    p = l.split()
    if not p: continue
    if p[0] == 'M' and p[3] == '2': M.append((int(p[1]), p[4][-6:], int(p[2])))
    elif p[0] == 'W' and 'worker' in p[3]: W.append(int(p[1]))
    elif p[0] == 'Z': Z.append((int(p[1]), p[3], int(p[4][3:])))
    elif p[0] == 'U': U[(p[3], p[4])].append(int(p[1]))
    elif p[0] == 'Q' and 'nvidia' in l: QI.append(int(p[1]))
if os.path.exists(d + '/fence.txt'):
    for l in open(d + '/fence.txt'):
        p = l.split()
        if len(p) == 4 and p[0] == 'F': F[int(p[2])].append(int(p[1]))
DBt = [t for t, o, _ in M if o == 'bb0090']
if PH is not None:   # phases of a batched run: bursts of doorbells separated by > 2 s of silence in the loop's tokens; keep the PH-th long burst
    runs = []; cur = [DBt[0]]
    for a, b in zip(DBt, DBt[1:]):
        if b - a > 2_000_000_000: runs.append(cur); cur = []
        cur.append(b)
    runs.append(cur); runs = [r for r in runs if len(r) > 1500]
    DBt = runs[PH]
    lo_t, hi_t = DBt[0], DBt[-1]
    M = [m for m in M if lo_t - 1_000_000 <= m[0] <= hi_t + 1_000_000]; W = [x for x in W if lo_t <= x <= hi_t]; Z = [z for z in Z if lo_t <= z[0] <= hi_t]; QI = [x for x in QI if lo_t <= x <= hi_t]
    for k in list(U): U[k] = [x for x in U[k] if lo_t <= x <= hi_t]
    for k in list(F): F[k] = [x for x in F[k] if lo_t <= x <= hi_t]
D0 = []; last = -10**12
for t in DBt:
    if t - last > GAP: D0.append(t)
    last = t
Mt = [m[0] for m in M]; Wt = W; Zt = [z[0] for z in Z]
Ft = {c: v for c, v in F.items() if len(v) > 100}
# the completion cell = the candidate whose change follows the last interrupt-tree write most tightly (the other is the CPU's submit-side write)
ISRt = [t for t, o, _ in M if o.startswith('b81')]
def lag(v):
    x = [(f - ISRt[bisect.bisect_right(ISRt, f) - 1]) for f in v[20:-20]]
    return st.median(x)
cells = sorted(Ft, key=lambda c: lag(Ft[c]))
rows = collections.defaultdict(list)
def add(k, v): rows[k].append(v)
def first_after(arr, t, lim):
    j = bisect.bisect_left(arr, t)
    return arr[j] if j < len(arr) and arr[j] < lim else None
def last_before(arr, lim, t0):
    j = bisect.bisect_left(arr, lim) - 1
    return arr[j] if j >= 0 and arr[j] >= t0 else None
for i in range(30, len(D0) - 30):
    t0, t1 = D0[i], D0[i + 1]
    if not WMIN < t1 - t0 < WMAX: continue
    lo = bisect.bisect_left(Mt, t0); hi = bisect.bisect_left(Mt, t1)
    seg = M[lo:hi]
    db = [t for t, o, _ in seg if o == 'bb0090']
    isr = [t for t, o, _ in seg if o.startswith('b81')]
    cyc = [t for t, o, _ in seg if o == 'b81408']                      # one LEAF_EN_CLEAR(2) per interrupt service cycle (ISR entry)
    add('period D0 -> next D0', (t1 - t0) / 1e3)
    add('trapped BAR0 writes per submit', len(seg)); add('  of which doorbells (0xbb0090)', len(db)); add('  of which interrupt-tree (0xb81xxx)', len(isr)); add('  interrupt service cycles (EN_CLEAR(2) writes)', len(cyc))
    add('H1  D0 -> last doorbell (3 traps)', (db[-1] - t0) / 1e3)
    w1 = first_after(Wt, t0, t1)
    if w1 is not None: add('H2  D0 -> first worker wake (trap->IPI->thread runs)', (w1 - t0) / 1e3)
    s1 = first_after(U[('serve', 'E')], t0, t1)
    if s1 is not None and w1 is not None: add('H2b first wake -> first serve() entry', (s1 - w1) / 1e3)
    pe = [x for x in U[('pub_entry', 'E')] if t0 <= x < t1]
    if pe:
        add('relay GP entries published per submit', len(pe))
        add('H3  first serve -> LAST pub_entry (relay busy)', (pe[-1] - s1) / 1e3 if s1 else float('nan'))
        add('    last pub_entry (rel D0)', (pe[-1] - t0) / 1e3)
    pm = [x for x in U[('pub_marker', 'E')] if t0 <= x < t1]
    if pm: add('    markers published per submit', len(pm)); add('    last pub_marker (rel D0)', (pm[-1] - t0) / 1e3)
    wg = [x for x in U[('wr_guest_get', 'E')] if t0 <= x < t1]
    if wg: add('    guest GP_GET write-backs per submit', len(wg))
    gi = [x for x in QI if t0 <= x < t1]
    if gi:
        add('    host GPU interrupts (irq 85 nvidia) per submit', len(gi)); add('    first host GPU interrupt (rel D0)', (gi[0] - t0) / 1e3); add('    last host GPU interrupt (rel D0)', (gi[-1] - t0) / 1e3)
        if pm: add('HG  last relay marker publish -> first host GPU interrupt (GPU leg)', (gi[0] - pm[-1]) / 1e3)
    # first injection (eventfd write on the irqfd) before the first interrupt-tree write
    zi = [z for z in Z if False]
    j = bisect.bisect_left(Zt, t0)
    inj = []
    while j < len(Z) and Z[j][0] < t1:
        if Z[j][2] == IRQFD or (Z[j][1].startswith('kf3-worker') and Z[j][2] == IRQFD): inj.append(Z[j][0])
        j += 1
    if inj: add('    irqfd eventfd writes per submit', len(inj))
    if isr:
        i1 = isr[0]
        pre = [x for x in inj if x <= i1]
        if inj: add('    first irqfd write (rel D0)', (inj[0] - t0) / 1e3)
        if pe: add('H4  last pub_entry -> first irqfd write (GPU + NSI + worker)', (inj[0] - pe[-1]) / 1e3 if inj else float('nan'))
        if gi and inj: add('HN  first host GPU interrupt -> first irqfd write (host RM + worker)', (inj[0] - gi[0]) / 1e3)
        if inj: add('H5  first irqfd write -> first ISR trap (injection + guest vector entry)', (i1 - inj[0]) / 1e3)
        add('    first interrupt-tree trap (rel D0)', (i1 - t0) / 1e3)
        add('H6  first -> last interrupt-tree trap (ISR/DPC chain)', (isr[-1] - i1) / 1e3)
        add('    last interrupt-tree trap (rel D0)', (isr[-1] - t0) / 1e3)
        if cells:
            fc = Ft[cells[0]]; k = bisect.bisect_right(fc, isr[-1]); 
            if k < len(fc) and fc[k] < t1:
                add('H7  last interrupt trap -> completion cell written (DPC)', (fc[k] - isr[-1]) / 1e3)
                add('    completion cell written (rel D0)', (fc[k] - t0) / 1e3)
                add('H8  completion cell -> next D0 (guest sees it, calls Execute, 1st doorbell)', (t1 - fc[k]) / 1e3)
def q(v, f): v = sorted(x for x in v if x == x); return v[min(len(v) - 1, int(f * len(v)))]
print("run %s: %d submits; completion cell = candidate %s of %s (lag after last ISR write)" % (d, len(rows['period D0 -> next D0']), cells[0] if cells else None, sorted(Ft)))
for k, v in rows.items():
    v = [x for x in v if x == x]
    print("%-78s n=%5d med=%9.1f p90=%9.1f max=%9.1f" % (k, len(v), st.median(v), q(v, .9), max(v)))
