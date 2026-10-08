import json, struct, sys, collections, gzip, re
# setnotif.py GSP_JSONL | KF3_LOG.gz -- NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION (0x20800301) event/action in order
p = sys.argv[1]
c = collections.Counter(); order = []
if p.endswith('.jsonl'):
    for line in open(p):
        r = json.loads(line)
        if r.get('kind') != 'record' or r['direction'] != 0:
            continue
        b = bytes.fromhex(r['payload_hex']); i = b.find(b'VRPC')
        if i < 4 or struct.unpack_from('<I', b, i - 4 + 12)[0] != 76:
            continue
        body = b[i - 4 + 32:]
        if struct.unpack_from('<I', body, 8)[0] != 0x20800301:
            continue
        ev, act = struct.unpack_from('<2I', body, 40)
        c[(ev, act)] += 1; order.append((ev, act))
else:
    for line in gzip.open(p, 'rt', errors='replace'):
        if re.search(r'SET_NOTIFICATION|0x20800301', line) and 'rpc-trace' not in line:
            print('KF', line.strip()[:200])
print(sorted(c.items()))
print(order)
