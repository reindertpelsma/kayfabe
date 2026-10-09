import sys, re, collections
path, t0, t1 = sys.argv[1], sys.argv[2], sys.argv[3]   # HH:MM:SS bounds (UTC)
step = float(sys.argv[4]) if len(sys.argv) > 4 else 1.0
R = re.compile(r'^2026-10-09T(\d\d):(\d\d):(\d\d\.\d+)Z (\S+)\s+\(0000:01:00.0:region0\+(0x[0-9a-f]+), (?:(0x[0-9a-f]+), )?(\d)\)(?: = (0x[0-9a-f]+))?')
def secs(h,m,s): return int(h)*3600+int(m)*60+float(s)
def hms(x): return "%02d:%02d:%06.3f" % (x//3600, (x%3600)//60, x%60)
a = [int(x) for x in t0.split(':')]; b = [int(x) for x in t1.split(':')]
A = a[0]*3600+a[1]*60+a[2]; B = b[0]*3600+b[1]*60+b[2]
bins = collections.defaultdict(collections.Counter)
msi = collections.Counter()
for l in open(path, errors='replace'):
    if l[11:13] < '%02d' % a[0] or False: continue
    if 'vfio_msi_interrupt' in l:
        m = re.match(r'^2026-10-09T(\d\d):(\d\d):(\d\d\.\d+)Z', l)
        if m:
            t = secs(*m.groups())
            if A <= t < B: msi[int(t//step)] += 1
        continue
    m = R.match(l)
    if not m: continue
    t = secs(*m.groups()[:3])
    if not (A <= t < B): continue
    kind, off = m.group(4), int(m.group(5), 16)
    k = int(t//step)
    if kind == 'vfio_region_write':
        bins[k]['w:%#x' % (off & ~0xfff)] += 1
    else:
        bins[k]['r:%#x' % (off & ~0xfff)] += 1
for k in sorted(set(bins) | set(msi)):
    top = ' '.join('%s=%d' % kv for kv in bins[k].most_common(7))
    print(hms(k*step), 'msi=%d' % msi[k], top)
