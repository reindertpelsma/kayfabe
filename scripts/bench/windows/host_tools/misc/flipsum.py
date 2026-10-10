#!/usr/bin/env python3
"""flipsum.py QEMU_LOG -- summarise a kf3 Windows boot for the H-flip/H-loadv record (display write trace lines
`kf3: display: WTRACE t=...`): PUTs per channel kind and the time of the first/last, the LAST_DATA enable/disable
timeline (RM_INTR_EN_HEAD_TIMING(0) = 0x611d80), raised head-timing interrupts and whether a window latch came
between consecutive raised VSyncs, window latches, the stall marker and the Passthrough births/frees."""
import re, sys, collections
W = re.compile(r'WTRACE t=([\d.]+) (WRITE|VSYNC|LATCH) (.*)')
rows = []
other = collections.Counter()
births = frees = 0
marker_t = None
last_t = None
for line in open(sys.argv[1], errors='replace'):
    m = W.search(line)
    if m:
        t = float(m.group(1)); last_t = t
        rows.append((t, m.group(2), m.group(3)))
        continue
    mt = re.search(r'maplog t=([\d.]+)', line)
    if mt:
        last_t = float(mt.group(1))
    if 'BORN Passthrough' in line:
        births += 1
    if 'route=passthrough' in line:
        frees += 1
    if 'cmd=0x00730108' in line and births and marker_t is None:
        marker_t = last_t
puts = collections.Counter(); first_put = {}; last_put = {}
en = []
vs = []
lat = []
w1c = 0
blank = []
for t, k, rest in rows:
    if k == 'WRITE':
        off = int(rest.split()[0], 16); val = int(rest.split()[2], 16)
        if 'Put(' in rest:
            chn = int(re.search(r'Put\((\d+)\)', rest).group(1))
            kind = 'core' if chn == 0 else 'win%d' % (chn - 1) if chn <= 32 else 'winim%d' % (chn - 33) if chn <= 64 else 'c%d' % chn
            puts[kind] += 1; first_put.setdefault(kind, t); last_put[kind] = t
        elif off == 0x611d80:
            en.append((t, val))
        elif off == 0x611800:
            w1c += 1
        elif off == 0x680240:
            blank.append((t, val))
    elif k == 'VSYNC':
        vs.append(t)
    elif k == 'LATCH':
        lat.append((t, rest))
t0 = rows[0][0] if rows else 0
print('trace lines', len(rows), 'first t', t0)
print('PUTs per channel:', dict(puts))
for kind in sorted(first_put):
    print('  %s first %.6f last %.6f' % (kind, first_put[kind], last_put[kind]))
print('EVT_STAT W1C writes (0x611800):', w1c, ' SET_GET_BLANKING_CTRL writes (0x680240):', ['%.6f=%#x' % x for x in blank][:20])
print('LAST_DATA enable writes (0x611d80): %d' % len(en))
for t, v in en[:60]:
    print('  %.6f EN <- %#x (%s)' % (t, v, 'on' if v & 2 else 'off'))
print('raised head-timing interrupts (VSYNC lines):', len(vs))
print('window latches:', len(lat), collections.Counter(r for _, r in lat).most_common(8))
# latches that were followed by a raised VSync while enabled (a flip retired by a vsync, from the display side)
vi = 0
for t, r in lat:
    nxt = [x for x in vs if x > t]
    if nxt and nxt[0] - t < 0.034:
        vi += 1
print('window latches followed by a raised VSync within 34 ms:', vi, 'of', len(lat))
print('passthrough births', births, 'passthrough frees', frees, 'stall marker (0x00730108 after first birth) near t', marker_t)
