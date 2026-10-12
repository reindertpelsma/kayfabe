#!/usr/bin/env python3
"""Reference-only, read-only sampling from QEMU's guest-RAM memfd.

No ptrace, GPU MMIO, driver controls, writes, or whole-RAM export. QEMU's actual
ram0 alias layout plus memfd bounds resolve public single-page SYSTEM-list PFNs.
Only USERD Reference/GP_GET/GP_PUT fields are retained. Passive samples do not
prove trigger execution or general lifetime semantics. Do not use as product code.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import stat
import struct
import sys
import time

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tools/vfio-gsp-observer'))
import decode

PAGE = 4096


def aliases(text, size):
    result = set()
    for line in text.splitlines():
        match = re.search(r'([0-9a-f]{16})-([0-9a-f]{16}) .*alias (ram-(?:below|above)-4g) @ram0 ([0-9a-f]{16})-([0-9a-f]{16})', line)
        if match:
            begin, end, name, offset, last = match.groups()
            begin, end, offset, last = (int(x, 16) for x in (begin, end, offset, last))
            if end < begin or last < offset or end - begin != last - offset or last >= size:
                raise ValueError('inconsistent RAM alias bounds')
            result.add((begin, end + 1, offset))
    if len(result) != 2:
        raise ValueError('expected two distinct public PC ram0 aliases')
    return sorted(result)


def resolve(ranges, gpa, size, memory_size):
    if gpa < 0 or size <= 0 or gpa + size >= 1 << 64:
        raise ValueError('invalid physical span')
    matches = [base + gpa - start for start, end, base in ranges
               if start <= gpa and gpa + size <= end]
    if len(matches) != 1 or matches[0] + size > memory_size:
        raise ValueError('physical span is not fully in a unique guest RAM alias')
    return matches[0]


def memory_request(record):
    b = bytes.fromhex(record['payload_hex'])[80:]
    if record['rpc_function'] != 4 or record['direction'] != 'request' or len(b) < 64:
        return None
    client, device, handle, cls, flags, adjust, fmt = struct.unpack_from('<7I', b)
    length = struct.unpack_from('<Q', b, 32)[0]
    pages = struct.unpack_from('<I', b, 40)[0]
    desc = struct.unpack_from('<I', b, 48)[0]
    # Public rpc.c _issueRpcWithPtes: IDR_NONE, PFN = guestPages >>12.
    # Restrict this experiment to exactly one aligned SYSTEM page, no PDE walk.
    if cls != 0x81 or length != PAGE or pages != 1 or adjust or desc != 1 << 16:
        return None
    pfn = struct.unpack_from('<Q', b, 56)[0]
    if pfn >= 1 << 52:
        raise ValueError('PFN overflow')
    return dict(client=hex(client), handle=hex(handle), gpa=pfn << 12)


def fields(page):
    if len(page) != PAGE:
        raise ValueError('short guest RAM sample')
    return [dict(slot=i * 512, reference=struct.unpack_from('<I', page, i * 512 + 0x48)[0],
                 get=struct.unpack_from('<I', page, i * 512 + 0x88)[0],
                 put=struct.unpack_from('<I', page, i * 512 + 0x8c)[0]) for i in range(8)]


def qmp(sockpath, command, arguments=None):
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(5)
        sock.connect(str(sockpath))
        stream = sock.makefile('rwb')
        def receive():
            for _ in range(128):
                row = json.loads(stream.readline(1024 * 1024))
                if 'event' not in row:
                    return row
            raise ValueError('QMP event bound exceeded')
        if 'QMP' not in receive():
            raise ValueError('missing QMP greeting')
        stream.write(b'{"execute":"qmp_capabilities"}\n'); stream.flush()
        if 'return' not in receive():
            raise ValueError('QMP capabilities failed')
        stream.write((json.dumps(dict(execute=command, arguments=arguments or {}))+'\n').encode()); stream.flush()
        reply = receive()
        if 'return' not in reply:
            raise ValueError('QMP query failed: '+str(reply))
        return reply['return']


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('boot', type=Path)
    ap.add_argument('output', type=Path)
    ap.add_argument('--seconds', type=int, default=330, choices=range(1, 601))
    args = ap.parse_args()
    pid = int((args.boot/'qemu.pid').read_text())
    proc = Path('/proc')/str(pid)
    command = (proc/'cmdline').read_bytes().replace(b'\0', b' ')
    if str(args.boot/'qmp.sock').encode() not in command or b'memory-backend-memfd,id=ram0,size=8G,share=on' not in command:
        raise ValueError('unexpected owned QEMU command')
    size = qmp(args.boot/'qmp.sock', 'qom-get', dict(path='/objects/ram0', property='size'))
    if size != 8 * 1024**3:
        raise ValueError('unexpected RAM size')
    layout = qmp(args.boot/'qmp.sock', 'human-monitor-command', dict(**{'command-line': 'info mtree'}))
    ranges = aliases(layout, size)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.with_suffix('.mtree.txt').write_text(layout)
    fds = []
    for path in (proc/'fd').iterdir():
        try:
            target, info = os.readlink(path), path.stat()
        except FileNotFoundError:
            continue
        if target.startswith('/memfd:memory-backend-memfd') and stat.S_ISREG(info.st_mode) and info.st_size == size:
            fds.append(path)
    if len(fds) != 1:
        raise ValueError('guest RAM memfd identity is ambiguous')
    with fds[0].open('rb', buffering=0) as ram, (args.boot/'gsp.jsonl').open() as trace, args.output.open('x') as output:
        digest, count, reads, registrations = hashlib.sha256(), 0, 0, 0
        def emit(kind, **values):
            nonlocal count
            if count >= 100000:
                raise ValueError('sample output record cap reached')
            raw = json.dumps(dict(kind=kind, sequence=count, sampled_ns=time.monotonic_ns(), **values), sort_keys=True)+'\n'
            output.write(raw); output.flush(); digest.update(raw.encode()); count += 1
        emit('header', schema='kayfabe-native-userd-samples/1', pid=pid, guest_ram_bytes=size,
             ranges=ranges, complete=False, period_seconds=.001, ram_fd=str(fds[0]),
             note='Two CPU snapshots; no atomic GPU execution/cursor-transfer claim.')
        end, tracked, last, serial, record_index = time.monotonic() + args.seconds, {}, {}, 0, 0
        reason = 'observation-deadline'
        active_generation = None
        while time.monotonic() < end:
            if (args.boot/'stop-userd-sampler').exists():
                reason = 'owner-collection-stop'; break
            if not proc.exists():
                reason = 'qemu-exited'; break
            for _ in range(4096):
                position = trace.tell(); line = trace.readline(140001)
                if not line or not line.endswith('\n'):
                    trace.seek(position); break
                if len(line) > 140000:
                    raise ValueError('trace line cap exceeded')
                row = json.loads(line)
                if row.get('kind') != 'record':
                    continue
                record_index += 1
                if row['generation'] != active_generation:
                    tracked.clear(); last.clear(); active_generation = row['generation']
                    emit('generation-change', generation=active_generation)
                if row['direction'] != 0 or row['rpc_function'] not in {4, 10, 76}:
                    continue
                # Verify selected records' framing/checksum individually while
                # live. Full footer/hash validation is mandatory after shutdown.
                vals = [row[k] for k in ('magic','header_bytes','payload_bytes','direction','qpc','table_pa','queue_sequence','rpc_sequence','rpc_function','rpc_result','flags','missing_before','rpc_version','reserved')]
                payload = bytes.fromhex(row['payload_hex'])
                if len(payload) != row['payload_bytes'] or len(payload) > 65536:
                    raise ValueError('record payload mismatch')
                binary = decode.FILE.pack(0x5457474b,1,64,64,1000000000,0,1,0,0,0,0) + decode.RECORD.pack(*vals) + payload
                rec = decode.parse(binary)['records'][0]
                generation = row['generation']
                memory = memory_request(rec)
                if memory:
                    key = (generation, memory['client'], memory['handle'])
                    if key not in tracked and len(tracked) >= 32:
                        raise ValueError('tracked memory cap exceeded')
                    serial += 1
                    offset = resolve(ranges, memory['gpa'], PAGE, size)
                    tracked[key] = dict(**memory, allocation_serial=serial, offset=offset)
                    emit('memory-candidate', record_index=record_index, observed_ns=row['qpc'], generation=generation, **tracked[key])
                b = payload[80:]
                if rec['rpc_function'] == 10 and len(b) >= 16:
                    key = (generation, hex(struct.unpack_from('<I', b)[0]), hex(struct.unpack_from('<I', b, 8)[0]))
                    old = tracked.pop(key, None)
                    if old:
                        emit('observed-free', generation=generation, record_index=record_index, **old)
                ctrl = rec.get('control', {})
                param = bytes.fromhex(ctrl.get('params_hex', ''))
                if ctrl.get('command') in {'0x50800103', '0x50800104'} and len(param) == 584 and struct.unpack_from('<I', param, 4)[0] == 0x20801116:
                    registrations += 1
                    values = struct.unpack_from('<4I2Q', param, 24)
                    emit('registration', record_index=record_index, observed_ns=row['qpc'], generation=generation,
                        client=hex(values[0]), channel=hex(values[1]), userd=hex(values[2]), entries=values[3],
                        fifo=hex(values[4]), slot=values[5], api_handle=hex(struct.unpack_from('<I', param)[0]))
            # Deduplicate physical pages, but retain active observed aliases. No
            # read is made through GPU BARs or arbitrary process virtual memory.
            pages = {}
            for key, item in tracked.items():
                pages.setdefault(item['offset'], []).append(dict(generation=key[0], **item))
            for offset, candidates in pages.items():
                first = fields(os.pread(ram.fileno(), PAGE, offset))
                second = fields(os.pread(ram.fileno(), PAGE, offset)); reads += 2
                value = (first, second, candidates)
                if last.get(offset) != value:
                    emit('sample', offset=offset, candidates=candidates, fields=first, second=second, stable_fields=first == second)
                    last[offset] = value
            time.sleep(.001)
        footer = dict(kind='footer', complete=False, file_export_complete=True, records=count,
                      sha256=digest.hexdigest(), reads=reads, registrations=registrations, reason=reason)
        output.write(json.dumps(footer,sort_keys=True)+'\n'); output.flush()
    print(json.dumps(footer, sort_keys=True))


if __name__ == '__main__':
    main()
