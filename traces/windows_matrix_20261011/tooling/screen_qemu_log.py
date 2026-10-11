#!/usr/bin/env python3
"""Bounded diagnosis summary; cumulative counters never prove correctness."""
from collections import Counter
import json
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
counts = Counter()
values = {k: [] for k in ['birth_refused_cap', 'unreconciled', 'mem_held', 'inval_max_us']}
examples = []
snapshots = []
channel_budget = []
memory_refusals = []
walk_samples = []
refusal_groups = Counter()
for line in path.open(errors='replace'):
    for key in ['birth_refused_cap', 'unreconciled']:
        match = re.search(r'\b' + key + r'=(\d+)', line)
        if match:
            values[key].append(int(match[1]))
    mem = re.search(r'mem\[([^]]*)\]', line)
    held = re.search(r'\bheld=(\d+)', mem[1]) if mem else None
    if held:
        values['mem_held'].append(int(held[1]))
    timing = re.search(r'vat\[invals=(\d+) arrive->clear_avg_us=(\d+) max_us=(\d+)', line)
    if timing:
        values['inval_max_us'].append(int(timing[3]))
    for word in ['OverDeclaredCap', 'birth REFUSED', 'HELD-BY-HOST', 'HELD BY HOST']:
        if word in line:
            counts[word] += 1
            if len(examples) < 8:
                examples.append(line.strip()[:1000])
    # Standalone REFUSED, not status counters or words describing a policy.
    if re.search(r'\bREFUSED\b', line):
        counts['REFUSED_lines'] += 1
        if len(examples) < 8:
            examples.append(line.strip()[:1000])
    if 'REFUSED' in line:
        group = re.search(r'(HOST-ABI REFUSED control 0x[0-9a-f]+|GSP REFUSED fn\d+/0x[0-9a-f]+=0x[0-9a-f]+)', line)
        if group and len(refusal_groups) < 256:
            refusal_groups[group[1]] += 1
    if 'mem t=' in line and 'REFUSED' in line and len(memory_refusals) < 32:
        memory_refusals.append(line.strip()[:1200])
    if 'walk refusal sample:' in line and len(walk_samples) < 32:
        walk_samples.append(line.strip()[:1200])
    if 'channel budget' in line and len(channel_budget) < 4:
        channel_budget.append(line.strip()[:1000])
    if 'PT-SNAP BEGIN' in line:
        snapshots.append(line.strip())
    if 'PT-SNAP tok=' in line and 'methods[' in line:
        counts['snapshot_decoded_segments'] += 1
json.dump(dict(source=str(path), byte_size=path.stat().st_size,
               maxima={k: max(v) if v else None for k, v in values.items()},
               sample_counts={k: len(v) for k, v in values.items()},
               counts=dict(counts), examples=examples, channel_budget=channel_budget,
               control_refusals=dict(refusal_groups), memory_refusals=memory_refusals, walk_samples=walk_samples,
               snapshot_count=len(snapshots), snapshot_first=snapshots[:1], snapshot_last=snapshots[-1:],
               limitations=['invalidate maximum is cumulative across boot, not fault-time latency',
                            'zero counters do not prove absence of stale or wrong mappings',
                            'REFUSED text requires interpretation; counter word occurrence is not refusal',
                            'missing snapshots do not rule out software methods']), sys.stdout, indent=2)
print()
