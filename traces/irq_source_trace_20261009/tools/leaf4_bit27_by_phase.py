# usage: python3 -I leaf4_bit27_by_phase.py BOOT_DIR   (LEAF(4) reads with bit 27 pending and MSIs, per boot3 phase)
import sys
d = sys.argv[1]
def ts(s):
    h, m, r = s.split(':')
    return int(h) * 3600 + int(m) * 60 + float(r)
ph = [("init 20:10:10-12", 72610, 72612), ("busy 20:10:14-20", 72614, 72620), ("quiet 20:10:20-50", 72620, 72650),
      ("lockscreen idle 20:10:54-:11:08", 72654, 72668), ("idle 20:11:10-:11:39", 72670, 72699), ("end 20:11:40-:11:49", 72700, 72710), ("last 20:11:49+", 72709, 80000)]
tot = [[0, 0] for _ in ph]
msi = [0] * len(ph)
for l in open(d + '/trace.log'):
    if 'vfio_msi_interrupt' in l:
        t = ts(l[11:26])
        for i, (n, a, b) in enumerate(ph):
            if a <= t < b: msi[i] += 1
    elif 'vfio_region_read' in l and 'region0+0xb81010,' in l:
        t = ts(l[11:26]); v = int(l.rsplit('= ', 1)[1], 16)
        for i, (n, a, b) in enumerate(ph):
            if a <= t < b:
                tot[i][0] += 1
                if v & (1 << 27): tot[i][1] += 1
for i, (n, a, b) in enumerate(ph):
    print(n, 'leaf4 reads', tot[i][0], 'bit27', tot[i][1], 'pct', round(100 * tot[i][1] / max(1, tot[i][0]), 1), 'msi', msi[i])
