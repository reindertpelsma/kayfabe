#!/usr/bin/env python3
"""Offline H1 evidence audit. Registration is never treated as a GPU trigger.

RPC layouts: public OGKM 580.65.06 g_rpc-structures.h, ctrl5080.h,
alloc_channel.h; field offsets are emitted by layout.c, not reverse engineered.
Run from any directory; all inputs are already published beside this evidence.
"""
import collections
import gzip
import hashlib
import json
from pathlib import Path
import re
import struct

HERE = Path(__file__).resolve().parent
GRAVITY = HERE.parent
layout = json.loads((HERE / 'layout.json').read_text())
log_path = GRAVITY / 'snapshot-result/run1361-qemu.log.gz'
lines = list(gzip.open(log_path, 'rt', errors='replace'))
registration_re = re.compile(
    r'DEFERRED-API (0x[0-9a-f]+) on (0x[0-9a-f]+):(0x[0-9a-f]+) '
    r'hApiHandle=(0x[0-9a-f]+) cmd=(0x[0-9a-f]+) psz=(\d+) -> (0x[0-9a-f]+)')
registrations = []
for n, line in enumerate(lines, 1):
    match = registration_re.search(line)
    if match:
        outer, client, obj, handle, cmd, size, status = match.groups()
        registrations.append(dict(line=n, outer=outer, client=client, object=obj,
                                  api_handle=handle, inner=cmd,
                                  param_size=int(size), status=status))

channels = []
selected = set()
for client, obj in sorted({(r['client'], r['object']) for r in registrations}):
    own_regs = [r for r in registrations if (r['client'], r['object']) == (client, obj)]
    alloc = next(((n, s) for n, s in enumerate(lines, 1)
                  if f'{client}:' in s and
                  (f'NV50_DEFERRED_API {obj} ADMITTED' in s or
                   f'{client}:{obj} NV50_DEFERRED_API' in s)), None)
    birth = next(((n, s) for n, s in enumerate(lines, 1)
                  if f'chan {client}:' in s and ' BORN ' in s), None)
    if birth is None or alloc is None:
        raise ValueError(f'missing channel/5080 allocation for {client}:{obj}')
    b = re.search(r'chan \S+:(0x[0-9a-f]+) BORN (\w+): token (0x[0-9a-f]+) -> host (0x[0-9a-f]+)', birth[1])
    if b is None:
        raise ValueError('unrecognized birth format')
    handle, kind, token, host = b.groups()
    schedule = next(((n, s) for n, s in enumerate(lines, 1)
                     if n >= birth[0] and f'{client}:' in s and
                     'GPFIFO_SCHEDULE enable=true' in s), None)
    relay = [(n, s) for n, s in enumerate(lines, 1)
             if f'chan token {token} (host {host}) USERD relay: GP_PUT' in s]
    trigger = [(n, s) for n, s in enumerate(lines, 1)
               if 'DEFERRED-API trigger #' in s and f'on {client}:{obj} ' in s]
    selected.update([birth[0], alloc[0], *(r['line'] for r in own_regs)])
    selected.update(n for n, _ in [*relay[:1], *relay[-1:], *trigger[:1]])
    if schedule:
        selected.add(schedule[0])
    release = next(((n, s) for n, s in enumerate(lines, 1)
                    if f'chan token {token} (host {host}) USERD relay released:' in s), None)
    if release:
        selected.add(release[0])
    channels.append(dict(client=client, channel=handle, deferred_object=obj,
                         classification=kind, guest_token=token, host_token=host,
                         channel_birth_line=birth[0], object_alloc_line=alloc[0],
                         registrations=own_regs,
                         schedule_enable_line=schedule[0] if schedule else 'UNMEASURED',
                         scheduled_at_birth='scheduled_at_birth=true' in birth[1],
                         token_handout='UNMEASURED: birth records assignment, not GET_WORK_SUBMIT_TOKEN reply',
                         first_forwarded_relay_line=relay[0][0] if relay else 'UNMEASURED',
                         first_logged_trigger_line=trigger[0][0] if trigger else 'UNMEASURED',
                         trigger_log_coverage='bounded logging; absence is not proof of absence',
                         direct_guest_doorbell='UNMEASURED; relay forwarding is measured separately'))

(HERE / 'run1361-selected-lines.txt').write_text(
    ''.join(f'{n}: {lines[n-1]}' for n in sorted(selected)))
vfio_path = GRAVITY / 'vfio-reference/gsp.jsonl.gz'
rpc = []
malformed = 0
with gzip.open(vfio_path, 'rt') as stream:
    for n, line in enumerate(stream, 1):
        r = json.loads(line)
        if r.get('kind') != 'record':
            continue
        p = bytes.fromhex(r['payload_hex'])
        i = p.find(b'VRPC')
        if i < 4 or len(p) < i + 28:
            malformed += 1
            continue
        header = struct.unpack_from('<8I', p, i - 4)
        length, fn = header[2:4]
        if length < 32 or i - 4 + length > len(p) or fn != r['rpc_function']:
            malformed += 1
            continue
        body = p[i + 28:i - 4 + length]
        e = dict(line=n, qpc=r['qpc'], generation=r['generation'],
                 direction=r['direction'], function=fn, body=body)
        rpc.append(e)

