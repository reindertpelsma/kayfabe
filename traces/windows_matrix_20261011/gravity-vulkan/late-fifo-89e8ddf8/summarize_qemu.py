#!/usr/bin/env python3
"""Bounded offline/public-log observation. A sample is NOT GPU execution proof."""
import argparse
import collections
import datetime
import hashlib
import json
from pathlib import Path
import re

ap = argparse.ArgumentParser()
ap.add_argument('log', type=Path)
ap.add_argument('out', type=Path)
a = ap.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
size = a.log.stat().st_size
# Explicit archival bound; oversized capture is refused, never silently complete.
if size > 256 * 1024 * 1024:
    raise SystemExit('REFUSED: log exceeds offline 256MiB capture bound')
with a.log.open('rb') as f:
    raw = f.read(size)
lines = raw.decode('utf-8', errors='replace').splitlines()
keys = ['RELAY-PEEK ARMED', 'SW-SUBCH METHOD', 'method 0x200 ',
        'CHANGED after its doorbell', 'RELAY-PEEK BUDGET exhausted',
        'walk capacity ceiling', 'slot pool exhausted', 'OverDeclaredCap', 'birth_refused', 'HELD-BY-HOST', 'REFUSED',
        'GSP phase Running -> Suspending', 'cmd=0x20801116']
counts = {key: sum(key in line for line in lines) for key in keys}
pattern = re.compile(r'RELAY-PEEK|DEFERRED-API.*(?:cmd=0x20801116|trigger)|'
                     r'chan .* BORN |NV50_DEFERRED_API|GPFIFO_SCHEDULE enable=|'
                     r'USERD relay released:|channel budget|OverDeclaredCap|HELD-BY-HOST|walk capacity ceiling|split ticket.*REFUSED')
selected = [(n, line) for n, line in enumerate(lines, 1) if pattern.search(line)]
cap = 10000
(a.out / 'selected-qemu-lines.txt').write_text(''.join(
    f'{n}: {line[:16384]}\n' for n, line in selected[:cap]))
triggers = []
for n, line in enumerate(lines, 1):
    m = re.search(r'chan token (0x[0-9a-f]+).*SW-SUBCH METHOD '
                  r'GP\[(0x[0-9a-f]+)\] word (\d+) subch (\d+) '
                  r'method 0x200 data \[([^]]*)\]', line)
    if m:
        token, gp, word, subch, data = m.groups()
        triggers.append(dict(line=n, guest_token=token, gp_index=gp,
                             word=int(word), subchannel=int(subch),
                             raw_hex_data=data))
maxima = collections.defaultdict(int)
for line in lines:
    if 'phase=' not in line:
        continue
    for key in ['birth_refused_cap', 'unreconciled', 'held', 'named_missed']:
        m = re.search(r'\b' + key + r'=(\d+)', line)
        if m:
            maxima[key] = max(maxima[key], int(m[1]))
    m = re.search(r'arrive->clear_avg_us=\d+ max_us=(\d+)', line)
    if m:
        maxima['invalidate_boot_max_us'] = max(maxima['invalidate_boot_max_us'], int(m[1]))
meta = dict(captured_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
            source=str(a.log), snapshot_bytes=len(raw), source_size_at_open=size,
            prefix_sha256=hashlib.sha256(raw).hexdigest(), line_count=len(lines),
            counts=counts, counter_maxima=dict(maxima),
            selected_count=len(selected), selected_written=min(len(selected), cap),
            selected_truncated=len(selected) > cap,
            candidate_methods_0x200=triggers,
            limits='Guest-byte sample after forwarding, not GPU fetch/execution proof. '
                   'Earlier object bindings/Legacy/arming/post-doorbell races remain. '
                   'Boot-wide invalidate maxima do not identify the fault-time latency.')
(a.out / 'qemu-observation.json').write_text(json.dumps(meta, indent=2) + '\n')
print(json.dumps({k: meta[k] for k in ['source', 'snapshot_bytes', 'counts', 'counter_maxima', 'selected_count', 'selected_truncated']}))
