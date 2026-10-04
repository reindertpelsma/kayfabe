#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Decode passive GSP observations; never label them complete boot traces."""
import argparse
import collections
import json
from pathlib import Path
import struct
import sys

FILE = struct.Struct('<4I2Q2I3Q')
RECORD = struct.Struct('<4I2Q8I')
QUERY = 0x2080121f


class InvalidTrace(ValueError):
    pass


def parse(blob):
    if len(blob) < FILE.size:
        raise InvalidTrace('truncated file header')
    h = FILE.unpack_from(blob)
    if h[:4] != (0x5457474b, 1, 64, 64) or h[6] != 1 or any(h[7:]) or not h[4]:
        raise InvalidTrace('unsupported file header or missing sampled flag')
    records = []
    at = FILE.size
    prior = {}
    while at < len(blob):
        if len(blob) - at < RECORD.size:
            raise InvalidTrace('truncated record header')
        r = RECORD.unpack_from(blob, at)
        magic, header, size, direction, qpc, table, seq, rpc_seq, fn, result, flags, gap, version, reserved = r
        if magic != 0x5247474b or header != 64 or not 80 <= size <= 65536 or size % 8 or direction > 1:
            raise InvalidTrace('invalid record framing')
        if flags & ~7 or not flags & 1 or reserved or bool(flags & 4) != bool(gap):
            raise InvalidTrace('invalid record flags')
        at += RECORD.size
        if size > len(blob) - at:
            raise InvalidTrace('truncated payload')
        payload = blob[at:at + size]
        at += size
        if any(payload[:32]) or version != 0x03000000:
            raise InvalidTrace('encrypted or unsupported RPC header version')
        checksum = 0
        for (word,) in struct.iter_unpack('<I', payload):
            checksum ^= word
        if checksum:
            raise InvalidTrace('message checksum mismatch')
        e_seq, pages = struct.unpack_from('<II', payload, 36)
        rpc = struct.unpack_from('<8I', payload, 48)
        if (rpc[0], rpc[1], rpc[3], rpc[4], rpc[6], e_seq) != (version, 0x43505256, fn, result, rpc_seq, seq):
            raise InvalidTrace('record metadata disagrees with payload')
        length = rpc[2]
        if length < 32 or (48 + length + 7) & ~7 != size or pages != (48 + length + 4095) // 4096:
            raise InvalidTrace('message length mismatch')
        key = (table, direction)
        if key in prior and not flags & 2:
            delta = (seq - prior[key]) & 0xffffffff
            if not 0 < delta < 0x80000000:
                raise InvalidTrace('duplicate or reversed sequence')
            # Overflowed driver output may itself lose records: surface this too.
            missing = max(gap, delta - 1)
        else:
            missing = gap
        prior[key] = seq
        item = dict(direction='request' if direction == 0 else 'reply', table_pa=hex(table),
                    observed_qpc=qpc, queue_sequence=seq, rpc_sequence=rpc_seq,
                    rpc_function=fn, rpc_status=hex(result), missing_before=missing,
                    prefix_unknown=bool(flags & 2), payload_hex=payload.hex())
        if fn == 76:
            if length < 72:
                raise InvalidTrace('short RM_CONTROL envelope')
            client, obj, command, status, params_size = struct.unpack_from('<5I', payload, 80)
            if params_size > length - 72:
                raise InvalidTrace('RM_CONTROL declared params exceed payload')
            item['control'] = dict(client=hex(client), object=hex(obj), command=hex(command),
                                   status=hex(status), params_hex=payload[120:120 + params_size].hex())
            if command == QUERY:
                if params_size != 40:
                    raise InvalidTrace('GFX_POOL_QUERY_SIZE layout differs from the 40-byte known ABI')
                fields = struct.unpack_from('<II4Q', payload, 120)
                item['gfx_pool'] = dict(zip(('maxSlots', 'slotStride', 'ctrlStructSize', 'ctrlStructAlign',
                                             'poolSize', 'poolAlign'), fields))
        records.append(item)
    return dict(schema='kayfabe-gsp-observer/1', complete=False, qpc_frequency=h[4],
                note='QPC is observation time, including retained history; it is not submission time.', records=records)


def summarize(trace, stats=None):
    records = trace['records']
    queries = [r for r in records if 'gfx_pool' in r]
    grouped = collections.defaultdict(list)
    for r in queries:
        c = r['control']
        grouped[(r['table_pa'], r['rpc_sequence'], c['client'], c['object'])].append(r)
    pairs = []
    for group in grouped.values():
        req = [r for r in group if r['direction'] == 'request']
        rep = [r for r in group if r['direction'] == 'reply']
        if len(req) == len(rep) == 1 and req[0]['gfx_pool']['maxSlots'] == rep[0]['gfx_pool']['maxSlots']:
            pairs.append(dict(request=req[0]['gfx_pool'], reply=rep[0]['gfx_pool'],
                              successful=rep[0]['rpc_status'] == '0x0' and rep[0]['control']['status'] == '0x0',
                              rpc_sequence=req[0]['rpc_sequence'], table_pa=req[0]['table_pa']))
    return dict(schema=trace['schema'], complete=False, records=len(records),
                observed_missing=sum(r['missing_before'] for r in records),
                driver_stats=stats, gfx_pool_observations=queries, unambiguous_query_pairs=pairs,
                warning='Passive samples cannot prove completeness or cross-GPU behavior. Retained history may predate collector start.')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('trace', type=Path)
    ap.add_argument('--all-records', action='store_true')
    ap.add_argument('--require-query-pair', action='store_true', help='exit 4 unless an unambiguous successful query request/reply exists')
    args = ap.parse_args()
    try:
        trace = parse(args.trace.read_bytes())
        sidecar = Path(str(args.trace) + '.stats.json')
        stats = json.loads(sidecar.read_text()) if sidecar.exists() else None
        output = summarize(trace, stats)
        if args.all_records:
            output['observations'] = trace['records']
        print(json.dumps(output, indent=2))
        if args.require_query_pair and not any(p['successful'] for p in output['unambiguous_query_pairs']):
            return 4
        return 0
    except (OSError, ValueError, struct.error) as exc:
        print(f'REFUSED: {exc}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
