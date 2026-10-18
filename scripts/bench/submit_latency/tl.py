#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""tl.py TRACE [start_line] [n_events]: print a relative-time timeline of an `up`/`up2` bpftrace trace around a doorbell triple."""
import sys
f = sys.argv[1]; start = int(sys.argv[2]) if len(sys.argv) > 2 else 0; n = int(sys.argv[3]) if len(sys.argv) > 3 else 120
names = {}
t0 = None; shown = 0
for i, l in enumerate(open(f)):
    if i < start: continue
    p = l.split()
    if not p or p[0] == '#': continue
    k = p[0]; t = int(p[1])
    if t0 is None: t0 = t
    if k == 'M':
        what = "MMIO %s=%s" % (p[4][-6:], p[5])
        print("%9.1f tid=%s  %s" % ((t - t0) / 1e3, p[2], what))
    elif k == 'U': print("%9.1f tid=%s    U %s %s" % ((t - t0) / 1e3, p[2], p[3], p[4]))
    elif k == 'W': print("%9.1f tid=%s    WAKE %s %s %s" % ((t - t0) / 1e3, p[2], p[3], p[4], p[5]))
    elif k == 'S': print("%9.1f %s SW %s" % ((t - t0) / 1e3, p[2], ' '.join(p[3:])))
    else: print("%9.1f %s" % ((t - t0) / 1e3, ' '.join(p)))
    shown += 1
    if shown >= n: break
