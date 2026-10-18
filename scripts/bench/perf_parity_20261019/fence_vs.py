#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""fence_vs.py RUNDIR: for each candidate fence word found in guest RAM, when does it change relative to the doorbells (BAR0 0xbb0090)
and the interrupt-tree traps (BAR0 0xb81xxx) of the same run?  Host clock only (CLOCK_MONOTONIC), no guest-clock alignment."""
import sys, bisect, collections, statistics as st
d = sys.argv[1]
F = collections.defaultdict(list)
for l in open(d + '/fence.txt'):
    p = l.split()
    if len(p) == 4 and p[0] == 'F': F[int(p[2])].append(int(p[1]))
DB = []; I = []
for l in open(d + '/trace.txt'):
    p = l.split()
    if p and p[0] == 'M' and p[3] == '2':
        o = p[4][-6:]; t = int(p[1])
        if o == 'bb0090': DB.append(t)
        elif o.startswith('b81'): I.append(t)
def qs(v): v = sorted(v); return tuple(v[int(len(v) * x)] for x in (.05, .25, .5, .75, .95))
for c, ts in sorted(F.items()):
    ts = ts[20:-20]
    nd = [(DB[bisect.bisect_right(DB, f)] - f) / 1e3 for f in ts if bisect.bisect_right(DB, f) < len(DB)]
    pd = [(f - DB[bisect.bisect_right(DB, f) - 1]) / 1e3 for f in ts]
    pi = [(f - I[bisect.bisect_right(I, f) - 1]) / 1e3 for f in ts]
    ni = [(I[bisect.bisect_right(I, f)] - f) / 1e3 for f in ts if bisect.bisect_right(I, f) < len(I)]
    print("cand %d: n=%d  changes/iter-ish; (us) next doorbell after: p5/25/50/75/95 = %.1f %.1f %.1f %.1f %.1f | prev doorbell before: %.1f %.1f %.1f %.1f %.1f | prev ISR-write before: %.1f %.1f %.1f %.1f %.1f | next ISR-write after: %.1f %.1f %.1f %.1f %.1f" % ((c, len(ts)) + qs(nd) + qs(pd) + qs(pi) + qs(ni)))
