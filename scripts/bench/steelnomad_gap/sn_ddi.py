#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sn_ddi.py DIR TAG: Profiler keyword (0x2) events 105 (function enter) / 106 (exit), field Function=<id>, paired per thread (stack) ->
per Function id: calls/s, duration median/p95/p99/max in us, total ms per second. Run on a TAG captured with keywords 0x3 (sn_run_capture_late.sh).
Window: the capture's middle (drop the first/last 3 s)."""
import pickle, sys, collections, json
d, tag = sys.argv[1], sys.argv[2]
ev, _ = pickle.load(open(f'{d}/{tag}/{tag}_ev.pkl', 'rb'))
E = sorted([(e[0], 1, e[2], e[4]) for e in ev.get('105', [])] + [(e[0], 0, e[2], e[4]) for e in ev.get('106', [])])
t0, t1 = E[0][0] + 3e7, E[-1][0] - 3e7; W = (t1 - t0) / 1e7
stk = collections.defaultdict(list); dur = collections.defaultdict(list); depth0 = collections.defaultdict(list)
for t, isenter, tid, fn in E:
    if isenter: stk[tid].append((t, fn))
    elif stk[tid]:
        ts, f = stk[tid].pop()
        if t0 <= ts <= t1: dur[f].append((t - ts) / 10.0)   # us; includes nested calls
def pct(a, p): a = sorted(a); return a[min(len(a) - 1, int(len(a) * p))]
out = {}
for f, a in sorted(dur.items(), key=lambda kv: -sum(kv[1])):
    out[f] = {'calls_per_s': round(len(a) / W, 1), 'med_us': round(pct(a, .5), 2), 'p95_us': round(pct(a, .95), 2), 'p99_us': round(pct(a, .99), 2), 'max_us': round(max(a), 1), 'total_ms_per_s': round(sum(a) / 1e3 / W, 2)}
json.dump({'window_s': W, 'functions': out}, open(f'{d}/{tag}/{tag}_ddi.json', 'w'), indent=1)
print('window s', round(W, 1))
for f, v in list(out.items())[:14]: print(f, v)
