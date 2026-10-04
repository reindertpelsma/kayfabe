#!/usr/bin/env python3
"""Decode only the source-defined Windows context forms relevant to v3's twin policy."""
import argparse
import collections
import gzip
import hashlib
import json
from pathlib import Path
import struct


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('capture', type=Path)
    ap.add_argument('output', type=Path)
    args = ap.parse_args()
    data = gzip.decompress(args.capture.read_bytes())
    assert hashlib.sha256(data).hexdigest() == '9d75e47b8ec5dd837dc64e14847779d62a16f14d92169ede8f9b62681b93de29'
    contexts, zbc_allocations, zbc_controls = [], collections.Counter(), collections.Counter()
    for line, text in enumerate(data.decode().splitlines(), 1):
        r = json.loads(text)
        if r.get('kind') != 'record':
            continue
        b = bytes.fromhex(r['payload_hex'])
        if r['rpc_function'] == 103 and struct.unpack_from('<I', b, 92)[0] == 0x9096:
            zbc_allocations[str(r['direction'])] += 1
        if r['rpc_function'] != 76:
            continue
        cmd = struct.unpack_from('<I', b, 88)[0]
        if cmd >> 16 == 0x9096:
            zbc_controls[str(r['direction'])] += 1
        if r['direction'] != 0 or cmd not in (0x2080012b, 0x50800101):
            continue
        size = struct.unpack_from('<I', b, 96)[0]
        p = b[120:]
        assert len(p) == size == (584 if cmd == 0x50800101 else 560)
        out = dict(line=line, queue_sequence=r['queue_sequence'], outer=f'0x{cmd:08x}')
        if cmd == 0x50800101:
            handle, cmd, flags, client_va, device_va = struct.unpack_from('<5I', p)
            out['wrapper'] = dict(handle=hex(handle), flags=hex(flags), client_va=hex(client_va), device_va=hex(device_va))
            p = p[24:]
        engine, client, chid, chan_client, obj, virt_mem = struct.unpack_from('<6I', p)
        out.update(command=f'0x{cmd:08x}', engine=engine, client=hex(client), chid=chid,
                   channel_client=hex(chan_client), object=hex(obj), virtual_memory_handle=hex(virt_mem))
        if cmd == 0x2080012b:
            va, size, count = struct.unpack_from('<QQI', p, 24)
            assert count <= 16
            out.update(virtual_address=hex(va), size=size, entry_count=count, entries=[])
            for i in range(count):
                pa, va, size, attr, bid, init, nonmapped = struct.unpack_from('<QQQIHBB', p, 48 + 32 * i)
                out['entries'].append(dict(buffer_id=bid, phys=hex(pa), va=hex(va), size=size,
                                           phys_attr=hex(attr), initialize=init, nonmapped=nonmapped))
        else:
            assert cmd == 0x2080012d
            out.update(physical_address=hex(struct.unpack_from('<Q', p, 24)[0]),
                       phys_attr=hex(struct.unpack_from('<I', p, 32)[0]),
                       size=struct.unpack_from('<Q', p, 48)[0])
        contexts.append(out)
    result = dict(schema=1, source_abi='OGKM 580.65.06 ctrl2080gpu.h and ctrl5080.h',
                  capture_text_sha256=hashlib.sha256(data).hexdigest(), contexts=contexts,
                  zbc_allocations=dict(zbc_allocations), zbc_control_records=dict(zbc_controls),
                  summary=dict(direct_promotes=sum('wrapper' not in r for r in contexts),
                               deferred_promotes=sum('wrapper' in r and r['command'] == '0x2080012b' for r in contexts),
                               deferred_initializes=sum(r['command'] == '0x2080012d' for r in contexts)))
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    print(json.dumps(result['summary'], sort_keys=True))


if __name__ == '__main__':
    main()
