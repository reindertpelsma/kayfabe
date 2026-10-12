#!/usr/bin/env python3
"""Recorded successful 5080 allocation versus first relay-forward order.

Offline only. A missing forward is UNMEASURED, not proof of no GPU submission.
Log order is evidence; this does not prove universal guest or CPU/GPU ordering.
"""
import gzip
import hashlib
import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
INPUTS = [
    'snapshot-result/run1360-qemu.log.gz',
    'snapshot-result/run1361-qemu.log.gz',
    'late-fifo-89e8ddf8/control/run1495-qemu.log.gz',
    'late-fifo-89e8ddf8/probe/run1510-qemu.log.gz',
    'registration-refusal/result/qemu.log.gz',
]
BIRTH = re.compile(r'chan (0x[0-9a-f]+):(0x[0-9a-f]+) BORN Passthrough: token (0x[0-9a-f]+) -> host (0x[0-9a-f]+)')
OBJECT = re.compile(r'(0x[0-9a-f]+):(0x[0-9a-f]+) NV50_DEFERRED_API on Windows user-work twin host (0x[0-9a-f]+) -> host object (0x[0-9a-f]+)')
REGISTER = re.compile(r'DEFERRED-API (0x5080010[134]) on (0x[0-9a-f]+):(0x[0-9a-f]+) hApiHandle=(0x[0-9a-f]+) cmd=(0x[0-9a-f]+) psz=\d+ -> (0x[0-9a-f]+)')
FORWARD = re.compile(r'chan token (0x[0-9a-f]+) \(host (0x[0-9a-f]+)\) USERD relay: GP_PUT')
reports = []
for name in INPUTS:
    path = ROOT / name
    rows, active, objects_by_owner, missing = [], {}, {}, []
    with gzip.open(path, 'rt', errors='replace') as stream:
        for line_no, text in enumerate(stream, 1):
            birth = BIRTH.search(text) if 'WINDOWS-USER-WORK' in text else None
            if birth:
                client, channel, token, host = birth.groups()
                row = dict(client=client, channel=channel, token=token, host=host,
                           birth_line=line_no, object_alloc_lines=[], first_forward_line=None, registrations=[])
                rows.append(row)
                active[host] = row
            obj = OBJECT.search(text)
            if obj:
                client, handle, host, native = obj.groups()
                row = active.get(host)
                if row is None or row['client'] != client:
                    missing.append(dict(kind='object', line=line_no))
                else:
                    row['object_alloc_lines'].append(line_no)
                    objects_by_owner[(client, handle)] = row
            registration = REGISTER.search(text)
            if registration:
                outer, client, handle, api, inner, status = registration.groups()
                row = objects_by_owner.get((client, handle))
                if row is not None:
                    row['registrations'].append(dict(line=line_no, outer=outer, api=api, inner=inner, status=status))
            forward = FORWARD.search(text)
            if forward:
                token, host = forward.groups()
                row = active.get(host)
                if row is None or row['token'] != token:
                    missing.append(dict(kind='forward', line=line_no))
                elif row['first_forward_line'] is None:
                    row['first_forward_line'] = line_no
    counts = dict(births=len(rows), with_object=0, before_first_forward=0,
                  after_first_forward=0, forwarded_without_logged_object=0,
                  no_logged_forward=0, with_successful_registration=0, with_fifo_update_registration=0)
    for row in rows:
        successful = [r for r in row['registrations'] if int(r['status'], 16) == 0]
        counts['with_successful_registration'] += bool(successful)
        counts['with_fifo_update_registration'] += any(r['inner'] == '0x20801116' for r in successful)
        objects = row['object_alloc_lines']
        counts['with_object'] += bool(objects)
        forward = row['first_forward_line']
        if forward is None:
            counts['no_logged_forward'] += 1
        elif not objects:
            counts['forwarded_without_logged_object'] += 1
        elif objects[0] < forward:
            counts['before_first_forward'] += 1
        else:
            counts['after_first_forward'] += 1
    reports.append(dict(input=name, compressed_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                        counts=counts, unmatched_events=missing, channels=rows))
assert all(not r['unmatched_events'] for r in reports)
output = dict(scope='five retained Windows diagnostic campaigns, not full application matrix',
              reports=reports)
(HERE / 'audit.json').write_text(json.dumps(output, indent=2) + '\n')
for report in reports:
    print(report['input'], report['counts'])
