import re, sys, bisect
# real HW: for each LAST_DATA enable (0x611d80 bit1 0->1): was 0x611800 W1C'd just before; latency to first DISP W1C after
W = re.compile(r'T(\d\d):(\d\d):(\d\d\.\d+)Z vfio_region_(write|read)\s+\(0000:01:00.0:region0\+(0x611800|0x611d80), (?:(0x[0-9a-f]+), \d\)|\d\) = (0x[0-9a-f]+))')
ev = []
for line in open(sys.argv[1], errors='replace'):
    m = W.search(line)
    if m:
        t = int(m.group(1))*3600 + int(m.group(2))*60 + float(m.group(3))
        ev.append((t, m.group(4), m.group(5), int(m.group(6) or m.group(7), 16)))
lat = []; pre = []
for i, (t, k, r, v) in enumerate(ev):
    if k == 'write' and r == '0x611d80' and v & 2:
        cleared = any(e[1] == 'write' and e[2] == '0x611800' and t - e[0] < 0.0002 for e in ev[max(0, i-6):i])
        nxt = next((e[0] for e in ev[i+1:] if e[1] == 'write' and e[2] == '0x611800'), None)
        lat.append((nxt - t) * 1000 if nxt else None); pre.append(cleared)
l = sorted(x for x in lat if x is not None)
print('enables', len(lat), 'cleared-just-before', sum(pre), 'latency to first DISP W1C ms: min %.2f med %.2f max %.2f' % (l[0], l[len(l)//2], l[-1]))
print('latencies < 1 ms:', sum(1 for x in l if x < 1.0))
