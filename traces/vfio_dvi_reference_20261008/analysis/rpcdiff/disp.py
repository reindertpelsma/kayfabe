import re, sys, collections
# disp.py VFIO_SEQ VEND KF_SEQ KEND NAMES -- display-related controls/allocs (0x0073*, 0xc37x/c57x/c67x/c77x, 0x5070,
# 0x0080 display-ish) up to request index VEND / KEND: count and status set per kind, both sides
names = dict(l.split()[:2] for l in open(sys.argv[5]) if len(l.split()) >= 2)
def load(path, end, tag):
    c = collections.defaultdict(collections.Counter); first = {}
    for line in open(path):
        m = re.search(r' [RK](\d+) .*?(ctrl|alloc) (0x[0-9a-f]+) reply=(\S+)', line)
        if not m or int(m.group(1)) > end:
            continue
        k = m.group(3)
        disp = (m.group(2) == 'ctrl' and (k.startswith('0x0073') or k[:6] in ('0xc370', '0xc372', '0xc570', '0xc670', '0xc770') or k.startswith('0x5070'))) \
            or (m.group(2) == 'alloc' and k[:5] in ('0xc37', '0xc57', '0xc67', '0xc77') or k in ('0x0073', '0x5070', '0x402c', '0x9072'))
        if disp:
            c[m.group(2) + ' ' + k][m.group(4)] += 1
            first.setdefault(m.group(2) + ' ' + k, int(m.group(1)))
    return c, first
v, vf = load(sys.argv[1], int(sys.argv[2]), 'V'); k, kf = load(sys.argv[3], int(sys.argv[4]), 'K')
print('%-18s %-48s %-28s %-28s %s' % ('kind', 'name', 'VFIO (status x n)', 'kayfabe (status x n)', 'note'))
for key in sorted(set(v) | set(k), key=lambda x: (vf.get(x, 10**6), kf.get(x, 10**6))):
    fv = ' '.join('%s x%d' % (s, n) for s, n in sorted(v[key].items())) or '-'
    fk = ' '.join('%s x%d' % (s, n) for s, n in sorted(k[key].items())) or '-'
    note = 'VFIO only' if key not in k else ('kayfabe only' if key not in v else ('STATUS DIFFERS' if set(v[key]) != set(k[key]) else ''))
    print('%-18s %-48s %-28s %-28s %s' % (key, names.get(key.split()[1], '?'), fv, fk, note))
