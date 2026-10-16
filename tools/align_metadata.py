#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Evidence-only ordered API/status comparison; never compares reply bodies."""
import collections
import csv
import difflib
from pathlib import Path
import sys

native, virtual, output = map(Path, sys.argv[1:])
def rows(path):
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError('metadata input exceeds bound')
    with path.open() as f:
        result = list(csv.DictReader(f, delimiter='\t'))
    if len(result) > 100000:
        raise ValueError('metadata row bound exceeded')
    return result
def key(r):
    return int(r['function'], 0), r['control_or_class']
n, v = rows(native), rows(virtual)
pending = collections.defaultdict(collections.deque)
for r in n:
    if r['dir'] == 'status' and int(r['function'], 0) < 0x1000:
        pending[key(r)].append(r)
commands = []
for r in n:
    if r['dir'] != 'cmd':
        continue
    q = pending[key(r)]
    status = q.popleft() if q else None
    r = dict(r)
    r['status_sequence'] = status['element_sequence'] if status else '-'
    r['effective_result'] = 'unknown'
    if status:
        control = 0 if status['control_status'] == '-' else int(status['control_status'], 0)
        r['effective_result'] = hex(control or int(status['rpc_result'], 0))
    commands.append(r)
# Bound startup alignment. Matching blocks preserve API order. Missing calls
# and changes in repetition remain explicit; equality says nothing about bodies.
kinds = (1, 65, 76, 103)
a = [r for r in commands if key(r)[0] in kinds][:500]
b = [r for r in v if key(r)[0] in kinds]
matcher = difflib.SequenceMatcher(a=list(map(key, a)), b=list(map(key, b)), autojunk=False)
fields = ['native_command_sequence', 'native_status_sequence', 'function',
          'control_or_class', 'native_effective_result', 'virtual_ordinal',
          'virtual_effective_result', 'status_equal', 'matching_block_length',
          'next_native_function', 'next_native_control_or_class']
with output.open('w') as f:
    writer = csv.writer(f, delimiter='\t', lineterminator='\n')
    writer.writerow(fields)
    count = differing = 0
    for block in matcher.get_matching_blocks():
        for i, j in zip(range(block.a, block.a + block.size), range(block.b, block.b + block.size)):
            x, y = a[i], b[j]
            following = next((r for r in commands if int(r['element_sequence']) > int(x['element_sequence'])), None)
            equal = x['effective_result'] == y['effective_result']
            writer.writerow([x['element_sequence'], x['status_sequence'], x['function'],
                x['control_or_class'], x['effective_result'], y['ordinal'],
                y['effective_result'], int(equal), block.size,
                following['function'] if following else '-',
                following['control_or_class'] if following else '-'])
            count += 1
            differing += not equal
print(f'native_commands={len(commands)} virtual_rows={len(v)} aligned_calls={count} status_differences={differing}')
print(f'unpaired_native_replies={sum(map(len, pending.values()))}')
