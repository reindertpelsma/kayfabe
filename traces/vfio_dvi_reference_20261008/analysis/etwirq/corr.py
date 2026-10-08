#!/usr/bin/env python3
"""corr.py BOOTDIR GUEST_MINUS_HOST_MS OUTPREFIX -- correlate MSI (+vectors, display, doorbells) with DxgKrnl ETW.
All times printed as host UTC seconds-of-day (HH:MM:SS.ffffff)."""
import bisect, collections, re, sys, datetime

bd, gmh, outp = sys.argv[1], float(sys.argv[2]), sys.argv[3]
LINE = re.compile(r'^\S+T(\d\d):(\d\d):(\d\d\.\d+)Z (vfio_\w+)\s+\((\S+?)(?::region0\+(0x[0-9a-f]+), (0x[0-9a-f]+))?[,)]')
RD = re.compile(r'^\S+T(\d\d):(\d\d):(\d\d\.\d+)Z vfio_region_read\s+\(0000:01:00.0:region0\+(0x[0-9a-f]+), \d\) = (0x[0-9a-f]+)')
VEC = {0: 'GR0', 2: 'SEC2', 3: 'NVDEC0', 7: 'CE2', 8: 'CE3', 10: 'CE4', 11: 'NVENC1', 18: 'OFA0', 64: 'RFAULT',
       72: 'ACCNTR', 129: 'CPUDB', 131: 'RFAULTERR', 154: 'DISP', 155: 'GSP'}


def sod(h, m, s):
    return int(h) * 3600 + int(m) * 60 + float(s)


