#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""lock_stats.py TRACE (gen_bt.py `lock` profile): per-submit time of the two kf3 workers inside RelayLoop::peek, user_ring_record,
serve and std's Mutex::lock_contended, by caller (return address resolved with nm+maps), plus per-call duration quantiles.
Iteration windows = doorbell triples (BAR0 0xbb0090), D0 -> next D0; only windows between 700 us and 1.5 ms are kept (steady loop)."""
import sys, bisect, statistics as st, subprocess, collections
f = sys.argv[1]; nmf = sys.argv[2]; base = int(sys.argv[3], 16)
syms = sorted((int(l.split()[0], 16), l.rstrip("\n").split(" ", 2)[2]) for l in open(nmf) if len(l.split()) >= 3 and l.split()[1] in "tT")
addrs = [s[0] for s in syms]
def res(r):
    o = r - base; i = bisect.bisect_right(addrs, o) - 1
    n = syms[i][1]; n = n.replace('<kf_qemu::chan::ChanPlane as kf_chan::worker::RelayLoop>', 'RelayLoop').replace('<kf_qemu::chan::ChanPlane>', 'ChanPlane')
    return "%s+%#x" % (n[:60], o - syms[i][0])
M = []; Ev = collections.defaultdict(list)    # (kind,label) -> [(t,tid,phase,extra)]
for l in open(f):
    p = l.split()
    if not p or p[0] == '#': continue
    if p[0] == 'M' and p[3] == '2': M.append((int(p[1]), p[4][-6:]))
    elif p[0] == 'U': Ev[p[3]].append((int(p[1]), int(p[2]), p[4]))
    elif p[0] == 'C': Ev['lock'].append((int(p[1]), int(p[2]), p[3], p[4] if len(p) > 4 else ''))
D0 = []; last = -10**12
for t, o in M:
    if o == 'bb0090':
        if t - last > 300_000: D0.append(t)
        last = t
def pairs(evs):
    open_ = {}; out = []
    for e in evs:
        t, tid, ph = e[0], e[1], e[2]
        if ph == 'E': open_[tid] = e
        elif ph == 'R' and tid in open_:
            s = open_.pop(tid); out.append((s[0], t - s[0], tid, s[3] if len(s) > 3 else ''))
    return out
tot = {}
for lab in ('peek', 'waiting', 'user_ring_record', 'serve'):
    tot[lab] = pairs(Ev[lab])
lk = pairs([e for e in Ev['lock'] if e[2] in ('E', 'R')])
tot['lock_contended'] = lk
def q(v, f): v = sorted(v); return v[min(len(v) - 1, int(f * len(v)))]
print("per-call durations (us; includes uprobe cost ~1 us per probe pair):")
for k, v in tot.items():
    d = [x[1] / 1e3 for x in v]
    if d: print("  %-18s n=%7d med=%7.2f p90=%7.2f p99=%8.2f max=%9.2f" % (k, len(d), st.median(d), q(d, .9), q(d, .99), max(d)))
print("lock_contended by caller:")
byc = collections.defaultdict(list)
for t, dur, tid, extra in lk:
    r = extra.split('=')[1] if '=' in extra else '0'
    byc[res(int(r, 16)) if r != '0' else '?'].append(dur / 1e3)
for k, v in sorted(byc.items(), key=lambda kv: -sum(kv[1])):
    print("  %-70s n=%6d med=%7.2f p90=%7.2f sum_ms=%8.1f" % (k, len(v), st.median(v), q(v, .9), sum(v) / 1e3))
# per-submit sums within windows
wins = [(D0[i], D0[i + 1]) for i in range(len(D0) - 1) if 700_000 < D0[i + 1] - D0[i] < 1_600_000]
print("steady windows:", len(wins))
for lab, v in tot.items():
    ts = [x[0] for x in v]
    s = []
    for a, b in wins:
        i = bisect.bisect_left(ts, a); j = bisect.bisect_left(ts, b)
        s.append(sum(x[1] for x in v[i:j]) / 1e3)
    n = [bisect.bisect_left(ts, b) - bisect.bisect_left(ts, a) for a, b in wins]
    print("  per submit %-18s calls med=%6.1f   summed time over both workers (us) med=%7.1f p90=%7.1f" % (lab, st.median(n), st.median(s), q(s, .9)))
print("  per-submit window length med=%.1f us" % st.median([(b - a) / 1e3 for a, b in wins]))
