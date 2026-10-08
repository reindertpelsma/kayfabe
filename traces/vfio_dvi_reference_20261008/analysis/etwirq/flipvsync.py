#!/usr/bin/env python3
"""flipvsync.py BOOTDIR GUEST_MINUS_HOST_MS -- for each MMIOFLIP queue packet: QI time, QE time, the next VSync
interrupt (ETW VI) and the next display MSI (0x611800 W1C), and the LAST_DATA enable writes (0x611d80 bit1)
around it. Real HW reference for the flip -> vsync path."""
import bisect, datetime, re, sys
bd, gmh = sys.argv[1], float(sys.argv[2])
off = -gmh / 1000
def ft2(ft):
    dt = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=ft // 10)
    return dt.hour * 3600 + dt.minute * 60 + dt.second + dt.microsecond / 1e6 + off
qi, qe, vi = {}, {}, []
for line in open(bd + '/vdr-etw.txt', errors='replace'):
    if line.startswith('D VI '):
        vi.append(ft2(int(line.split()[2][2:])))
    elif 'MMIOFLIP' in line and (line.startswith('D QI ') or line.startswith('D QE ')):
        p = line.split(None, 6)
        seq = p[6].split(',')[2].strip()
        key = (p[6].split(',')[0], seq)
        (qi if p[1] == 'QI' else qe)[key] = ft2(int(p[2][2:]))
W = re.compile(r'T(\d\d):(\d\d):(\d\d\.\d+)Z vfio_region_write\s+\(0000:01:00.0:region0\+(0x611800|0x611d80), (0x[0-9a-f]+)')
disp, en = [], []
for line in open(bd + '/trace.log', errors='replace'):
    m = W.search(line)
    if m:
        t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3))
        (disp if m.group(4) == '0x611800' else en).append((t, int(m.group(5), 16)))
vi.sort(); dt_ = [t for t, _ in disp]; et = [t for t, _ in en]
def fmt(t):
    return '%02d:%02d:%09.6f' % (t // 3600, t % 3600 // 60, t % 60)
gaps = []
for k in sorted(qe, key=qe.get):
    t = qe[k]
    i = bisect.bisect_left(vi, t); j = bisect.bisect_left(dt_, t)
    nv = vi[i] - t if i < len(vi) else None
    nd = dt_[j] - t if j < len(dt_) else None
    e0 = bisect.bisect_left(et, qi.get(k, t) - 0.001)
    ens = [('%+.1fms=%x' % ((x - t) * 1000, v)) for x, v in en[e0:e0 + 3] if x - t < 0.05]
    gaps.append(nv)
    print('%s QE flip %s  QI-QE %.2f ms  next VI %+.2f ms  next DISP-MSI %+.2f ms  LAST_DATA-en writes: %s' % (
        fmt(t), k[1], (t - qi.get(k, t)) * 1000, (nv or -1) * 1000, (nd or -1) * 1000, ' '.join(ens) or '-'))
g = sorted(x for x in gaps if x is not None)
print('# flips %d; next VI after flip QE: median %.2f ms p90 %.2f ms max %.2f ms' % (
    len(g), 1000 * g[len(g) // 2], 1000 * g[int(len(g) * .9)], 1000 * g[-1]))
print('# enable writes 0x611d80: %d (bit1 on %d, off %d); display W1C 0x611800: %d' % (
    len(en), sum(1 for _, v in en if v & 2), sum(1 for _, v in en if not v & 2), len(disp)))
# enable state at each flip QE and at each VI
def state(t):
    i = bisect.bisect_right(et, t) - 1
    return None if i < 0 else bool(en[i][1] & 2)
import collections
c = collections.Counter((state(qe[k]), (lambda i: vi[i] - qe[k] < 0.0301 if i < len(vi) else False)(bisect.bisect_left(vi, qe[k]))) for k in qe)
print('# (LAST_DATA enabled at flip QE, VI within 30 ms):', dict(c))
cv = collections.Counter(state(t) for t in vi)
print('# LAST_DATA enable state at each VI:', dict(cv))
