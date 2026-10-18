#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sn_ddi_cmp.py DIR TAG_A TAG_B [N]: side-by-side per-DDI/function statistics from DIR/TAG/TAG_ddi.json (sn_ddi.py), sorted by A's total time."""
import json, sys
d, a, b = sys.argv[1:4]; n = int(sys.argv[4]) if len(sys.argv) > 4 else 16
A = json.load(open(f'{d}/{a}/{a}_ddi.json'))['functions']; B = json.load(open(f'{d}/{b}/{b}_ddi.json'))['functions']
print(f'| function | calls/s {a} | calls/s {b} | median us {a} | median us {b} | p99 us {a} | p99 us {b} | total ms/s {a} | total ms/s {b} |')
print('|---|---|---|---|---|---|---|---|---|')
for f in sorted(A, key=lambda k: -A[k]['total_ms_per_s'])[:n]:
    x = A[f]; y = B.get(f, {})
    print(f"| {f} | {x['calls_per_s']} | {y.get('calls_per_s', '-')} | {x['med_us']} | {y.get('med_us', '-')} | {x['p99_us']} | {y.get('p99_us', '-')} | {x['total_ms_per_s']} | {y.get('total_ms_per_s', '-')} |")
