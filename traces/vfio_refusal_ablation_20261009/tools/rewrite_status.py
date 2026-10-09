#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""rewrite_status.py GSP_JSONL [REFUSE_FILE] -- what the guest was told for the requests the ablation rewrote.

⊘ EXPLORATORY ONLY -- superseded by verify_rewrites.py for anything that needs to be trusted. This script's FIFO
request/reply pairing (same assumption as vfio_kf_rpc_diff.py) silently miscounts once the observer's capture has a
gap (sequence_gaps in its footer): a single dropped request or reply shifts every later pairing. verify_rewrites.py
avoids this by classifying replies directly from the field the GSP echoes back, never pairing by submission order.
This file is kept for the exploratory session record (see traces/vfio_refusal_ablation_20261009/README.md); its
counts for t1/probe2 undercounted relative to the corrected tool and should not be cited.

Requests (direction 0) are paired FIFO per function with replies (direction 1), as vfio_kf_rpc_diff.py does. The observer
records the ORIGINAL request (it samples before the rewrite), so `req` is what the guest sent; `reply_key` is what the
real GSP echoed (the rewritten key); rpc_result/status are what the guest received. Prints, per (fn, req key) in the
refuse file (or every non-zero status when no file is given): count, the reply key, rpc_result, status.
"""
import collections
import json
import struct
import sys


def pairs(path):
    pend = collections.defaultdict(collections.deque)
    for line in open(path, encoding='utf-8-sig'):
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if r.get('kind') != 'record':
            continue
        p = bytes.fromhex(r['payload_hex'])
        i = p.find(b'VRPC')
        if i < 4 or len(p) < i + 28:
            continue
        hv, sig, ln, fn, res, resp, sq, sp = struct.unpack_from('<8I', p, i - 4)
        body = p[i + 28:]
        d = {'dir': r['direction'], 'fn': fn, 'res': res, 'key': None, 'st': 0, 'len': ln, 'qpc': r['qpc']}
        if fn == 76 and len(body) >= 16:
            d['key'], d['st'] = struct.unpack_from('<II', body, 8)
        elif fn == 103 and len(body) >= 20:
            d['key'], d['st'] = struct.unpack_from('<II', body, 12)
        if d['dir'] == 0:
            pend[fn].append(d)
        elif fn < 0x1000 and pend[fn]:
            yield pend[fn].popleft(), d


def main():
    path = sys.argv[1]
    rules = None
    if len(sys.argv) > 2:
        rules = set()
        for l in open(sys.argv[2]):
            l = l.split('#')[0].split()
            if len(l) >= 2:
                rules.add((int(l[0], 0), int(l[1], 0)))
    out = collections.Counter()
    for q, rp in pairs(path):
        if q['fn'] not in (76, 103):
            continue
        k = (q['fn'], q['key'])
        if rules is not None and k not in rules:
            continue
        if rules is None and not (rp['res'] or rp['st']):
            continue
        out[(q['fn'], q['key'], rp['key'], rp['res'], rp['st'], rp['len'] > 4000)] += 1
    print('fn   req_key     reply_key   rpc_result status  big_reply  count')
    for (fn, rk, pk, res, st, big), n in sorted(out.items(), key=lambda kv: (kv[0][0], kv[0][1] or 0)):
        print('%-4d 0x%08x  0x%08x  0x%-8x  0x%-5x  %-9s  %d' % (fn, rk, pk or 0, res, st, big, n))
    stat = collections.Counter((fn, res, st) for (fn, rk, pk, res, st, big), n in out.items() for _ in range(n))
    print('status histogram (fn, rpc_result, status):', dict(stat))


main()
