#!/usr/bin/env python3
"""Strictly validate a passive reference export, then audit allocation candidates.

Uses compiler-produced public580.65.06 layout.json, not driver analysis.
Additional RPC fields: public595.84 g_rpc-structures.h rpc_alloc_memory_v13_01
and rpc_free_v03_00; NVOS00_PARAMETERS in nvos.h. Memory-list class meanings:
public580.65.06 class/cl84a0.h. Allocation candidates are not lifetime proofs;
queue observations across directions need not be chronological GPU execution.
"""
import argparse
import collections
import json
from pathlib import Path
import struct
import sys

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tools/vfio-gsp-observer'))
import decode

LAYOUT = json.loads((Path(__file__).parent.parent / 'h1-channel-audit/layout.json').read_text())


def u32(b, at=0):
    return struct.unpack_from('<I', b, at)[0]


def fields(b, layout):
    if len(b) != layout['size']:
        raise ValueError('unexpected public structure size')
    return {k: hex(struct.unpack_from('<Q' if k in {'gpFifoOffset', 'userdOffset'} else '<I', b, off)[0])
            for k, off in layout.items() if k != 'size'}


def candidate(records, before, generation, client, handle, classes=None):
    # Same queue direction and generation, latest observed alloc preceding the
    # request. Reject observed free/reuse barriers. Sampling still can omit one.
    found = None
    for index, r in enumerate(records[:before], 1):
        if r.get('generation') != generation or r['direction'] != 'request':
            continue
        a = r.get('allocation', {})
        if a.get('client') == client and a.get('object') == handle:
            found = dict(index=index, allocation=a)
        b = bytes.fromhex(r['payload_hex'])[80:]
        if r['rpc_function'] == 4 and len(b) >= 44 and hex(u32(b)) == client and hex(u32(b, 8)) == handle:
            cls = u32(b, 12)
            found = dict(index=index, memory_list=dict(client=client, object=handle,
                class_id=hex(cls), aperture={0x81: 'SYSTEM', 0x82: 'FBMEM', 0x83: 'OBJECT_RELATIVE'}.get(cls, 'UNKNOWN'),
                length=hex(struct.unpack_from('<Q', b, 32)[0]), page_count=u32(b, 40),
                pte_adjust=u32(b, 20)))
        if r['rpc_function'] == 10 and len(b) >= 16 and hex(u32(b)) == client and hex(u32(b, 8)) == handle:
            found = None
    if found is not None and classes is not None:
        if found.get('allocation', {}).get('class_id') not in classes:
            return None
    return found


def audit(trace):
    records = trace['records']
    updates = []
    for index, r in enumerate(records, 1):
        c = r.get('control', {})
        if r['direction'] != 'request' or c.get('command') not in {'0x50800103', '0x50800104'}:
            continue
        b = bytes.fromhex(c['params_hex'])
        if len(b) < 8 or u32(b, 4) != 0x20801116:
            continue
        if not c['params_complete'] or len(b) != LAYOUT['registration']['size']:
            raise ValueError('incomplete or unknown FIFO registration ABI')
        at = LAYOUT['registration']['api_bundle']
        update = fields(b[at:at + LAYOUT['update']['size']], LAYOUT['update'])
        obj = candidate(records, index - 1, r['generation'], c['client'], c['object'], {'0x5080'})
        parent = obj['allocation']['parent'] if obj else None
        chan = candidate(records, index - 1, r['generation'], c['client'], parent, {'0xc56f'}) if parent else None
        birth = None
        if chan and chan['allocation']['params_complete']:
            birth = fields(bytes.fromhex(chan['allocation']['params_hex']), LAYOUT['channel'])
        memory = candidate(records, index - 1, r['generation'], update['hClient'], update['hUserdMemory'])
        replies = []
        for j, rep in enumerate(records[index:], index + 1):
            q = rep.get('control', {})
            if rep.get('generation') != r['generation'] or rep['direction'] != 'reply':
                continue
            if all(q.get(k) == c[k] for k in ['client', 'object', 'command']) and q.get('params_hex') == c['params_hex']:
                replies.append(dict(index=j, status=q['status'], rpc_status=rep['rpc_status']))
        updates.append(dict(index=index, generation=r['generation'], client=c['client'], object=c['object'],
            api_handle=hex(u32(b)), update=update, deferred_object_candidate=obj,
            channel_birth_candidate=chan, birth=birth, userd_memory_candidate=memory,
            replies=replies, target_matches_parent=parent == update['hChannel'],
            comparison_to_birth=({k: dict(old=birth[k], requested=update[k], changed=birth[k] != update[k])
                 for k in ['hUserdMemory', 'gpFifoEntries', 'gpFifoOffset', 'userdOffset']} if birth else None),
            execution='UNMEASURED: registration is not an executed GPU method'))
    coverage = collections.Counter((r['allocation']['params_complete'], r['allocation']['params_bytes_declared'])
        for r in records if r['direction'] == 'request' and r.get('allocation', {}).get('class_id') == '0xc56f')
    return dict(schema='kayfabe-native-ring-allocation-audit/1', complete=False,
        layout_source=LAYOUT['source'], records=len(records), export=trace['text_export'],
        capture_stats=trace['export_stats'], channel_request_coverage=[dict(complete=k[0], bytes=k[1], count=v) for k,v in coverage.items()],
        fifo_registrations=updates,
        limitations=['Passive sampled capture, limited=true: absence is not nonexecution.',
            'Allocation tails have stable transport/export-hash provenance, outside RPC checksum.',
            'Candidates use observed request order and free barriers, not authoritative ownership/lifetime pins.',
            'No old/new USERD cursor samples, native execution or retirement semantics are captured.',
            'Each comparison is against birth; a preceding registration is not assumed executed.'])


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('trace', type=Path)
    args = ap.parse_args()
    print(json.dumps(audit(decode.parse_jsonl(args.trace)), indent=2))
