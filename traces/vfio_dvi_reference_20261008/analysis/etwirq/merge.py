#!/usr/bin/env python3
"""merge.py BOOTDIR GUEST_MINUS_HOST_MS FROM TO PREFIX -- one merged host-time timeline of MSIs (with sources),
doorbells and DxgKrnl ETW events in [FROM, TO] (HH:MM:SS.ffffff host UTC)."""
import datetime, sys
bd, gmh, lo, hi, pre = sys.argv[1], float(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]
rows = []
for f, tag in ((pre + '-msi-timeline.txt', 'MSI'), (pre + '-doorbells.txt', 'DB ')):
    for line in open(f):
        t = line.split()[0]
        if lo <= t <= hi:
            rows.append((t, tag + ' ' + ' '.join(line.split()[1:])))
off = -gmh / 1000
for line in open(bd + '/vdr-etw.txt', errors='replace'):
    if not line.startswith('D '):
        continue
    p = line.split(None, 6)
    if len(p) < 7 or p[1] in ('ID0',):
        continue
    ft = int(p[2][2:])
    dt = datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=ft // 10) + datetime.timedelta(seconds=off)
    t = dt.strftime('%H:%M:%S.%f')
    if lo <= t <= hi:
        rows.append((t, 'ETW %-5s %s %s' % (p[1], p[3], p[6].strip()[:160])))
for t, s in sorted(rows):
    print(t, s)
