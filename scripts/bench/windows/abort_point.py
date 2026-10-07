#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""abort_point.py -- where Windows StartDevice gives up on kayfabe, and what a
Code43-free VFIO boot does from that point on.

usage:
  abort_point.py points KAYFABE_QEMU_LOG[.gz]...
  abort_point.py forecast VFIO_DECODED_JSON KAYFABE_QEMU_LOG[.gz]... [--names NAMES]

`points`: per kayfabe log (KF3_RPC_TRACE=1), the last non-Free RPC before the
first run of >=20 consecutive Free RPCs (the driver's teardown) and its result.
`forecast`: decodes the VFIO GSP-observer export (`kayfabe-gsp-observer/1`),
pairs requests and replies in queue order, finds the last kayfabe abort RPC in
the VFIO sequence and lists every distinct later control/class with its VFIO
status, count and whether any given kayfabe log ever served/refused it.

Diagnostic only. Inputs are untrusted data: reads are bounded, nothing is
executed or forwarded, and handles and timestamps are not part of any key.
"""
import collections
import gzip
import json
import re
import struct
import sys

LIMIT = 256 * 1024 * 1024
TEARDOWN_RUN = 20
TRACE = re.compile(r"kf-rm: rpc-trace fn=(\d+) (\w+) seq=\d+ ?(?:cmd=(0x[0-9a-f]+)|class=(0x[0-9a-f]+))?.*? result=(\S+)")


def read_text(path):
    opener = gzip.open if path.endswith('.gz') else open
    with opener(path, 'rb') as f:
        data = f.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError(f'{path}: exceeds read bound')
    return data.decode('utf-8', 'replace')


def kayfabe(path):
    out = []
    for line in read_text(path).splitlines():
        m = TRACE.search(line)
        if not m:
            continue
        cmd, cls, res = m.group(3), m.group(4), m.group(5)
        key = '%08x' % int(cmd, 16) if cmd else '%04x' % int(cls, 16) if cls else ''
        out.append((int(m.group(1)), key, None if res == 'none' else int(res, 16)))
    return out


def vfio(path):
    doc = json.loads(read_text(path))
    if doc.get('schema') != 'kayfabe-gsp-observer/1':
        raise ValueError('unexpected observer schema')
    seen, pending, out = set(), [], []
    for o in doc['observations']:
        b = bytes.fromhex(o['payload_hex'])
        dedup = (o['direction'], o['table_pa'], o['queue_sequence'], o['payload_hex'][:512])
        if dedup in seen:  # the observer re-attaches and replays retained history
            continue
        seen.add(dedup)
        i = b.find(b'VRPC')
        if i < 0 or len(b) < i + 28:
            continue
        fn, res = struct.unpack_from('<II', b, i + 8)
        if fn >= 0x1000:  # GSP-initiated event, not a reply
            continue
        body, key, status = b[i + 28:], '', None
        if fn == 76 and len(body) >= 32:  # rpc_gsp_rm_control_v03_00
            key, status = '%08x' % struct.unpack_from('<I', body, 8)[0], struct.unpack_from('<I', body, 12)[0]
        elif fn == 103 and len(body) >= 20:  # rpc_gsp_rm_alloc_v03_00
            key, status = '%04x' % struct.unpack_from('<I', body, 12)[0], struct.unpack_from('<I', body, 16)[0]
        if o['direction'] == 'request':
            r = [fn, key, None]
            out.append(r)
            pending.append(r)
            continue
        for j, r in enumerate(pending):  # GSP serves its command queue in order
            if r[0] == fn:
                r[2] = status if status is not None else res
                del pending[j]
                break
    return [tuple(r) for r in out]


def abort_index(seq):
    for i in range(len(seq) - TEARDOWN_RUN + 1):
        if all(r[0] == 10 for r in seq[i:i + TEARDOWN_RUN]):
            j = i - 1
            while j >= 0 and seq[j][0] == 10:
                j -= 1
            return j, i
    return None, None


def fmt(status):
    return 'refused(unserviced)' if status is None else hex(status)


def main(argv):
    names = {}
    if '--names' in argv:
        k = argv.index('--names')
        for line in read_text(argv[k + 1]).splitlines():
            parts = line.split()
            if len(parts) == 2:
                names[parts[0]] = parts[1]
        del argv[k:k + 2]
    if len(argv) >= 2 and argv[0] == 'points':
        for path in argv[1:]:
            seq = kayfabe(path)
            j, t = abort_index(seq)
            if j is None:
                print(f'{path}\trpcs={len(seq)}\tno teardown')
                continue
            fn, key, st = seq[j]
            print(f'{path}\trpcs={len(seq)}\tteardown_at={t}\tlast=fn{fn}/{key} {names.get(key, "")}\tresult={fmt(st)}')
        return 0
    if len(argv) >= 3 and argv[0] == 'forecast':
        v = vfio(argv[1])
        served, refused, last = collections.Counter(), collections.Counter(), None
        for path in argv[2:]:
            seq = kayfabe(path)
            for fn, key, st in seq:
                (served if st == 0 else refused)[(fn, key)] += 1
            j, _ = abort_index(seq)
            if j is not None:
                last = seq[j][:2]
        start = next(i for i, r in enumerate(v) if r[:2] == last)
        print(f'# VFIO rpcs={len(v)}; last kayfabe abort RPC fn{last[0]}/{last[1]} {names.get(last[1], "")} is VFIO index {start}')
        print('# vfio_index fn key name vfio_status vfio_count kayfabe')
        counts = collections.Counter(r[:2] for r in v[start:])
        done = set()
        for i in range(start, len(v)):
            fn, key, st = v[i]
            if fn == 10 or (fn, key) in done:
                continue
            done.add((fn, key))
            k = (fn, key)
            seen = ('served' if k in served and k not in refused else 'served+refused' if k in served
                    else 'REFUSED' if k in refused else 'never-seen')
            print(i, fn, key or '-', names.get(key, '-'), fmt(st), counts[k], seen)
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
