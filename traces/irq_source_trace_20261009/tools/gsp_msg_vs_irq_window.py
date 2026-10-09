# usage: python3 -I gsp_msg_vs_irq_window.py BOOT_DIR GSP_SEQ.txt T0 T1   (as gsp_msg_vs_irq.py, restricted to messages in [T0, T1) seconds of the UTC day)
import sys, bisect, collections
d = sys.argv[1]
gsp = sys.argv[2]
def ts(s):
    h, m, r = s.split(':')
    return int(h) * 3600 + int(m) * 60 + float(r)
msi = []
rd = []   # (t, value)
w1c = []
for l in open(d + '/trace.log'):
    if 'vfio_msi_interrupt' in l:
        msi.append(ts(l[11:26]))
    elif 'vfio_region_read' in l and 'region0+0xb81010,' in l:
        rd.append((ts(l[11:26]), int(l.rsplit('= ', 1)[1], 16)))
    elif 'vfio_region_write' in l and 'region0+0xb81010,' in l:
        w1c.append((ts(l[11:26]), l.split('region0+0xb81010, ')[1].split(',')[0]))
rt = [r[0] for r in rd]
T0=float(sys.argv[3]);T1=float(sys.argv[4])
msg = []
for l in open(gsp):
    p = l.split()
    if T0<=ts(p[0])<T1: msg.append((ts(p[0]), p[1][0], l.strip()))
print('msi', len(msi), 'leaf4 reads', len(rd), 'w1c', len(w1c), 'gsp', len(msg))
# per message class: any bit27 pending read within 0..W ms after, and any MSI within 0..W ms after
for W in (0.0005, 0.002, 0.01):
    out = collections.Counter()
    for t, k, line in msg:
        kind = 'event' if k == 'E' else 'reply'
        i = bisect.bisect_left(rt, t)
        j = bisect.bisect_right(rt, t + W)
        reads = rd[i:j]
        out[(kind, 'n')] += 1
        if reads:
            out[(kind, 'has_reads')] += 1
            if any(v & (1 << 27) for _, v in reads):
                out[(kind, 'bit27_pending')] += 1
        mi = bisect.bisect_left(msi, t)
        mj = bisect.bisect_right(msi, t + W)
        if mj > mi:
            out[(kind, 'msi')] += 1
    print('window', W, dict(out))
