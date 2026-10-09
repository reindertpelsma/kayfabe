#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""make_refusal_list.py RUNDIR... > refusal-full.txt

The refusal set of kayfabe: every (fn, key) whose reply in the observer stream (gsp.jsonl) of a kayfabe Windows run is a
refusal. Requests (direction 0) are paired FIFO per function with replies (direction 1), as vfio_kf_rpc_diff.py does.
A reply is a refusal when the VRPC rpc_result is non-zero (kayfabe answers a refused control that way, with an all-zero
body) or the body status is non-zero (kayfabe's alloc refusals: rpc_result == status).
⊘ this FIFO pairing can miscount individual occurrences once a capture has gaps (see verify_rewrites.py's header for
why), so the per-key COUNTS below are approximate; the SET of refused (fn,key) pairs is not -- it was cross-checked
against kayfabe's own `kf3: GSP REFUSED fnF/0xK=0xST` qemu.log lines (its internal refusal decision, independent of
any reply pairing) for runs 114/118/152 and matched exactly (the only extra qemu.log line, fn103/0xc56f, is the one
already excluded below).
  fn 76  GSP_RM_CONTROL : key = cmd    (request body +8)
  fn 103 GSP_RM_ALLOC   : key = hClass (request body +12)
  fn 71  CONTINUATION_RECORD: the continuation records of an over-long control/alloc; kayfabe answers them with the head's
          result, they are not requests of their own. Not rewritable: listed in a comment only.
  excluded on purpose: fn103 hClass 0xc56f with status 0x40 (a different cause; the owner's brief says ignore it).
Output rules are `FN KEY` (hex), sorted; comments say how many times each was refused in how many of the runs.
"""
import collections
import json
import struct
import sys

IGNORE = {(103, 0xc56f)}


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
        d = {'dir': r['direction'], 'fn': fn, 'res': res, 'key': None, 'st': 0}
        if fn == 76 and len(body) >= 16:
            d['key'], d['st'] = struct.unpack_from('<II', body, 8)
        elif fn == 103 and len(body) >= 20:
            d['key'], d['st'] = struct.unpack_from('<II', body, 12)
        if d['dir'] == 0:
            pend[fn].append(d)
        elif fn < 0x1000 and pend[fn]:
            yield pend[fn].popleft(), d


def main(runs):
    seen = collections.defaultdict(lambda: [0, set()])
    for run in runs:
        for q, rp in pairs(run + '/gsp.jsonl'):
            if rp['res'] not in (0, 0xffffffff) or rp['st']:
                k = (q['fn'], q['key'])
                seen[k][0] += 1
                seen[k][1].add(run.rstrip('/').split('-')[-1])
    print('# kayfabe refusal set, computed by make_refusal_list.py from the observer streams of runs',
          ' '.join(r.rstrip('/').split('-')[-1] for r in runs))
    print('# format: FN KEY  (NEWKEY optional: default = vg_refuse_default_key)')
    n = 0
    for (fn, key), (cnt, rs) in sorted(seen.items(), key=lambda kv: (kv[0][0], kv[0][1] or 0)):
        if fn == 71:
            print('# fn71 CONTINUATION_RECORD: refused %d times in runs %s; not a request of its own, not rewritten' %
                  (cnt, ','.join(sorted(rs))))
            continue
        if (fn, key) in IGNORE:
            print('# ignored: fn%d 0x%x (different cause)' % (fn, key))
            continue
        print('%d 0x%x   # refused %d times in runs %s' % (fn, key, cnt, ','.join(sorted(rs))))
        n += 1
    print('# rules: %d' % n)


main(sys.argv[1:])
