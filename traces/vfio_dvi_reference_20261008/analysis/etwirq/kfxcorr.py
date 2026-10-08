#!/usr/bin/env python3
"""kfxcorr.py KF3_LOG.gz ETW.txt.gz -- kayfabe run: find the offset between maplog t= (host monotonic s) and the guest
ETW FILETIME by cross-correlating 'NSI host GR0 wake raised=true' with QE RENDER, then report, at that offset,
what fraction of QE RENDER/SIGNAL/PAGING follow a raised GR0/CE3 (or any raised) wake within 0.5 ms."""
import bisect, collections, datetime, gzip, re, sys
NSI = re.compile(r'maplog t=(\d+\.\d+) NSI host (\w+) wake #\d+ raised=(\w+)')
raised = collections.defaultdict(list)
for line in gzip.open(sys.argv[1], 'rt', errors='replace'):
    m = NSI.search(line)
    if m and m.group(3) == 'true':
        raised[m.group(2)].append(float(m.group(1)))
        raised['ANY'].append(float(m.group(1)))
qe = collections.defaultdict(list)
for line in gzip.open(sys.argv[2], 'rt', errors='replace'):
    if not line.startswith('D QE '):
        continue
    p = line.split(None, 5)
    ft = int(p[2][2:])
    typ = line.split('"')[1].replace('DXGKETW_', '').replace('_COMMAND_BUFFER', '').strip()
    qe[typ].append(ft / 1e7)          # seconds since 1601
g = sorted(raised['GR0'])
best = None
r = qe['RENDER']
# coarse: candidate offsets = QE - GR0 pairs within the first 50 QE x all GR0 (bounded), vote at 1 ms
votes = collections.Counter()
for t in r:
    for x in g:
        votes[round(t - x, 3)] += 1
off, n = votes.most_common(1)[0]
# refine at 0.1 ms
votes2 = collections.Counter()
for t in r:
    i = bisect.bisect_left(g, t - off - 0.003); j = bisect.bisect_right(g, t - off + 0.003)
    for x in g[i:j]:
        votes2[round(t - x - off, 4)] += 1
d, n2 = votes2.most_common(1)[0]
off += d
print('offset FILETIME_s - mono_s = %.4f (votes %d coarse, %d fine); top fine bins %s' % (off, n, n2, votes2.most_common(5)))
ep = datetime.datetime(1601, 1, 1)
for typ in ('RENDER', 'SIGNAL', 'PAGING', 'WAIT', 'MMIOFLIP'):
    for src in ('GR0', 'CE3', 'ANY'):
        s = sorted(raised[src])
        c = 0
        for t in qe[typ]:
            m = t - off
            i = bisect.bisect_right(s, m + 0.0002) - 1
            if i >= 0 and m - s[i] <= 0.0005:
                c += 1
        print('QE %-8s n=%4d preceded within 0.5 ms by raised %s: %d' % (typ, len(qe[typ]), src, c))
last = max(max(v) for v in qe.values())
print('last ETW QE at mono %.3f' % (last - off))
