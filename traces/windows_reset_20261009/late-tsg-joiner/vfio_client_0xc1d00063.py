import json, struct

ev = []
for l in open('vfio_gsp.jsonl'):
    r = json.loads(l)
    if r['kind'] != 'record' or r['direction'] != 0:
        continue
    b = bytes.fromhex(r['payload_hex'])
    i = b.find(b'VRPC')
    if i < 4:
        continue
    h = i - 4
    ver, sig, length, fn, res, resp, seq, u = struct.unpack_from('<8I', b, h)
    ev.append((r['qpc'], fn, b[h + 32:]))
ev.sort(key=lambda e: e[0])
t0 = ev[0][0]
for q, fn, body in ev:
    t = (q - t0) / 1e9
    if not (59.40 <= t <= 61.5):
        continue
    if fn == 103:
        hc, hp, ho, cls, st, ps, fl = struct.unpack_from('<7I', body, 0)
        if hc == 0xc1d00063:
            print(f"{t:.4f} ALLOC cls={cls:#x} parent={hp:#x} h={ho:#x}")
    elif fn == 76:
        hc, ho, cmd, st, ps = struct.unpack_from('<5I', body, 0)
        if hc == 0xc1d00063:
            print(f"{t:.4f} CTRL cmd={cmd:#x} obj={ho:#x} ps={ps}")
