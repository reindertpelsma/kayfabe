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
        'KF3_WIN_REFUSE_PT_FIFO_UPDATE', 'fifo_update_registration_refused=', 'walk capacity ceiling', 'slot pool exhausted', 'OverDeclaredCap', 'birth_refused', 'HELD-BY-HOST', 'REFUSED',
        'GSP phase Running -> Suspending', 'cmd=0x20801116']
counts = {key: sum(key in line for line in lines) for key in keys}
pattern = re.compile(r'KF3_WIN_REFUSE_PT_FIFO_UPDATE|fifo_update_registration_refused=|RELAY-PEEK|DEFERRED-API.*(?:cmd=0x20801116|trigger)|'
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
                  r'method 0x200 data (Some\([0-9a-f]+\)|\[[^]]*\]|None)', line)
    if m:
        token, gp, word, subch, data = m.groups()
        form = data.split('(', 1)[0] if data.startswith('Some(') else ('list' if data.startswith('[') else 'None')
        data = data[5:-1] if form == 'Some' else (data[1:-1] if form == 'list' else '')
        triggers.append(dict(line=n, guest_token=token, gp_index=gp,
                             word=int(word), subchannel=int(subch),
                             raw_hex_data=data, logged_data_form=form))
maxima = collections.defaultdict(int)
for line in lines:
    if 'phase=' not in line:
        continue
    for key in ['birth_refused_cap', 'unreconciled', 'held', 'named_missed', 'fifo_update_registration_refused']:
        m = re.search(r'\b' + key + r'=(\d+)', line)
        if m:
            maxima[key] = max(maxima[key], int(m[1]))
    m = re.search(r'arrive->clear_avg_us=\d+ max_us=(\d+)', line)
    if m:
        maxima['invalidate_boot_max_us'] = max(maxima['invalidate_boot_max_us'], int(m[1]))
# Decode only the logged prefix with the compiler-derived public layout.
# This cannot establish GPU fetch, binding before this window, or execution.
layout_path = Path(__file__).resolve().parent.parent / 'h1-channel-audit' / 'layout.json'
# Remote standalone deployment passes the public layout beside the script.
if not layout_path.exists():
    layout_path = Path(__file__).with_name('kf-h1-layout.json')
armed = []
layout = json.loads(layout_path.read_text()) if layout_path.exists() else None
import struct
for n, line in enumerate(lines, 1):
    m = re.search(r'chan token (0x[0-9a-f]+) \(host (0x[0-9a-f]+)\) '
                  r'RELAY-PEEK ARMED.* on (0x[0-9a-f]+):(0x[0-9a-f]+) '
                  r'api=(0x[0-9a-f]+) cmd=(0x[0-9a-f]+) '
                  r'GP=\[(0x[0-9a-f]+),(0x[0-9a-f]+)\) entries=(\d+) t=([0-9.]+)', line)
    if not m:
        continue
    tok, host, client, obj, api, cmd, prev, put, entries, stamp = m.groups()
    record = dict(line=n, guest_token=tok, host_token=host, client=client,
                  object=obj, api=api, command=cmd, previous_put=prev, put=put,
                  old_ring_entries=int(entries), log_t=float(stamp))
    for later in lines[n:n+8]:
        b = re.search(r'chan token ' + re.escape(tok) +
                      r' RELAY-PEEK registration bundle prefix.*: \[([^]]*)\]', later)
        if not b:
            continue
        raw_bundle = bytes(int(x.strip(), 16) for x in b[1].split(',') if x.strip())
        record['logged_prefix_bytes'] = len(raw_bundle)
        if layout and len(raw_bundle) >= layout['update']['size']:
            record['update_fields_public_layout'] = {
                key: hex(struct.unpack_from('<Q' if key in {'gpFifoOffset', 'userdOffset'}
                                             else '<I', raw_bundle, off)[0])
                for key, off in layout['update'].items() if key != 'size'}
        else:
            record['update_fields_public_layout'] = 'UNMEASURED: missing layout/short prefix'
        break
    record['matching_sample_lines'] = [t['line'] for t in triggers if
        t['line'] > n and t['guest_token'] == tok and
        any(int(v.strip(), 16) == int(api, 16) for v in t['raw_hex_data'].split(',') if v.strip())]
    armed.append(record)
meta = dict(captured_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
            source=str(a.log), snapshot_bytes=len(raw), source_size_at_open=size,
            prefix_sha256=hashlib.sha256(raw).hexdigest(), line_count=len(lines),
            counts=counts, counter_maxima=dict(maxima),
            selected_count=len(selected), selected_written=min(len(selected), cap),
            selected_truncated=len(selected) > cap,
            candidate_methods_0x200=triggers, armed_registrations=armed,
            public_layout_source=layout["source"] if layout else "UNMEASURED",
            public_layout_sha256=hashlib.sha256(layout_path.read_bytes()).hexdigest() if layout else None,
            limits='Guest-byte sample after forwarding, not GPU fetch/execution proof. '
                   'Earlier object bindings/Legacy/arming/post-doorbell races remain. '
                   'Boot-wide invalidate maxima do not identify the fault-time latency.')
(a.out / 'qemu-observation.json').write_text(json.dumps(meta, indent=2) + '\n')
print(json.dumps({k: meta[k] for k in ['source', 'snapshot_bytes', 'counts', 'counter_maxima', 'selected_count', 'selected_truncated']}))
