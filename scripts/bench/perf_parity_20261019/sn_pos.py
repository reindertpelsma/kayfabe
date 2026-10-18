#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sn_pos.py DIR TAG [...]: per-position view of the frame on the heavy queues. Frame = interval between the target's present packets (178 SOFTWARE bPresent);
for queue Q0, the n-th packet of each frame: mean/median completion spacing (T_done - previous T_done on the queue) and DmaBufferSize.
Prints the table; with two tag lists separated by '--' also the per-position difference (kayfabe minus vfio)."""
import pickle, sys, bisect, collections, statistics as S, json
TICK = 1e7
def load(d, tag):
    ev, _ = pickle.load(open(f'{d}/{tag}/{tag}_ev.pkl', 'rb')); m = json.load(open(f'{d}/{tag}/{tag}_metrics.json')); PID = m['pid']
    sub = {(e[4], e[6]): e for e in ev['178']}
    dn = {(e[4], e[5]): e[0] for e in ev['451']}
    cc = collections.Counter(); first = {}
    for e in ev['178']:
        if e[1] == PID and 'RENDER' in e[5]: cc[e[4]] += 1; first.setdefault(e[4], e[0])
    heavy = sorted([c for c, n in cc.items() if n >= 3000], key=lambda c: first[c])
    pres = sorted(e[0] for e in ev['178'] if e[1] == PID and e[10] == 'true' and 'SOFTWARE' in e[5])
    # window as the analyzer: take the middle of the capture's steady frames via metrics window_s is not stored as absolute: use frames with >= 20 heavy packets
    c = heavy[0]
    pk = sorted((e[0], e[13], int(e[7])) for (hc, s), e in sub.items() if hc == c and e[1] == PID)
    ts = [p[0] for p in pk]
    done = [dn.get((c, p[1])) for p in pk]
    frames = []
    for a, b in zip(pres, pres[1:]):
        i = bisect.bisect_left(ts, a); j = bisect.bisect_left(ts, b)
        if j - i >= 8 and all(done[k] for k in range(max(0, i - 1), j)): frames.append((i, j))
    # steady: drop the first/last 3 s worth (≈ the same trimming): keep the middle frames between the 10% and 95% marks of the heavy-packet frames
    n = len(frames); frames = frames[int(n * 0.08):int(n * 0.95)]
    by = collections.defaultdict(list); sizes = collections.defaultdict(list); cnt = collections.Counter()
    for i, j in frames:
        cnt[j - i] += 1
        for k in range(i, j):
            by[k - i].append((done[k] - done[k - 1]) / 1e4); sizes[k - i].append(pk[k][2])
    return by, sizes, cnt, len(frames)
def table(by, sizes, nf, cnt):
    top = max(cnt, key=cnt.get)
    return {k: (S.fmean(by[k]), S.median(by[k]), S.fmean(sizes[k])) for k in sorted(by) if k < top + 1}, top
args = sys.argv[1:]
d = args[0]; sep = args.index('--') if '--' in args else len(args)
sets = [args[1:sep], args[sep + 1:]] if sep < len(args) else [args[1:]]
res = []
for tags in sets:
    acc = collections.defaultdict(list); sz = collections.defaultdict(list); tops = []
    for t in tags:
        by, sizes, cnt, nf = load(d, t); tb, top = table(by, sizes, nf, cnt); tops.append((top, cnt.most_common(3), nf))
        for k, (m, md, s) in tb.items(): acc[k].append(m); sz[k].append(s)
    res.append({k: (S.fmean(v), S.fmean(sz[k])) for k, v in acc.items()}); print('packets-per-frame mode / top3 / frames:', tops)
ks = sorted(set().union(*[r.keys() for r in res]))
print('pos | ' + ' | '.join(f'set{i}: mean spacing ms, mean DmaBufSize' for i in range(len(res))) + ' | diff set0-set1 ms')
tot = [0, 0]
for k in ks:
    row = [f'{k:2d}']
    for i, r in enumerate(res):
        if k in r: row.append(f'{r[k][0]:7.3f} {r[k][1]:9.0f}'); tot[i] += r[k][0]
        else: row.append('   -')
    if len(res) == 2 and k in res[0] and k in res[1]: row.append(f'{res[0][k][0] - res[1][k][0]:+7.3f}')
    print(' | '.join(row))
print('sum of the position means (ms):', [round(x, 2) for x in tot])
