"""rdiff.py BODIES KF_REPLIES -- hardware reply vs kf reply for every replayed display control, grouped by
(cmd, distinct hardware request, distinct difference). Prints status differences and the differing u32 words."""
import collections, json, struct, sys
hw = {}
for l in open(sys.argv[1]):
    e = json.loads(l)
    if e['fn'] == 76:
        hw[e['n']] = e
groups = collections.OrderedDict()
for l in open(sys.argv[2]):
    f = l.split()
    if len(f) < 4 or not f[1].startswith('0x'):
        continue
    n = int(f[0]); cmd = int(f[1], 16); who = f[2]; kst = int(f[3].split('=')[1], 16)
    krep = bytes.fromhex(f[4]) if len(f) > 4 and f[4] != '-' else b''
    e = hw[n]
    hst = e.get('status')
    hrep = bytes.fromhex(e.get('rep') or '')
    req = bytes.fromhex(e['req'] or '')
    diffs = []
    if hst == 0 and kst == 0:
        m = max(len(hrep), len(krep))
        a = hrep.ljust(m + 4, b'\0'); b = krep.ljust(m + 4, b'\0'); r = req.ljust(m + 4, b'\0')
        for o in range(0, m, 4):
            x, y, q = struct.unpack_from('<I', a, o)[0], struct.unpack_from('<I', b, o)[0], struct.unpack_from('<I', r, o)[0]
            if x != y:
                diffs.append('+0x%03x req %08x hw %08x kf %08x' % (o, q, x, y))
    key = (cmd, hst, kst, who, tuple(diffs[:24]), len(diffs))
    g = groups.setdefault(key, [])
    g.append((n, e['t'], req))
for (cmd, hst, kst, who, diffs, nd), es in groups.items():
    if hst == kst and not diffs:
        continue
    n, t, req = es[0]
    print('== 0x%08x x%d first n=%d t=%.6f  HW status=0x%x  KF(%s) status=0x%x  differing words=%d' % (
        cmd, len(es), n, t % 60, hst, who, kst, nd))
    print('   first request: %s' % (req[:48].hex() + ('...' if len(req) > 48 else '')))
    for d in diffs:
        print('   ' + d)
print('# identical (status and body) groups:', sum(len(es) for (c, h, k, w, d, nd), es in groups.items() if h == k and not d))
