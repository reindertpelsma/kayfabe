# usage: python3 -I msi_leaf4_per_second.py BOOT_DIR   (BOOT_DIR/trace.log: MSIs and LEAF(4) reads with bit 27 pending, per second)
import sys, collections
d = sys.argv[1]
leaf = collections.defaultdict(lambda: [0, 0])
msi = collections.Counter()
for l in open(d + '/trace.log'):
    if 'vfio_msi_interrupt' in l:
        msi[l[11:19]] += 1
    elif 'vfio_region_read' in l and 'region0+0xb81010,' in l:
        v = int(l.rsplit('= ', 1)[1], 16)
        s = l[11:19]
        leaf[s][0] += 1
        if v & (1 << 27):
            leaf[s][1] += 1
for k in sorted(set(msi) | set(leaf)):
    print(k, 'msi', msi.get(k, 0), 'leaf4 reads', leaf[k][0], 'bit27', leaf[k][1])
