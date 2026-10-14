#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location('decoder', Path(__file__).resolve().parents[1] / 'decode.py')
decoder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(decoder)

def query(generation, direction):
    return dict(generation=generation, direction=direction, table_pa='0x1000',
                rpc_sequence=0, control=dict(client='0x1', object='0x2', status='0x0'),
                gfx_pool=dict(maxSlots=16), missing_before=0, rpc_status='0x0')

# Same address, handles and sequence zero must not pair across queue lifetimes.
trace = dict(schema='test', records=[query(1, 'request'), query(2, 'reply')])
assert not decoder.summarize(trace)['unambiguous_query_pairs']
trace['records'].append(query(2, 'request'))
pairs = decoder.summarize(trace)['unambiguous_query_pairs']
assert len(pairs) == 1 and pairs[0]['generation'] == 2
trace['records'].append(query(2, 'request'))
assert not decoder.summarize(trace)['unambiguous_query_pairs']
print('Generation reuse and ambiguous query pairing tests passed')

# Allocation parameters can extend beyond the RPC checksum span while remaining
# inside the transport's committed element. Preserve that distinction explicitly.
import struct

def alloc_blob(params_size=368, capture_tail=True):
    payload = bytearray(112 + (params_size if capture_tail else 0))
    payload += bytes((-len(payload)) % 8)
    struct.pack_into('<II', payload, 36, 0, 1)
    struct.pack_into('<8I', payload, 48, 0x03000000, 0x43505256, 64, 103, 0, 0, 0, 0)
    struct.pack_into('<7I', payload, 80, 1, 2, 3, 0xc56f, 0, params_size, 0)
    if capture_tail:
        payload[112:] = b'\xa5' * (len(payload) - 112)
    checksum = 0
    for (word,) in struct.iter_unpack('<I', payload[:112]):
        checksum ^= word
    struct.pack_into('<I', payload, 32, checksum)
    header = decoder.FILE.pack(0x5457474b, 1, 64, 64, 1000000000, 0, 1, 0, 0, 0, 0)
    record = decoder.RECORD.pack(0x5247474b, 64, len(payload), 0, 1, 0x1000, 0, 0, 103, 0,
                                 3 | (8 if capture_tail else 0), 0, 0x03000000, 0)
    return header + record + payload

allocation = decoder.parse(alloc_blob())['records'][0]['allocation']
assert allocation['params_bytes_declared'] == 368
assert allocation['params_bytes_observed'] == 368
assert allocation['params_complete'] is True
assert allocation['rpc_checksum_covers_params'] is False
assert allocation['params_hex'] == 'a5' * 368
fragment = decoder.parse(alloc_blob(capture_tail=False))['records'][0]['allocation']
assert fragment['params_bytes_observed'] == 0 and not fragment['params_complete']
for mutation in ('wrong_function', 'short_tail', 'bad_prefix_checksum', 'out_of_element'):
    data = bytearray(alloc_blob())
    if mutation == 'wrong_function':
        struct.pack_into('<I', data, 64 + 40, 72)
    elif mutation == 'short_tail':
        struct.pack_into('<I', data, 64 + 8, 112)
        del data[64 + 64 + 112:]
    elif mutation == 'bad_prefix_checksum':
        data[64 + 64 + 80] ^= 1
    else:
        # Checksum otherwise intact; metadata alone names a too-small extent.
        struct.pack_into('<I', data, 64 + 64 + 40, 0)
    try:
        decoder.parse(data)
        assert False, mutation
    except decoder.InvalidTrace:
        pass
print('Bounded allocation tail, incomplete params, checksum provenance and malformed records passed')
