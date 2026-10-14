#!/usr/bin/env python3
"""Offline public RPC measurement summary; no captured values enter the product.
Uses the existing vfio_ctrl_window.py open RPC header/control contract.
Whole-boot counts cannot identify the app's initiating method or prove no reset.
"""
import collections
import gzip
import json
from pathlib import Path
import struct
import sys

source = Path(sys.argv[1])
functions = collections.Counter()
missing = collections.Counter()
generations = set()
commands = {}
clients = {}
malformed = 0
records = 0
complete = None
with gzip.open(source, 'rt') as stream:
    for line in stream:
        r = json.loads(line)
        if r.get('kind') == 'header':
            complete = r.get('capture_complete')
        if r.get('kind') != 'record':
            continue
        records += 1
        direction = r['direction']
        functions[(direction, r['rpc_function'])] += 1
        missing[direction] += r.get('missing_before', 0)
        generations.add(r['generation'])
        p = bytes.fromhex(r['payload_hex'])
        i = p.find(b'VRPC')
        if i < 4 or len(p) < i - 4 + 32:
            malformed += 1; continue
        version, sig, length, fn, result, reply, sequence, spare = struct.unpack_from('<8I', p, i - 4)
        if length < 32 or i - 4 + length > len(p) or fn != r['rpc_function']:
            malformed += 1; continue
        b = p[i - 4 + 32:i - 4 + length]
        if fn == 76 and len(b) >= 40:
            hc, ho, cmd, status, size = struct.unpack_from('<5I', b)
            key = f'{cmd:08x}'
            c = commands.setdefault(key, dict(requests=0, replies=0, reply_status={}, param_sizes={}, clients={}))
            name = 'requests' if direction == 0 else 'replies'
            c[name] += 1
            if direction == 1:
                k = f'{status:#x}'; c['reply_status'][k] = c['reply_status'].get(k, 0) + 1
            k = str(size); c['param_sizes'][k] = c['param_sizes'].get(k, 0) + 1
            k = f'{hc:08x}'; c['clients'][k] = c['clients'].get(k, 0) + 1
            e = clients.setdefault(k, dict(first_qpc=r['qpc'], last_qpc=r['qpc'], requests={}, alloc_classes={}))
            e['last_qpc'] = max(e['last_qpc'], r['qpc'])
            if direction == 0:
                e['requests'][key] = e['requests'].get(key, 0) + 1
        elif fn == 103 and direction == 0 and len(b) >= 16:
            hc, parent, handle, cls = struct.unpack_from('<4I', b)
            e = clients.setdefault(f'{hc:08x}', dict(first_qpc=r['qpc'], last_qpc=r['qpc'], requests={}, alloc_classes={}))
            e['last_qpc'] = max(e['last_qpc'], r['qpc'])
            k = f'{cls:08x}'; e['alloc_classes'][k] = e['alloc_classes'].get(k, 0) + 1
output = dict(source=str(source), records=records, malformed_records=malformed,
              capture_complete_header=complete, generations=sorted(generations),
              reported_missing={str(k): v for k, v in missing.items()},
              functions={f'dir{k[0]}:fn{k[1]}': v for k, v in sorted(functions.items())},
              controls=commands, clients=clients,
              limitations=['whole-boot counts include setup and other guest processes',
                           'observer generations are not a measured GSP recovery count',
                           'no initiating GPU method or causal mapping is identified'])
Path(sys.argv[2]).write_text(json.dumps(output, indent=2) + '\n')
print(json.dumps({k: output[k] for k in ['records','malformed_records','capture_complete_header','generations','reported_missing']}))
for k in ['50800103','50800104','20801116']:
    print(k, json.dumps(commands.get(k)))
