#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sn_analyze.py DIR TAG [PM_CSV]: per-capture metrics from DIR/TAG_ev.pkl (sn_extract.py) and PresentMon's CSV -> DIR/TAG_metrics.json.
stdlib only. Event semantics (guest DxgKrnl manifest; HAGS is ON, so there are no DmaPackets for render work, only QueuePackets + HwQueue events):
  178 QueuePacket start = the UMD's submit reaches dxgkrnl (thread of the app)       T_sub
  450 HwQueue submit   = VidSch writes the packet to the hardware queue (mostly in a DPC, pid 0) T_hw
  451 HwQueue progress = the progress fence of that packet is seen (completion interrupt/DPC)   T_done
  180 QueuePacket stop = packet retired by dxgkrnl                                            T_ret
A packet is joined across the four events by (hContext == hHwQueue, ProgressFenceValue) and (hContext, SubmitSequence).
The graphics-test window = the largest run of target presents whose frame holds >=20 packets on the heavy queues (>=3000 render packets), minus 3 s at each end.
HONEST LIMIT: the guest sees no GPU-side timestamps. T_hw..T_done is 'outstanding on the hardware queue' (it includes queueing behind
earlier packets, cross-queue fence waits and the completion-notification latency); `service` = T_done - max(T_hw, previous T_done of the
same queue) is the time attributable to that packet if a queue executes in order."""
import pickle, sys, json, csv, bisect, collections, statistics as S

d, tag = sys.argv[1], sys.argv[2]
pm_path = sys.argv[3] if len(sys.argv) > 3 else f'{d}/{tag}_pm.csv'
TRIM = 3.0   # s
ev, nrows = pickle.load(open(f'{d}/{tag}_ev.pkl', 'rb'))
TICK = 1e7
def pct(a, p):
    if not a: return None
    a = sorted(a); k = (len(a) - 1) * p; f = int(k); c = min(f + 1, len(a) - 1)
    return a[f] + (a[c] - a[f]) * (k - f)
def stats(a, scale=1.0):
    if not a: return None
    a = [x * scale for x in a]
    return {'n': len(a), 'mean': round(S.fmean(a), 4), 'med': round(pct(a, .5), 4), 'p95': round(pct(a, .95), 4), 'p99': round(pct(a, .99), 4), 'max': round(max(a), 4)}

# --- target process: PresentMon's 3DMarkSteelNomad.exe
pm = list(csv.DictReader(open(pm_path, encoding='utf-8-sig')))
cnt = collections.Counter((r['Application'], int(r['ProcessID'])) for r in pm if 'SteelNomad' in r['Application'] or 'Steel' in r['Application'])
PID = cnt.most_common(1)[0][0][1]
# --- index events
E178 = [e for e in ev['178']]
E180 = ev['180']; E450 = ev['450']; E451 = ev['451']
sub = {}                       # (hctx, seq) -> 178
for e in E178: sub[(e[4], e[6])] = e
ret = {(e[4], e[6]): e for e in E180}
hw = {(e[4], e[5]): e for e in E450}
dn = {(e[4], e[5]): e for e in E451}
# heavy contexts of the target: >= 3000 render packets, in order of first appearance
cc = collections.Counter(); first = {}
for e in E178:
    if e[1] == PID and 'RENDER' in e[5]:
        cc[e[4]] += 1; first.setdefault(e[4], e[0])
heavy = sorted([c for c, n in cc.items() if n >= 3000], key=lambda c: first[c])
hset = set(heavy)
hp = [e for e in E178 if e[1] == PID and e[4] in hset]
# test frames: the target's presents whose frame interval holds >= 20 packets on the heavy queues (loading / intro frames hold fewer)
allp = sorted(e[0] for e in E178 if e[1] == PID and e[10] == 'true' and 'SOFTWARE' in e[5])
hpt = sorted(e[0] for e in hp)
npk = [bisect.bisect_left(hpt, b) - bisect.bisect_left(hpt, a) for a, b in zip(allp, allp[1:])]
good = [i for i, n in enumerate(npk) if n >= 20]
# largest contiguous run of good frames
runs = []; st = None
for i in range(len(npk) + 1):
    ok = i < len(npk) and npk[i] >= 20
    if ok and st is None: st = i
    if not ok and st is not None: runs.append((st, i)); st = None
a_i, b_i = max(runs, key=lambda r: allp[r[1]] - allp[r[0]])
t_lo = allp[a_i] + TRIM * TICK; t_hi = allp[b_i] - TRIM * TICK
W = (t_hi - t_lo) / TICK
inw = lambda t: t_lo <= t <= t_hi
out = {'tag': tag, 'pid': PID, 'window_s': round(W, 3), 'rows': nrows, 'heavy_queues': len(heavy)}
out['window_filetime_100ns'] = [int(t_lo), int(t_hi)]

# --- frames: SOFTWARE packets with bPresent on the target
pres = sorted(e[0] for e in E178 if e[1] == PID and e[10] == 'true' and 'SOFTWARE' in e[5] and inw(e[0]))
pi = [(b - a) / 1e4 for a, b in zip(pres, pres[1:])]
out['frames'] = len(pres); out['fps'] = round(len(pi) / (sum(pi) / 1e3), 3)
out['frame_ms'] = stats(pi)
out['frame_ms']['stdev'] = round(S.pstdev(pi), 3)
# --- PresentMon (target, steady part)
pmr = [r for r in pm if r['Application'] == next(a for (a, p) in cnt if p == PID) and r['PresentMode'].startswith('Hardware')]
# keep rows whose QPC-based time lies inside the window: map PM TimeInQPC via the ETW present series (same order): use the middle part
if pmr:
    n0 = max(0, int(len(pmr) * 0.0)); rows = pmr
    # steady part: rows whose MsBetweenPresents < 100 ms and trim the same TRIM seconds from both ends
    t = [int(r['TimeInQPC']) for r in rows]
    lo = t[0] + TRIM * 1e7 + 20 * 1e7; hi = t[-1] - TRIM * 1e7   # skip the ~20 s of load/intro frames at the start of the Independent-Flip span
    sel = [r for r in rows if lo <= int(r['TimeInQPC']) <= hi]
    def col(n):
        a = []
        for r in sel:
            try: a.append(float(r[n]))
            except (ValueError, KeyError): pass
        return a
    out['pm'] = {'rows': len(sel)}
    for n in ('MsBetweenPresents', 'MsInPresentAPI', 'MsCPUBusy', 'MsCPUWait', 'MsGPULatency', 'MsGPUTime', 'MsGPUBusy', 'MsGPUWait', 'MsUntilDisplayed', 'MsBetweenDisplayChange'):
        a = col(n)
        if a: out['pm'][n] = stats(a)

# --- per-queue packet stats (target heavy queues + the rest of the target + all other processes)
def qstats(ctxs, name, only_pid=None):
    sw = []; hwd = []; svc = []; rt = []; life = []; n = 0
    byq = collections.defaultdict(list)
    for (hc, seq), e in sub.items():
        if hc not in ctxs or not inw(e[0]) or 'RENDER' not in e[5]: continue
        F = e[13]; h = hw.get((hc, F)); dnn = dn.get((hc, F)); r = ret.get((hc, seq))
        if not (h and dnn and r): continue
        n += 1; byq[hc].append((h[0], dnn[0], e[0], r[0]))
    for hc, L in byq.items():
        L.sort(); prev = None
        for (th, td, ts, tr) in L:
            sw.append((th - ts) / 1e4); hwd.append((td - th) / 1e4); rt.append((tr - td) / 1e4); life.append((tr - ts) / 1e4)
            svc.append((td - (max(th, prev) if prev is not None else th)) / 1e4)
            prev = td
    return {'queues': len(ctxs), 'packets': n, 'packets_per_s': round(n / W, 2), 'sw_queue_178_450_ms': stats(sw), 'hw_outstanding_450_451_ms': stats(hwd),
            'service_ms': stats(svc), 'retire_451_180_ms': stats(rt), 'lifetime_178_180_ms': stats(life), 'svc_sum_ms_per_s': round(sum(svc) / W, 2)}
out['queues'] = {}
for i, c in enumerate(heavy): out['queues'][f'Q{i}'] = qstats({c}, f'Q{i}')
others = {e[4] for e in E178 if e[1] == PID and 'RENDER' in e[5] and e[4] not in hset}
out['queues']['target_other'] = qstats(others, 'other')
allctx = {e[4] for e in E178 if 'RENDER' in e[5]}
out['queues']['all_other_processes'] = qstats({c for c in allctx if c not in hset and c not in others}, 'rest')

# --- hardware-queue outstanding union (all processes): intervals [T_hw, T_done]; service union with in-order adjust
iv_out = []; iv_svc = []; byq = collections.defaultdict(list)
for k, h in hw.items():
    dnn = dn.get(k)
    if dnn: byq[k[0]].append((h[0], dnn[0], k[1]))
nxt_info = {}   # T_hw -> (hctx, fence) for gap attribution
for hc, L in byq.items():
    L.sort(); prev = None
    for th, td, F in L:
        iv_out.append((th, td)); iv_svc.append((max(th, prev) if prev is not None else th, td)); prev = td
def union(iv):
    iv = sorted(iv); res = []
    for a, b in iv:
        if res and a <= res[-1][1]: res[-1][1] = max(res[-1][1], b)
        else: res.append([a, b])
    return res
def clip(res):
    return [[max(a, t_lo), min(b, t_hi)] for a, b in res if b > t_lo and a < t_hi]
U = clip(union(iv_out)); US = clip(union(iv_svc))
busy = sum(b - a for a, b in U) / TICK; busys = sum(b - a for a, b in US) / TICK
out['hw_outstanding_union_frac'] = round(busy / W, 4); out['hw_service_union_frac'] = round(busys / W, 4)
gaps = [(U[i][1], U[i + 1][0]) for i in range(len(U) - 1)]
gd = [(b - a) / 1e4 for a, b in gaps]
out['idle_gaps'] = {'count': len(gd), 'per_s': round(len(gd) / W, 2), 'total_frac': round(sum(gd) / 1e3 / W, 4), 'dur_ms': stats(gd),
                    'hist_ms': {lab: round(sum(1 for x in gd if lo <= x < hi), 0) for lab, lo, hi in (('<0.1', 0, .1), ('0.1-0.5', .1, .5), ('0.5-1', .5, 1), ('1-2', 1, 2), ('2-5', 2, 5), ('5-10', 5, 10), ('>=10', 10, 1e9))},
                    'time_by_bucket_ms_per_frame': {lab: round(sum(x for x in gd if lo <= x < hi) / max(1, len(pi)), 3) for lab, lo, hi in (('<0.1', 0, .1), ('0.1-0.5', .1, .5), ('0.5-1', .5, 1), ('1-2', 1, 2), ('2-5', 2, 5), ('5-10', 5, 10), ('>=10', 10, 1e9))}}
# gap attribution: the packet whose T_hw ends the gap
hw_sorted = sorted((h[0], k) for k, h in hw.items())
hw_t = [x[0] for x in hw_sorted]
t300 = sorted(e[0] for e in ev.get('300', []))
pre = []; post = []; qd_at_end = []; cpu_wake = []; pkt_kind = collections.Counter(); sub_before = 0
for a, b in gaps:
    i = bisect.bisect_left(hw_t, b)
    if i >= len(hw_t) or abs(hw_t[i] - b) > 1: continue
    k = hw_sorted[i][1]; s = None
    for (hc, seq), e in ():  # placeholder (kept out of the hot path)
        pass
    # find the 178 of this packet: (hctx, fence) match
    s178 = SUB_BY_FENCE.get(k) if 'SUB_BY_FENCE' in globals() else None
    if s178 is None:
        SUB_BY_FENCE = {(e[4], e[13]): e for e in E178 if 'RENDER' in e[5]}
        s178 = SUB_BY_FENCE.get(k)
    if s178 is None: continue
    ts = s178[0]
    g = (b - a) / 1e4
    if ts <= a: pre.append(0.0); post.append(g); sub_before += 1
    else: p1 = (min(ts, b) - a) / 1e4; pre.append(p1); post.append(g - p1)
    pkt_kind[('target' if s178[1] == PID else 'other', 'q' + str(heavy.index(k[0])) if k[0] in hset else 'x')] += 1
    j = bisect.bisect_right(t300, a)
    if j < len(t300) and t300[j] < b: cpu_wake.append((t300[j] - a) / 1e4)
out['gap_attribution'] = {'gaps_attributed': len(pre), 'next_packet_already_submitted_frac': round(sub_before / max(1, len(pre)), 3),
                          'pre_ms_per_frame': round(sum(pre) / max(1, len(pi)), 3), 'post_ms_per_frame': round(sum(post) / max(1, len(pi)), 3),
                          'pre_ms': stats([x for x in pre]), 'post_ms': stats(post), 'cpu_unwait_after_gap_start_ms': stats(cpu_wake), 'gap_end_packet_kind': {f'{a}/{b}': n for (a, b), n in pkt_kind.most_common(8)}}

# --- queue depth (time-weighted), hw outstanding (all queues), and software-queued (178..450) of the target heavy queues
def depth_hist(iv, cap=6):
    pts = []
    for a, b in iv:
        a2 = max(a, t_lo); b2 = min(b, t_hi)
        if b2 > a2: pts.append((a2, 1)); pts.append((b2, -1))
    pts.sort(); cur = 0; last = t_lo; h = collections.Counter()
    for t, dlt in pts:
        h[min(cur, cap)] += t - last; last = t; cur += dlt
    h[min(cur, cap)] += t_hi - last
    tot = sum(h.values()); return {str(k): round(v / tot, 4) for k, v in sorted(h.items())}, sum(k * v for k, v in h.items()) / tot
out['hw_depth_all_queues_frac'], out['hw_depth_all_mean'] = depth_hist(iv_out); out['hw_depth_all_mean'] = round(out['hw_depth_all_mean'], 3)
swiv = []
for (hc, seq), e in sub.items():
    if hc in hset and e[1] == PID and 'RENDER' in e[5]:
        h = hw.get((hc, e[13]))
        if h: swiv.append((e[0], h[0]))
out['sw_queued_depth_frac'], out['sw_queued_mean'] = depth_hist(swiv, 12); out['sw_queued_mean'] = round(out['sw_queued_mean'], 3)
# frames in flight: present packet 178 (SOFTWARE, bPresent) until its 180
fl = []
for e in E178:
    if e[1] == PID and e[10] == 'true' and 'SOFTWARE' in e[5]:
        r = ret.get((e[4], e[6]))
        if r: fl.append((e[0], r[0]))
out['frames_in_flight_frac'], out['frames_in_flight_mean'] = depth_hist(fl, 6); out['frames_in_flight_mean'] = round(out['frames_in_flight_mean'], 3)
out['present_pkt_lifetime_ms'] = stats([(b - a) / 1e4 for a, b in fl if inw(a)])


# --- per-queue starvation (a queue with nothing outstanding) and the software dependency path (wait -> unwait -> HW submit)
t238 = collections.defaultdict(list)
for e in ev.get('238', []): t238[e[5]].append(e[0])       # hContext -> unwait times
for v in t238.values(): v.sort()
hw_by_q = collections.defaultdict(list)
for (hc, F), h in hw.items(): hw_by_q[hc].append(h[0])
for v in hw_by_q.values(): v.sort()
done_all = sorted([e[0] for e in E451] + [e[0] for e in ev.get('552', [])])
qgap = {}
for qi, c in enumerate(heavy):
    iv = clip(union([(hw[(c, F)][0], dn[(c, F)][0]) for (hc, F) in hw if hc == c and (hc, F) in dn]))
    g = [(iv[i][1], iv[i + 1][0]) for i in range(len(iv) - 1)]
    gdur = [(b - a) / 1e4 for a, b in g]
    pre_u = []; post_u = []; l_chain = []
    for a, b in g:
        L = t238.get(c, []); j = bisect.bisect_right(L, b) - 1
        if j >= 0 and L[j] >= a - 50000:   # an unwait for this queue near/inside the gap
            tu = L[j]; pre_u.append(max(0, tu - a) / 1e4); post_u.append((b - max(tu, a)) / 1e4)
    qgap[f'Q{qi}'] = {'queue_empty_frac': round(sum(gdur) / 1e3 / W, 4), 'gaps_per_s': round(len(gdur) / W, 2), 'gap_ms': stats(gdur),
                      'gaps_with_unwait': len(pre_u), 'gap_to_unwait_ms': stats(pre_u), 'unwait_to_hwsubmit_ms': stats(post_u)}
out['queue_starvation'] = qgap
# unwait path latencies (target's heavy queues): completion seen (latest 451/552 before) -> 238 -> next 450 on that queue
l1 = []; l2 = []
for c in heavy:
    for tu in t238.get(c, []):
        if not inw(tu): continue
        j = bisect.bisect_right(done_all, tu) - 1
        if j >= 0: l1.append((tu - done_all[j]) / 1e4)
        L = hw_by_q.get(c, []); k = bisect.bisect_left(L, tu)
        if k < len(L): l2.append((L[k] - tu) / 1e4)
out['unwait_path'] = {'unwaits_per_frame': round(sum(1 for c in heavy for tu in t238.get(c, []) if inw(tu)) / max(1, len(pi)), 2),
                      'done_to_unwait_ms': stats(l1), 'unwait_to_hwsubmit_ms': stats(l2)}

# --- per-frame budget: the frame = [present_i, present_i+1)
def overlap(res, a, b):
    i = bisect.bisect_left([r[1] for r in res], a); s = 0
    while i < len(res) and res[i][0] < b:
        s += min(res[i][1], b) - max(res[i][0], a); i += 1
    return s
fb = []; fi = []; svs = []
Ub = U
svc_by_frame = collections.defaultdict(float)
for a, b in zip(pres, pres[1:]):
    ft = (b - a) / 1e4
    bo = overlap(U, a, b) / 1e4; bs = overlap(US, a, b) / 1e4
    fb.append((ft, bo, bs))
out['per_frame_ms'] = {'frame': round(S.fmean(x[0] for x in fb), 3), 'hw_outstanding_union': round(S.fmean(x[1] for x in fb), 3), 'hw_service_union': round(S.fmean(x[2] for x in fb), 3),
                       'idle_no_hw_work': round(S.fmean(x[0] - x[1] for x in fb), 3)}
# packets per frame (target heavy queues)
out['packets_per_frame_target_heavy'] = round(sum(1 for e in hp if inw(e[0])) / max(1, len(pi)), 2)
json.dump(out, open(f'{d}/{tag}_metrics.json', 'w'), indent=1)
print(json.dumps(out, indent=1)[:6000])
