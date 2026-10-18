#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""iter_stats.py TRACE: split a bpftrace trace (gen_bt.py up/up2) into submit iterations and print per-hop median/p90/max.
Iteration start D0 = a doorbell write (BAR0 0xbb0090, NV_USERMODE_NOTIFY_CHANNEL_PENDING) with no doorbell in the 300 us before it.
All times in us relative to D0 (one clock: bpftrace nsecs)."""
import sys, statistics as st, bisect
DB = 'c0bb0090'
ev = []
for l in open(sys.argv[1]):
    p = l.split()
    if not p or p[0] == '#': continue
    ev.append(p)
M = [(int(p[1]), int(p[2]), p[4], p[5]) for p in ev if p[0] == 'M' and p[3] == '2']
dbs = [m for m in M if m[2] == DB]
starts = []
last = -10**12
for m in dbs:
    if m[0] - last > 300_000: starts.append(m[0])
    last = m[0]
U = {}
for p in ev:
    if p[0] == 'U': U.setdefault((p[3], p[4]), []).append((int(p[1]), int(p[2])))
W = [(int(p[1]), p[3]) for p in ev if p[0] == 'W']
irqw = [m for m in M if m[2][-6:-3] == 'b81' ]       # interrupt-tree traps (ISR/DPC mask/unmask)
def q(v, f):
    v = sorted(v); return v[min(len(v) - 1, int(f * len(v)))]
rows = {}
def add(k, x): rows.setdefault(k, []).append(x)
for i, t0 in enumerate(starts[:-1]):
    t1 = starts[i + 1]
    if t1 - t0 > 3_000_000: continue
    add('period_us', (t1 - t0) / 1e3)
    d = [m for m in dbs if t0 <= m[0] < t0 + 300_000]
    add('n_doorbell', len(d)); add('doorbell_span_us', (d[-1][0] - d[0][0]) / 1e3)
    w = [x for x in W if t0 <= x[0] < t1 and x[1].startswith('kf3-worker')]
    if w: add('first_worker_wake_us', (w[0][0] - t0) / 1e3)
    for lab in ('serve', 'turn', 'pub_entry', 'pub_marker', 'gp_put', 'wr_guest_get', 'event', 'read_pb', 'stage'):
        e = [x for x in U.get((lab, 'E'), []) if t0 <= x[0] < t1]
        add('n_' + lab, len(e))
        if e and lab in ('serve', 'pub_entry'): add('first_%s_us' % lab, (e[0][0] - t0) / 1e3)
    r = [x for x in U.get(('pub_entry', 'R'), []) if t0 <= x[0] < t1]
    if r: add('last_pub_entry_end_us', (r[-1][0] - t0) / 1e3)
    mk = [x for x in U.get(('pub_marker', 'R'), []) if t0 <= x[0] < t1]
    if mk: add('last_pub_marker_end_us', (mk[-1][0] - t0) / 1e3)
    sr = [x for x in U.get(('serve', 'R'), []) if t0 <= x[0] < t1]
    if sr: add('last_serve_end_us', (sr[-1][0] - t0) / 1e3)
    ir = [m for m in irqw if t0 + 100_000 <= m[0] < t1]       # after the doorbell triple
    add('n_irq_tree_writes', len([m for m in irqw if t0 <= m[0] < t1]))
    if ir:
        add('first_irq_write_us', (ir[0][0] - t0) / 1e3); add('last_irq_write_us', (ir[-1][0] - t0) / 1e3)
    allm = [m for m in M if t0 <= m[0] < t1]
    add('n_trapped_writes', len(allm))
for k, v in rows.items():
    print("%-26s n=%5d med=%9.1f p90=%9.1f max=%9.1f" % (k, len(v), st.median(v), q(v, .9), max(v)))
