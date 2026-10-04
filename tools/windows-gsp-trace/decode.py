#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Decode passive GSP observations; never label them complete boot traces."""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import struct
import sys

FILE = struct.Struct('<4I2Q2I3Q')
RECORD = struct.Struct('<4I2Q8I')
QUERY = 0x2080121f


class InvalidTrace(ValueError):
    pass


def parse_jsonl(path, max_bytes=64 * 1024 * 1024):
    """Verify a bounded text-data export, then reuse all binary ABI checks."""
    blob = bytearray()
    footer = None
    count = 0
    fields = ('magic', 'header_bytes', 'payload_bytes', 'direction', 'qpc', 'table_pa',
              'queue_sequence', 'rpc_sequence', 'rpc_function', 'rpc_result', 'flags',
              'missing_before', 'rpc_version', 'reserved')
    try:
        with path.open(encoding='utf-8-sig') as stream:
            line_number = 0
            while line := stream.readline(140001):
                line_number += 1
                if len(line) > 140000:
                    raise InvalidTrace('text record exceeds 140000 characters')
                row = json.loads(line)
                if not isinstance(row, dict) or footer is not None:
                    raise InvalidTrace('invalid row or data after export footer')
                if line_number == 1:
                    if row.get('schema') != 'kayfabe-gsp-text/1' or row.get('kind') != 'header' or row.get('capture_complete') is not False:
                        raise InvalidTrace('unsupported text export header')
                    values = [row[k] for k in ('magic', 'version', 'header_bytes', 'record_header_bytes', 'qpc_frequency', 'started_qpc', 'flags', 'reserved')]
                    values += row['reserved2']
                    if any(type(value) is not int for value in values):
                        raise InvalidTrace('header fields must be integers')
                    blob += FILE.pack(*values)
                elif row.get('kind') == 'record':
                    values = [row[k] for k in fields]
                    encoded = row['payload_hex']
                    if any(type(value) is not int for value in values) or not isinstance(encoded, str) or len(encoded) != row['payload_bytes'] * 2 or len(encoded) > 131072 or any(c not in '0123456789abcdef' for c in encoded):
                        raise InvalidTrace('invalid text record fields or payload hex')
                    if len(blob) + RECORD.size + len(encoded) // 2 > max_bytes:
                        raise InvalidTrace('text capture exceeds decoded byte limit')
                    blob += RECORD.pack(*values) + bytes.fromhex(encoded)
                    count += 1
                elif row.get('kind') == 'footer':
                    footer = row
                else:
                    raise InvalidTrace('unknown text record kind')
        if footer is None or footer.get('file_export_complete') is not True or footer.get('capture_complete') is not False or footer.get('records') != count or footer.get('source_bytes') != len(blob) or footer.get('source_sha256') != hashlib.sha256(blob).hexdigest():
            raise InvalidTrace('missing or inconsistent export footer/hash')
        trace = parse(blob)
        trace['export_stats'] = footer.get('driver_stats')
        return trace
    except (KeyError, TypeError, struct.error) as error:
        raise InvalidTrace(f'invalid text export structure: {error}') from error


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
            observed_size = min(params_size, length - 72)
            item['control'] = dict(client=hex(client), object=hex(obj), command=hex(command),
                                   status=hex(status), params_bytes_declared=params_size,
                                   params_bytes_observed=observed_size, params_complete=observed_size == params_size,
                                   params_hex=payload[120:120 + observed_size].hex())
            if command == QUERY:
                if params_size != 40:
                    raise InvalidTrace('GFX_POOL_QUERY_SIZE layout differs from the 40-byte known ABI')
                if observed_size != params_size:
                    records.append(item)
                    continue  # retain a fragment, never read or fabricate absent bytes
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
    ap.add_argument('--jsonl', action='store_true', help='read text export (also automatic for .jsonl paths)')
    ap.add_argument('--max-input-mib', type=int, choices=range(1, 1025), default=64, metavar='1..1024', help='maximum decoded text-export bytes, default 64 MiB')
    args = ap.parse_args()
    try:
        trace = parse_jsonl(args.trace, args.max_input_mib * 1048576) if args.jsonl or args.trace.suffix.lower() == '.jsonl' else parse(args.trace.read_bytes())
        sidecar = Path(str(args.trace) + '.stats.json')
        stats = json.loads(sidecar.read_text()) if sidecar.exists() else trace.get('export_stats')
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
