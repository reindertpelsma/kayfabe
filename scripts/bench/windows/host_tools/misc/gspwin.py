#!/usr/bin/env python3
"""Print a window of a kayfabe-gsp-text/1 export: per record dir, fn, ctrl cmd, POST_EVENT fields.
usage: gspwin.py gsp.jsonl [--grep CMDHEX] [--ctx N] [--all]"""
import json, struct, sys

path = sys.argv[1]
grep = sys.argv[sys.argv.index('--grep') + 1].lower() if '--grep' in sys.argv else None
ctx = int(sys.argv[sys.argv.index('--ctx') + 1]) if '--ctx' in sys.argv else 15
recs = []
seen = set()
with open(path) as f:
    for line in f:
        o = json.loads(line)
        if o.get('kind') != 'record':
            continue
        h = o['payload_hex']
        d = (o['direction'], o['table_pa'], o['queue_sequence'], h[:512])
        if d in seen:
            continue
        seen.add(d)
        b = bytes.fromhex(h)
        i = b.find(b'VRPC')
        if i < 0 or len(b) < i + 28:
            continue
        fn = struct.unpack_from('<I', b, i + 8)[0]
        res = struct.unpack_from('<I', b, i + 12)[0]
        body = b[i + 28:]
        desc = ''
        if fn == 76 and len(body) >= 40:
            hc, ho, cmd, st, ps = struct.unpack_from('<IIIII', body, 0)
            p = body[40:40 + min(ps, 96)]
            desc = f'ctrl {cmd:08x} hC={hc:#x} hO={ho:#x} st={st:#x} psz={ps} p={p.hex()}'
        elif fn == 0x1003 and len(body) >= 29:
            hc, he, ni, da, i16 = struct.unpack_from('<IIIIH', body, 0)
            st, sz = struct.unpack_from('<II', body, 20)
            desc = f'POST_EVENT hC={hc:#x} hE={he:#x} idx={ni} data={da:#x} info16={i16:#x} st={st:#x} list={body[28]} ed={body[29:29+min(sz,32)].hex()}'
        elif fn == 103 and len(body) >= 24:
            hc, hp, ho, cls = struct.unpack_from('<IIII', body, 0)
            desc = f'alloc cls={cls:#x} hC={hc:#x} parent={hp:#x} h={ho:#x}'
        elif fn == 10 and len(body) >= 12:
            desc = 'free ' + body[:16].hex()
        else:
            desc = body[:48].hex()
        recs.append((o['qpc'], o['direction'], fn, res, desc))

t0 = recs[0][0]
hits = [k for k, r in enumerate(recs) if grep and grep in r[4]] if grep else []
if '--all' in sys.argv:
    show = range(len(recs))
else:
    s = set()
    for k in hits:
        s.update(range(max(0, k - ctx), min(len(recs), k + ctx + 1)))
    show = sorted(s)
prev = None
for k in show:
    if prev is not None and k != prev + 1:
        print('   ...')
    q, d, fn, res, desc = recs[k]
    print(f'{k:5d} t={(q - t0) / 1e9:10.6f} dir={d} fn={fn:#x} res={res:#x} {desc}')
    prev = k
