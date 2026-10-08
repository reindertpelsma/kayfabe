#!/usr/bin/env python3
"""kfwin.py KF3_LOG.gz ETW.txt.gz OFFSET LO HI -- merged kayfabe timeline (host monotonic s): NSI wakes, doorbell/ring
and act lines carrying maplog t=, plus guest ETW events mapped by OFFSET (FILETIME_s - mono_s)."""
import gzip, re, sys
log, etw, off, lo, hi = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
rows = []
T = re.compile(r't=(\d+\.\d+)')
for line in gzip.open(log, 'rt', errors='replace'):
    if 'maplog' not in line or any(k in line for k in ('SPLIT', 'ARRIVE', 'walk#', 'VasKey', 'CLEAR inval')):
        continue
    m = T.search(line)
    if m and lo <= float(m.group(1)) <= hi:
        rows.append((float(m.group(1)), 'KF  ' + line.strip()[len('kf3: maplog '):][:170]))
for line in gzip.open(etw, 'rt', errors='replace'):
    if not line.startswith('D '):
        continue
    p = line.split(None, 5)
    t = int(p[2][2:]) / 1e7 - off
    if lo <= t <= hi:
        rows.append((t, 'ETW ' + p[1] + ' ' + (p[5].strip() if len(p) > 5 else '')[:150]))
for t, s in sorted(rows):
    print('%.6f %s' % (t, s))
