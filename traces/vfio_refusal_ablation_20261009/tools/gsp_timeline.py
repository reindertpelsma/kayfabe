#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""gsp_timeline.py GSP_JSONL [MONO_OFFSET_NS] -- records per 10 s of observer time, the last request/reply, and the
last 12 requests. `qpc` is host CLOCK_MONOTONIC ns; MONO_OFFSET_NS (CLOCK_REALTIME - CLOCK_MONOTONIC, from the boot's
click.txt `clock_offset_realtime_minus_monotonic_ns`) turns it into host UTC. Prints a table; reads, never writes."""
import collections
import datetime
import json
import struct
import sys

path = sys.argv[1]
off = int(sys.argv[2]) if len(sys.argv) > 2 else 0
recs = []
start = None
for line in open(path, encoding='utf-8-sig'):
    try:
        r = json.loads(line)
    except ValueError:
        continue
    if r.get('kind') == 'header':
        start = r['started_qpc']
    if r.get('kind') != 'record':
        continue
    p = bytes.fromhex(r['payload_hex'])
    i = p.find(b'VRPC')
    fn = struct.unpack_from('<I', p, i + 8)[0] if i >= 4 and len(p) >= i + 12 else -1
    key = None
    if fn == 76 and len(p) >= i + 28 + 12:
        key = struct.unpack_from('<I', p, i + 28 + 8)[0]
    elif fn == 103 and len(p) >= i + 28 + 16:
        key = struct.unpack_from('<I', p, i + 28 + 12)[0]
    recs.append((r['qpc'], r['direction'], fn, key, r['trigger']))


def utc(q):
    if not off:
        return '+%.1fs' % ((q - start) / 1e9)
    return datetime.datetime.fromtimestamp((q + off) / 1e9, datetime.timezone.utc).strftime('%H:%M:%S.%f')[:-3]


b = collections.Counter(int((q - start) / 1e10) for q, *_ in recs)
print('records', len(recs), 'first', utc(recs[0][0]), 'last', utc(recs[-1][0]), 'span %.1fs' % ((recs[-1][0] - start) / 1e9))
print('per-10s buckets (bucket start, records):', ' '.join('%d:%d' % (k * 10, v) for k, v in sorted(b.items())))
print('last 12 records (time, dir, fn, key):')
for q, d, fn, key, trg in recs[-12:]:
    print('  ', utc(q), 'req' if d == 0 else 'rep', fn, hex(key) if key is not None else '-', 'trigger', trg)
