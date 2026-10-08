#!/usr/bin/env python3
"""xcorr.py MSI_TIMELINE ETW_DIR GUEST_MINUS_HOST_MS -- cross-correlate ETW QE (by type) with MSIs by source:
count of (QE, MSI) pairs per 0.25 ms lag bin, lag = QE - MSI, in [-15, +15] ms. A peak at a lag L means QE
events follow that MSI source by L (with the guest/host clock offset error folded in)."""
import bisect, collections, datetime, sys
tl, bd, gmh = sys.argv[1], sys.argv[2], float(sys.argv[3])
def sod(s):
    h, m, x = s.split(':'); return int(h) * 3600 + int(m) * 60 + float(x)
src = collections.defaultdict(list)
for line in open(tl):
    t, s = line.split()[:2]
    t = sod(t)
    for k in s.split('+'):
        k = 'DISP' if k.startswith('DISP611800') else k
        src[k].append(t)
    src['ANY'].append(t)
qe = collections.defaultdict(list)
off = -gmh / 1000
for line in open(bd + '/vdr-etw.txt', errors='replace'):
    if not line.startswith('D QE ') and not line.startswith('D VI ') and not line.startswith('D SIG ') and not line.startswith('D ID550 '):
        continue
    p = line.split(None, 6)
    ft = int(p[2][2:])
    dt = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=ft // 10)
    t = dt.hour * 3600 + dt.minute * 60 + dt.second + dt.microsecond / 1e6 + off
    typ = p[1] if p[1] != 'QE' else 'QE_' + p[6].split('"')[1].replace('DXGKETW_', '').replace('_COMMAND_BUFFER', '').strip()
    qe[typ].append(t)
for typ in sorted(qe):
    for s in ('GR0', 'CE2', 'CE3', 'DISP', 'GSP', '-'):
        ms = sorted(src.get(s, []))
        if not ms:
            continue
        h = collections.Counter()
        for t in qe[typ]:
            i = bisect.bisect_left(ms, t - 0.015); j = bisect.bisect_right(ms, t + 0.015)
            for x in ms[i:j]:
                h[int((t - x) * 4000) / 4.0 if t >= x else -int((x - t) * 4000) / 4.0 - 0.25] += 1
        if not h:
            continue
        top = h.most_common(3)
        base = sum(h.values()) / 120.0
        print('%-14s n=%5d vs %-4s (n=%5d): peak lag ms %s  (mean per bin %.1f)' % (
            typ, len(qe[typ]), s, len(ms), ', '.join('%+.2f:%d' % kv for kv in top), base))
