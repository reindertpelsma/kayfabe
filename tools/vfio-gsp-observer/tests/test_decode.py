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