def word(b, off):
    return struct.unpack_from('<I', b, off)[0]

def field_values(b, fields):
    # Only the two named address/offset fields below are NvU64 in these structs.
    return {name: hex(struct.unpack_from('<Q' if name in
                {'gpFifoOffset', 'userdOffset'} else '<I', b, off)[0])
            for name, off in fields.items() if name != 'size'}

vfio_regs = []
for e in rpc:
    b = e['body']
    if e['function'] != 76 or len(b) < 40 or e['direction'] != 0:
        continue
    client, obj, cmd, status, size = struct.unpack_from('<5I', b)
    if cmd not in {0x50800101, 0x50800103, 0x50800104}:
        continue
    if len(b) < 40 + size or size < 24:
        raise ValueError('truncated registration')
    params = b[40:40 + size]
    api, inner = struct.unpack_from('<2I', params)
    # 50800101 uses the same leading header but not the V2 FIFO-update union.
    update = (field_values(params[layout['registration']['api_bundle']:], layout['update'])
              if inner == 0x20801116 and cmd != 0x50800101 else None)
    reply = next((x for x in rpc if x['function'] == 76 and x['direction'] == 1
                  and x['generation'] == e['generation'] and x['line'] > e['line']
                  and len(x['body']) >= 48 and word(x['body'], 0) == client
                  and word(x['body'], 4) == obj and word(x['body'], 8) == cmd
                  and word(x['body'], 40) == api), None)
    vfio_regs.append(dict(line=e['line'], qpc=e['qpc'], generation=e['generation'],
                          client=hex(client), object=hex(obj), outer=hex(cmd),
                          api_handle=hex(api), inner=hex(inner), update=update,
                          reply_line=reply['line'] if reply else 'UNMEASURED',
                          reply_status=hex(word(reply['body'], 12)) if reply else 'UNMEASURED'))

vfio_channels = []
for client_s, obj_s in sorted({(r['client'], r['object']) for r in vfio_regs}):
    client, obj = int(client_s, 16), int(obj_s, 16)
    alloc = next((e for e in rpc if e['function'] == 103 and e['direction'] == 0
                  and len(e['body']) >= 32 and word(e['body'], 0) == client
                  and word(e['body'], 8) == obj and word(e['body'], 12) == 0x5080), None)
    parent = word(alloc['body'], 4) if alloc else None
    channel = next((e for e in rpc if e['function'] == 103 and e['direction'] == 0
                    and len(e['body']) >= 32 and word(e['body'], 0) == client
                    and word(e['body'], 8) == parent), None)
    facts = None
    if (channel and word(channel['body'], 20) == layout['channel']['size']
            and len(channel['body']) >= 32 + layout['channel']['size']):
        params = channel['body'][32:32 + layout['channel']['size']]
        facts = field_values(params, layout['channel'])
    controls = []
    for e in rpc:
        b = e['body']
        if (e['function'] == 76 and e['direction'] == 0 and len(b) >= 40
                and word(b, 0) == client and word(b, 8) in
                {0xa06c0101, 0xa06f0103, 0xc36f0108}):
            controls.append(dict(line=e['line'], qpc=e['qpc'],
                                 object=hex(word(b, 4)), command=hex(word(b, 8))))
    vfio_channels.append(dict(client=client_s, object=obj_s,
                              parent_channel=hex(parent) if parent else 'UNMEASURED',
                              alloc_line=alloc['line'] if alloc else 'UNMEASURED',
                              channel_alloc_line=channel['line'] if channel else 'UNMEASURED',
                              alloc_facts=facts, observed_schedule_token_controls=controls,
                              alloc_param_coverage=('complete' if facts else
                                  'UNMEASURED: channel alloc RPC header has no captured params'),
                              registrations=[r for r in vfio_regs if
                                             (r['client'], r['object']) == (client_s, obj_s)],
                              classification='UNMEASURED: native VFIO has no kayfabe route',
                              doorbell_and_gpu_trigger='UNMEASURED: RPC capture excludes GPU methods/BAR0 writes'))

output = dict(
    inputs={str(p.relative_to(HERE.parents[3])): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in [log_path, vfio_path]},
    run1361=channels, vfio=vfio_channels, vfio_malformed=malformed,
    conclusion='H1 INCONCLUSIVE: fault channel is Passthrough; actual update trigger unmeasured',
    registration_conversion='Pre-token/pre-schedule registration condition fails for fault channel; no conversion implemented')
(HERE / 'audit.json').write_text(json.dumps(output, indent=2) + '\n')
print(json.dumps(dict(kayfabe_channels=len(channels), kayfabe_registrations=len(registrations),
                      vfio_channels=len(vfio_channels), vfio_registrations=len(vfio_regs),
                      malformed=malformed, conclusion=output['conclusion'])))
