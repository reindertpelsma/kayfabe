#!/usr/bin/env python3
"""Evidence only: compare separate envelope/body statuses, never raw payloads.

Usage: align_wire_metadata.py NATIVE_SEQUENCE VIRTUAL_SEQUENCE OUTPUT
Function FIFO is an inference for replies whose refused body erased the key.
The successful reply-key conflict count must remain zero for this alignment.
"""
import collections
import csv
import difflib
from pathlib import Path
import sys

def read(path):
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError('input bound exceeded')
    with path.open() as f:
        rows = list(csv.DictReader(f, delimiter='\t'))
    if len(rows) > 100000:
        raise ValueError('row bound exceeded')
    return rows

def pairs(rows):
    statuses = collections.defaultdict(collections.deque)
    for row in rows:
        if row['dir'] == 'status' and int(row['function'], 0) < 0x1000:
            statuses[int(row['function'], 0)].append(row)
    result, conflicts = [], 0
    for row in rows:
        if row['dir'] != 'cmd' or int(row['function'], 0) not in (1, 65, 76, 103):
            continue
        q = statuses[int(row['function'], 0)]
        reply = q.popleft() if q else None
        if reply and reply['control_or_class'] != row['control_or_class'] and reply['rpc_result'] == '0x0':
            conflicts += 1
        result.append((row, reply))
    return result, conflicts

native, virtual, output = map(Path, sys.argv[1:])
na, nc = pairs(read(native))
va, vc = pairs(read(virtual))
if nc or vc:
    raise ValueError(f'successful reply-key conflicts: {nc}/{vc}; alignment refused')
key = lambda p: (int(p[0]['function'], 0), p[0]['control_or_class'])
matcher = difflib.SequenceMatcher(a=list(map(key, na[:500])), b=list(map(key, va)), autojunk=False)
fields = ['native_cmd_seq', 'native_reply_seq', 'virtual_cmd_seq', 'virtual_reply_seq',
          'function', 'request_control_or_class', 'native_envelope', 'native_control_status',
          'virtual_envelope', 'virtual_control_status', 'native_reply_key', 'virtual_reply_key',
          'status_fields_equal', 'pair_basis']
count = different = 0
with output.open('w') as f:
    writer = csv.writer(f, delimiter='\t', lineterminator='\n')
    writer.writerow(fields)
    for block in matcher.get_matching_blocks():
        for i, j in zip(range(block.a, block.a + block.size), range(block.b, block.b + block.size)):
            a, ar = na[i]
            b, br = va[j]
            if not ar or not br:
                continue
            equal = (ar['rpc_result'], ar['control_status']) == (br['rpc_result'], br['control_status'])
            erased = ar['control_or_class'] != a['control_or_class'] or br['control_or_class'] != b['control_or_class']
            basis = 'function FIFO; error body key unavailable' if erased else 'function FIFO; reply keys agree'
            writer.writerow([a['element_sequence'], ar['element_sequence'], b['element_sequence'],
                br['element_sequence'], a['function'], a['control_or_class'], ar['rpc_result'],
                ar['control_status'], br['rpc_result'], br['control_status'], ar['control_or_class'],
                br['control_or_class'], int(equal), basis])
            count += 1
            different += not equal
print(f'aligned={count} different_status_fields={different} successful_key_conflicts={nc}/{vc}')