def fmt(t):
    h = int(t // 3600); m = int(t % 3600 // 60); s = t % 60
    return '%02d:%02d:%09.6f' % (h, m, s)


msi = []  # [t, set(vectors), disp_clear_vals, disp_reads]
db = []   # (t, token)
for line in open(bd + '/trace.log', errors='replace'):
    m = LINE.match(line)
    if m:
        t = sod(m.group(1), m.group(2), m.group(3)); ev = m.group(4)
        if ev == 'vfio_msi_interrupt' and m.group(5) == '0000:01:00.0':
            msi.append([t, set(), [], []])
        elif ev == 'vfio_region_write' and m.group(6):
            off, val = int(m.group(6), 16), int(m.group(7), 16)
            if off == 0xbb0090:
                db.append((t, val))
            elif msi and 0xb81000 <= off < 0xb81020:
                leaf = (off - 0xb81000) // 4
                for b in range(32):
                    if val >> b & 1:
                        msi[-1][1].add(32 * leaf + b)
            elif msi and off == 0x611800:
                msi[-1][2].append(val)
        continue
    m = RD.match(line)
    if m and msi and int(m.group(4), 16) == 0x611800:
        msi[-1][3].append(int(m.group(5), 16))


def src(x):
    s = [VEC.get(v, str(v)) for v in sorted(x[1])]
    if x[2]:
        s.append('DISP611800w' + '/'.join('%x' % v for v in x[2]))
    return '+'.join(s) or '-'


# ETW
off = -gmh / 1000.0   # host = guest + off
etw = []
for line in open(bd + '/vdr-etw.txt', errors='replace'):
    if not line.startswith('D '):
        continue
    p = line.split(None, 6)
    if len(p) < 7:
        continue
    tag = p[1]
    if tag not in ('QE', 'QI', 'QS', 'S', 'E', 'SIG', 'ID550', 'ID551', 'VI', 'VD', 'UQP', 'UNW', 'QI2'):
        continue
    ft = int(p[2][2:])
    dt = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=ft // 10)
    t = dt.hour * 3600 + dt.minute * 60 + dt.second + dt.microsecond / 1e6 + (ft % 10) / 1e7 + off
    ud = p[6].strip()
    typ = ud.split('"')[1].strip() if '"' in ud else ''
    ctx = ud.split(',')[0]
    etw.append((t, tag, typ, ctx, ud))
etw.sort()
mt = [x[0] for x in msi]

with open(outp + '-msi-timeline.txt', 'w') as f:
    for x in msi:
        f.write('%s %s%s\n' % (fmt(x[0]), src(x), (' rd611800=' + '/'.join('%x' % v for v in x[3])) if x[3] else ''))
with open(outp + '-doorbells.txt', 'w') as f:
    for t, v in db:
        f.write('%s 0x%x\n' % (fmt(t), v))

# per-second table
per = collections.defaultdict(collections.Counter)
for x in msi:
    k = fmt(x[0])[:8]
    per[k]['MSI'] += 1
    for v in x[1]:
        per[k][VEC.get(v, str(v))] += 1
    if x[2]:
        per[k]['DISP611800'] += 1
    if not x[1] and not x[2]:
        per[k]['none'] += 1
etwps = collections.defaultdict(collections.Counter)
for e in etw:
    k = fmt(e[0])[:8]
    if e[1] == 'QE':
        etwps[k]['QE_' + e[2].replace('DXGKETW_', '').replace('_COMMAND_BUFFER', '')] += 1
    if e[1] in ('VI', 'SIG', 'ID550'):
        etwps[k][e[1]] += 1
dbps = collections.Counter(fmt(t)[:8] for t, _ in db)
with open(outp + '-per-second.txt', 'w') as f:
    for k in sorted(set(per) | set(etwps)):
        f.write('%s %s | doorbells=%d | %s\n' % (k, ' '.join('%s:%d' % kv for kv in sorted(per[k].items())), dbps[k],
                                               ' '.join('%s:%d' % kv for kv in sorted(etwps[k].items()))))

# first render activity
firsts = {}
for e in etw:
    key = (e[1], e[2])
    if key not in firsts:
        firsts[key] = e
print('ETW first occurrences (host time):')
for k, e in sorted(firsts.items(), key=lambda kv: kv[1][0]):
    print('  %s %s %s ctx=%s' % (fmt(e[0]), k[0], k[1], e[3]))

# fraction of render QE preceded by a nonstall MSI within N ms
def kinds_before(t, win):
    i = bisect.bisect_right(mt, t)
    out = collections.Counter()
    j = i - 1
    while j >= 0 and t - mt[j] <= win:
        x = msi[j]
        if x[1] & {0, 7, 8, 10}:
            out['nonstall'] += 1
            for v in x[1] & {0, 7, 8, 10}:
                out[VEC[v]] += 1
        if x[2]:
            out['disp'] += 1
        if 155 in x[1]:
            out['gsp'] += 1
        if not x[1] and not x[2]:
            out['none'] += 1
        out['any'] += 1
        j -= 1
    return out

for typ in ('RENDER', 'SIGNAL', 'WAIT', 'MMIOFLIP', 'PAGING', 'SOFTWARE'):
    qes = [e for e in etw if e[1] == 'QE' and typ in e[2]]
    if not qes:
        continue
    for win in (0.0005, 0.002, 0.010):
        c = collections.Counter()
        for e in qes:
            kb = kinds_before(e[0], win)
            for k in ('any', 'nonstall', 'GR0', 'CE2', 'CE3', 'disp', 'gsp', 'none'):
                if kb[k]:
                    c[k] += 1
        print('QE %-9s n=%5d  within %4.1f ms preceded by MSI: any %d, nonstall %d (GR0 %d CE2 %d CE3 %d), disp %d, gsp %d, unattributed %d'
              % (typ, len(qes), win * 1000, c['any'], c['nonstall'], c['GR0'], c['CE2'], c['CE3'], c['disp'], c['gsp'], c['none']))

# doorbell -> nonstall latency: for each nonstall MSI, the most recent doorbell before it
dt_ = [t for t, _ in db]
lat = collections.defaultdict(list)
for x in msi:
    for v in x[1] & {0, 7, 8, 10}:
        i = bisect.bisect_right(dt_, x[0]) - 1
        if i >= 0:
            lat[(VEC[v], db[i][1])].append(x[0] - db[i][0])
print('nonstall MSI <- last doorbell token: count, median ms, p90 ms')
for k, l in sorted(lat.items(), key=lambda kv: -len(kv[1]))[:20]:
    l.sort()
    print('  %s token 0x%x: n=%d med=%.3f p90=%.3f' % (k[0], k[1], len(l), 1000 * l[len(l) // 2], 1000 * l[int(len(l) * 0.9)]))
print('MSI total', len(msi), 'doorbells', len(db), 'etw events', len(etw))
