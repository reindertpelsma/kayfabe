#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""cuda_during.py RUNDIR: host-native CUDA GR launch+sync latency (cuda_lat timeline file cl_gr.txt) inside vs outside the window of the
guest's A2 loop (doorbell triples of the bpftrace trace). Same clock (CLOCK_MONOTONIC ns)."""
import sys, statistics as st
d = sys.argv[1]
D = []
for l in open(d + "/trace.txt"):
    p = l.split()
    if p and p[0] == "M" and p[3] == "2" and p[4].endswith("bb0090"): D.append(int(p[1]))
gaps = [(D[i + 1] - D[i]) for i in range(len(D) - 1)]
runs = []; s = None
for i, g in enumerate(gaps):
    ok = g < 1500000
    if ok and s is None: s = i
    if (not ok) and s is not None: runs.append((s, i)); s = None
if s is not None: runs.append((s, len(D) - 1))
runs = [r for r in runs if r[1] - r[0] > 300]
lo = D[runs[0][0]]; hi = D[runs[-1][1]]
print("VM A2 loop window: %.2f s" % ((hi - lo) / 1e9))
S = [tuple(map(float, l.split())) for l in open(d + "/" + (sys.argv[2] if len(sys.argv) > 2 else "cl_gr.txt"))]
print("cuda samples: %d over %.1f s" % (len(S), (S[-1][0] - S[0][0]) / 1e9))
def q(v, f): v = sorted(v); return v[int(f * len(v))]
inn = [l for t, l in S if lo <= t <= hi]; out = [l for t, l in S if t < lo - 2e9 or t > hi + 2e9]
for n, v in (("CUDA launch+sync during the VM A2 loop", inn), ("outside the loop", out)):
    if v: print("%-40s n=%d med=%.1f p90=%.1f p99=%.1f max=%.1f us" % (n, len(v), st.median(v), q(v, .9), q(v, .99), max(v)))
