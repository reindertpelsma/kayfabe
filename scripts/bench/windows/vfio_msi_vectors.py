#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""vfio_msi_vectors.py -- which interrupt source each MSI of a VFIO guest serviced (2026-10-08, VFIO DVI reference).

usage: vfio_msi_vectors.py TRACE_LOG [--timeline OUT] [--from HH:MM:SS --to HH:MM:SS]

TRACE_LOG is QEMU's `-msg timestamp=on -trace` log with vfio_msi_interrupt and vfio_region_write (BAR0 trapped by
x-gsp-observer). The 4070 under Windows uses ONE MSI vector, so the MSI vector does not name the source; the
interrupt tree does: after each MSI the guest's ISR clears what it serviced by writing 1s to
NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(i) (BAR0 0xB81000 + 4*i, write-1-to-clear), so a W1C bit b of leaf i is
interrupt vector 32*i + b. Vectors are named from the kernel interrupt table the GSP returned to this guest
(NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE reply in the observer capture, vfio boot2, 2026-10-08; engine
indices are MC_ENGINE_IDX_* of OGKM engine_idx.h). Writes between two MSIs are attributed to the first.
Prints per-vector totals and per-second counts; --timeline writes one line per MSI. Log content is data.
"""
import collections
import re
import sys

# vector -> name, from the GSP's reply to INTR_GET_KERNEL_TABLE (VFIO boot2 R00017), engine names per engine_idx.h
VEC = {0: 'GR0 nonstall', 2: 'SEC2 nonstall', 3: 'NVDEC0 nonstall', 7: 'CE2 nonstall', 8: 'CE3 nonstall',
       10: 'CE4 nonstall', 11: 'NVENC1 nonstall', 18: 'OFA0 nonstall', 64: 'REPLAYABLE_FAULT stall',
       72: 'ACCESS_CNTR stall', 129: 'CPU_DOORBELL stall', 131: 'REPLAYABLE_FAULT_ERROR stall',
       154: 'DISP stall', 155: 'GSP stall'}
LINE = re.compile(r'^(\S+)Z (vfio_\w+)\s+\((\S+?)(?::region0\+(0x[0-9a-f]+), (0x[0-9a-f]+))?[,)]')


def name(v):
    return '%d(%s)' % (v, VEC.get(v, '?'))


def main(argv):
    path = argv[1]
    tl = argv[argv.index('--timeline') + 1] if '--timeline' in argv else None
    lo = argv[argv.index('--from') + 1] if '--from' in argv else None
    hi = argv[argv.index('--to') + 1] if '--to' in argv else None
    msis = []          # [time, set(vectors)]
    for line in open(path, encoding='utf-8', errors='replace'):
        m = LINE.match(line)
        if not m:
            continue
        t, ev, dev = m.group(1), m.group(2), m.group(3)
        if ev == 'vfio_msi_interrupt' and dev == '0000:01:00.0':
            msis.append([t, set()])
        elif ev == 'vfio_region_write' and m.group(4) and msis:
            off, val = int(m.group(4), 16), int(m.group(5), 16)
            if 0xb81000 <= off < 0xb81020:
                leaf = (off - 0xb81000) // 4
                for b in range(32):
                    if val >> b & 1:
                        msis[-1][1].add(32 * leaf + b)
    sel = [x for x in msis if (lo is None or x[0][11:19] >= lo) and (hi is None or x[0][11:19] <= hi)]
    tot = collections.Counter()
    none = 0
    per_s = collections.defaultdict(collections.Counter)
    for t, vs in sel:
        if not vs:
            none += 1
        for v in vs:
            tot[v] += 1
            per_s[t[11:19]][v] += 1
    print('# MSIs (01:00.0) %d in window, %d total; with no LEAF W1C before the next MSI: %d' % (len(sel), len(msis), none))
    for v, n in tot.most_common():
        print('vector %s: %d' % (name(v), n))
    print('# per second: time  total-MSIs  vector:count ...')
    msi_s = collections.Counter(t[11:19] for t, _ in sel)
    for s in sorted(msi_s):
        print(s, msi_s[s], ' '.join('%d:%d' % (v, n) for v, n in sorted(per_s[s].items())))
    if tl:
        with open(tl, 'w') as f:
            for t, vs in sel:
                f.write('%s %s\n' % (t, ' '.join(name(v) for v in sorted(vs)) or '-'))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
