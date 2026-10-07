#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""mmio_window.py -- summarise a QEMU vfio trace (vfio_region_read/write) per GSP command-queue
head write. Head ordinal N (1-based) aligns with observer RPC index N.
usage: mmio_window.py MMIO_LOG FIRST_HEAD LAST_HEAD [--full-bar0]
Input: a VFIO run's `-trace` log (boundary runner without --no-mmio-trace). The GSP
command-queue head (BAR0 0x110c00) is written once per RPC request, so the head ordinal is the
RPC index (checked against vfio-10's observer in traces/windows_code43_walls_20261007/README.md).
Diagnostic only; reads the log line by line (untrusted data, nothing executed); bounded output.
"""
import collections
import re
import sys

HEAD = 0x110c00
POLL = {0x110094, 0xb81010}
INTR = range(0xb81200, 0xb81700)
RD = re.compile(r'vfio_region_read\s+\(0000:01:00\.0:region(\d)\+0x([0-9a-f]+), (\d+)\) = 0x([0-9a-f]+)')
WR = re.compile(r'vfio_region_write\s+\(0000:01:00\.0:region(\d)\+0x([0-9a-f]+), 0x([0-9a-f]+), (\d+)\)')


def main(argv):
    path, lo, hi = argv[0], int(argv[1]), int(argv[2])
    full = '--full-bar0' in argv
    n = 0
    per = collections.defaultdict(collections.Counter)
    vals = collections.defaultdict(set)
    seq = []
    with open(path, 'r', errors='replace') as f:
        for line in f:
            m = RD.search(line)
            if m:
                kind, reg, off, val = 'R', int(m.group(1)), int(m.group(2), 16), int(m.group(4), 16)
            else:
                m = WR.search(line)
                if not m:
                    continue
                kind, reg, off, val = 'W', int(m.group(1)), int(m.group(2), 16), int(m.group(3), 16)
            if reg == 0 and off == HEAD and kind == 'W':
                n += 1
                if n > hi:
                    break
                continue
            if n < lo:
                continue
            if reg == 0:
                if off in POLL or off in INTR:
                    per[n]['bar0-poll/intr'] += 1
                    continue
                per[n]['bar0 %s %06x' % (kind, off)] += 1
                if len(vals[(kind, off)]) < 8:
                    vals[(kind, off)].add(val)
                if full and len(seq) < 4000:
                    seq.append((n, kind, off, val))
            else:
                per[n]['region%d %s %s' % (reg, kind, '%x' % (off >> 16) + 'xxxx')] += 1
    for k in sorted(per):
        items = ', '.join('%s=%d' % (a, b) for a, b in sorted(per[k].items()))
        print('after head %d: %s' % (k, items[:1500]))
    print('# distinct BAR0 non-poll values (up to 8):')
    for (kind, off), v in sorted(vals.items(), key=lambda x: x[0][1]):
        print('%s %06x %s' % (kind, off, ' '.join(hex(x) for x in sorted(v))))
    for s in seq:
        print('SEQ', s[0], s[1], '%06x' % s[2], hex(s[3]))


if __name__ == '__main__':
    main(sys.argv[1:])
