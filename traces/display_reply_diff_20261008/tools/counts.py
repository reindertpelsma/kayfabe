"""counts.py BODIES KFSEQ TCUT -- per display-ish control: HW count/status set (before TCUT seconds-of-day) vs kf count/status."""
import collections, json, re, sys
hw = collections.defaultdict(lambda: [0, collections.Counter(), None, set()])
for l in open(sys.argv[1]):
    e = json.loads(l)
    if e['fn'] != 76 or e['t'] > float(sys.argv[3]):
        continue
    c = e['cmd']
    cls = c >> 16
    if cls in (0x73, 0x5070, 0xc370, 0xc372, 0xc37d, 0xc67d, 0xc77d, 0x402c) or c in (0x20808159,) or (cls == 0x2080 and (c >> 8) & 0xff in (0x0a,)):
        h = hw[c]
        h[0] += 1
        h[1][e.get('status')] += 1
        if h[2] is None:
            h[2] = e['n']
        h[3].add(e['psz'])
kf = collections.defaultdict(lambda: [0, collections.Counter(), None])
R = re.compile(r'K(\d+) \S+\s+ctrl (0x[0-9a-f]+) reply=(\S+)')
for l in open(sys.argv[2]):
    m = R.search(l)
    if m:
        c = int(m.group(2), 16)
        k = kf[c]
        k[0] += 1
        k[1][m.group(3)] += 1
        if k[2] is None:
            k[2] = int(m.group(1))
keys = sorted(set(hw) | set(k for k in kf if (k >> 16) in (0x73, 0x5070, 0xc370, 0xc372, 0xc37d, 0xc67d, 0x402c)), key=lambda c: (hw[c][2] if c in hw and hw[c][2] is not None else 1e9))
for c in keys:
    h = hw.get(c)
    k = kf.get(c)
    print('0x%08x HW n=%-4s first=%-5s st=%-18s psz=%-10s | KF n=%-4s first=%-5s st=%s' % (
        c, h[0] if h else 0, h[2] if h else '-', dict((hex(a) if a is not None else 'none', b) for a, b in h[1].items()) if h else '-',
        sorted(h[3]) if h else '-', k[0] if k else 0, k[2] if k else '-', dict(k[1]) if k else '-'))
