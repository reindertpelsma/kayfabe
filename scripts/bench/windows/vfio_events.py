#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""vfio_events.py -- which GSP->CPU events a real GSP posts in a VFIO boot, and which
notifiers Windows arms.

usage: vfio_events.py VFIO_DECODED_JSON [--from N] [--to M]

Reads a `kayfabe-gsp-observer/1` export (the same input as abort_point.py forecast), with the
same de-duplication and the same request indexing. So "RPC index" here is abort_point.py's VFIO
index. For every GSP-initiated message (function >= 0x1000) it prints the number of request RPCs
seen before it (`after_rpc`), the function, and for POST_EVENT (0x1003, rpc_post_event_v17_00:
hClient@0 hEvent@4 notifyIndex@8 data@12 info16@16 status@20 eventDataSize@24 bNotifyList@28
eventData@29, ogkm-580.65.06 g_rpc-structures.h:1545-1556) the decoded fields. It then lists
every NV2080 EVENT_SET_NOTIFICATION (0x20800301) and NV0073 EVENT_SET_NOTIFICATION (0x00730301)
request in [from, to) with its index and action. Finally it lists the armed NV2080 indices that
were never posted, and the posted ones.

Diagnostic only. Bounded reads; nothing is executed or forwarded.
"""
import collections
import json
import struct
import sys

LIMIT = 256 * 1024 * 1024
POST_EVENT = 0x1003


def main(argv):
    path = argv[0]
    lo = int(argv[argv.index('--from') + 1]) if '--from' in argv else 0
    hi = int(argv[argv.index('--to') + 1]) if '--to' in argv else 1 << 30
    with open(path, 'rb') as f:
        raw = f.read(LIMIT + 1)
    if len(raw) > LIMIT:
        raise ValueError('exceeds read bound')
    doc = json.loads(raw)
    if doc.get('schema') != 'kayfabe-gsp-observer/1':
        raise ValueError('unexpected observer schema')
    seen, reqs, events = set(), [], []
    funcs = collections.Counter()
    for o in doc['observations']:
        b = bytes.fromhex(o['payload_hex'])
        dedup = (o['direction'], o['table_pa'], o['queue_sequence'], o['payload_hex'][:512])
        if dedup in seen:
            continue
        seen.add(dedup)
        i = b.find(b'VRPC')
        if i < 0 or len(b) < i + 28:
            continue
        fn = struct.unpack_from('<I', b, i + 8)[0]
        body = b[i + 28:]
        if fn >= 0x1000:
            funcs[fn] += 1
            ev = {'after_rpc': len(reqs), 'fn': fn}
            if fn == POST_EVENT and len(body) >= 29:
                (ev['hClient'], ev['hEvent'], ev['notifyIndex'], ev['data'], ev['info16']) = \
                    struct.unpack_from('<IIIIH', body, 0)
                ev['status'], ev['eventDataSize'] = struct.unpack_from('<II', body, 20)
                ev['bNotifyList'] = body[28]
                n = min(ev['eventDataSize'], max(0, len(body) - 29), 64)
                ev['eventData'] = body[29:29 + n].hex()
            events.append(ev)
            continue
        if o['direction'] != 'request':
            continue
        key = ''
        params = b''
        if fn == 76 and len(body) >= 40:
            cmd, psize = struct.unpack_from('<I', body, 8)[0], struct.unpack_from('<I', body, 16)[0]
            key = '%08x' % cmd
            params = body[40:40 + psize]
        reqs.append((fn, key, params))
    print(f'# {path}: {len(reqs)} request RPCs, {len(events)} GSP-initiated messages')
    print('# GSP-initiated functions:', ', '.join(f'{k:#x}x{v}' for k, v in sorted(funcs.items())))
    posted = collections.Counter()
    for e in events:
        if e['fn'] != POST_EVENT:
            continue
        posted[e['notifyIndex']] += 1
        print(f"POST_EVENT after_rpc={e['after_rpc']} notifyIndex={e['notifyIndex']} "
              f"hClient={e['hClient']:#x} hEvent={e['hEvent']:#x} data={e['data']:#x} "
              f"info16={e['info16']:#x} status={e['status']:#x} list={e['bNotifyList']} "
              f"size={e['eventDataSize']} eventData={e['eventData']}")
    armed = collections.Counter()
    for idx, (fn, key, params) in enumerate(reqs):
        if key == '20800301' and len(params) >= 8:
            ev, action = struct.unpack_from('<II', params, 0)
            armed[ev] += 1
            if lo <= idx < hi:
                print(f'ARM2080 rpc={idx} event={ev} action={action}')
        elif key == '00730301' and len(params) >= 16 and lo <= idx < hi:
            sub, h, ev, action = struct.unpack_from('<IIII', params, 0)
            print(f'ARM0073 rpc={idx} sub={sub} hEvent={h:#x} event={ev} action={action}')
    print('# NV2080 indices armed (whole boot):', dict(sorted(armed.items())))
    print('# indices posted (whole boot):', dict(sorted(posted.items())))
    print('# armed, never posted:', sorted(set(armed) - set(posted)))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
