import re, sys, collections
# per UTC second: window-0 PUTs, LATCHes, Passthrough doorbells (maplog DOORBELL), kf3 clock -> UTC by OFFSET
f, off = sys.argv[1], float(sys.argv[2])
c = collections.defaultdict(collections.Counter)
t = None
for l in open(f, errors='replace'):
    m = re.search(r'(?:WTRACE|maplog) t=([\d.]+) ', l)
    if m:
        t = float(m.group(1))
    if t is None:
        continue
    s = t - off
    k = None
    if 'LATCH window' in l:
        k = 'latch'
    elif 'WRITE 0x690000' in l:
        k = 'put0'
    elif 'DOORBELL chid' in l:
        k = 'db ' + l.split('DOORBELL chid ')[1].split()[0]
    elif 'TRACE release chn 1' in l:
        k = 'rel1'
    elif 'TRACE notify chn 1' in l:
        k = 'not1'
    if k:
        c[int(s)][k] += 1
for s in sorted(c):
    m, sec = divmod(s, 60)
    h, m = divmod(m, 60)
    print('%02d:%02d:%02d' % (h % 24, m, sec), ' '.join('%s=%d' % kv for kv in sorted(c[s].items())))
