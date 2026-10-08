"""dispacc.py TRACE T0 T1 [lo hi] -- display-range BAR0 accesses between host-UTC seconds-of-day T0..T1."""
import re, sys
R = re.compile(r'T(\d\d):(\d\d):(\d\d\.\d+)Z vfio_region_(read|write)\s+\(0000:01:00.0:region0\+(0x[0-9a-f]+), (?:(\d)\) = (0x[0-9a-f]+)|(0x[0-9a-f]+), \d\))')
t0, t1 = float(sys.argv[2]), float(sys.argv[3])
lo = int(sys.argv[4], 16) if len(sys.argv) > 4 else 0x610000
hi = int(sys.argv[5], 16) if len(sys.argv) > 5 else 0x700000
for line in open(sys.argv[1], errors='replace'):
    m = R.search(line)
    if not m:
        continue
    t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3))
    if t < t0:
        continue
    if t > t1:
        break
    a = int(m.group(5), 16)
    if not lo <= a < hi:
        continue
    v = m.group(7) or m.group(8)
    print('%.6f %s 0x%06x %s' % (t % 60, 'R' if m.group(4) == 'read' else 'W', a, v))
