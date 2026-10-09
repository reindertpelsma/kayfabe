import sys, re, collections
path, t0, t1 = sys.argv[1:4]
lo = int(sys.argv[4], 16); hi = int(sys.argv[5], 16)
R = re.compile(r'^2026-10-09T(\d\d):(\d\d):(\d\d\.\d+)Z (\S+)\s+\(0000:01:00.0:region0\+(0x[0-9a-f]+), (?:(0x[0-9a-f]+), )?(\d)\)(?: = (0x[0-9a-f]+))?')
a = [int(x) for x in t0.split(':')]; b = [int(x) for x in t1.split(':')]
A = a[0]*3600+a[1]*60+a[2]; B = b[0]*3600+b[1]*60+b[2]
c = collections.Counter()
for l in open(path, errors='replace'):
    m = R.match(l)
    if not m: continue
    t = int(m.group(1))*3600+int(m.group(2))*60+float(m.group(3))
    if not (A <= t < B): continue
    off = int(m.group(5), 16)
    if not (lo <= off < hi): continue
    kind = 'w' if m.group(4).endswith('write') else 'r'
    val = m.group(6) if kind == 'w' else m.group(8)
    c[(kind, hex(off), val)] += 1
for k, v in c.most_common(int(sys.argv[6]) if len(sys.argv) > 6 else 25):
    print(v, *k)
