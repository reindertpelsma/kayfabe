#!/usr/bin/env python3
# cmdstats.py GSP_JSONL OUT.json -- per control command over a whole VFIO capture: count, real reply status histogram, params size, first reply bytes
import json, struct, sys, collections
st = collections.defaultdict(lambda: dict(n=0, status=collections.Counter(), psz=collections.Counter(), sample=None, objs=collections.Counter()))
tot = 0
for line in open(sys.argv[1]):
    try: r = json.loads(line)
    except ValueError: continue
    if r.get('kind') != 'record' or r['direction'] != 1: continue
    p = bytes.fromhex(r['payload_hex']); i = p.find(b'VRPC')
    if i < 4: continue
    hv, sig, ln, fn, res, resp, sq, spare = struct.unpack_from('<8I', p, i - 4)
    if fn != 76: continue
    b = p[i - 4 + 32:i - 4 + ln]
    if len(b) < 40: continue
    hc, ho, cmd, s, psz, f1, f2, f3 = struct.unpack_from('<8I', b, 0)
    e = st['%08x' % cmd]; e['n'] += 1; e['status']['%x' % s] += 1; e['psz'][psz] += 1; e['objs']['%08x' % ho] += 1; tot += 1
    if e['sample'] is None or (s == 0 and not e['sample'][0]): e['sample'] = (s == 0, b[40:40 + 64].hex())
out = {c: dict(n=e['n'], status=dict(e['status']), psz=dict(e['psz']), sample=e['sample'][1], objs=dict(e['objs'])) for c, e in st.items()}
json.dump(out, open(sys.argv[2], 'w'), indent=0)
print('replies', tot, 'distinct cmds', len(out))
