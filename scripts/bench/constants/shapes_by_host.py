#!/usr/bin/env python3
"""Accepted zero-request paramsSize per Windows-only control, per HOST driver (from the kit's SCAN lines).  (lane C, 2026-10-14)
usage: shapes_by_host.py OUT.tsv FILE...   -> one row per (control, host driver): the contiguous accepted runs (a run '1432-16384' means
the host accepts every multiple of 4 from the struct size up; the first value is the struct size).  Reads kit text only."""
import re, sys, os
CTL = ['852e', '853a', 'a618', 'b201', '852f']
def main():
    seen = {}
    for f in sys.argv[2:]:
        t = open(f).read().splitlines()
        if not any(l.startswith('SCAN') for l in t):
            continue
        drv = next((l.split('driver=')[1] for l in t if l.startswith('PROBE_SESSION')), None)
        if not drv:
            continue
        die = next((l.split('die=')[1] for l in t if l.startswith('PROBE_DIE')), '-')
        gpu = next((l for l in t if l.startswith('GPU ')), 'GPU ?')[4:].split(',')[0].replace('NVIDIA ', '').replace('GeForce ', '')
        row = {}
        for c in CTL:
            sz = [int(re.search(r'size=(\d+)', l).group(1)) for l in t if l.startswith('SCAN cmd=0x2080%s ' % c) and ' size=' in l]
            if not sz:
                row[c] = 'none'
                continue
            runs = []; s = p = sz[0]
            for x in sz[1:]:
                if x != p + 4:
                    runs.append((s, p)); s = x
                p = x
            runs.append((s, p))
            row[c] = ','.join(str(a) if a == b else f'{a}-{b}' for a, b in runs)
        seen[(drv, die, gpu)] = row
    out = ['\t'.join(['host_driver', 'die_by_host', 'board'] + CTL)]
    for (drv, die, gpu), row in sorted(seen.items(), key=lambda kv: [int(x) for x in kv[0][0].split('.')]):
        out.append('\t'.join([drv, die, gpu] + [row[c] for c in CTL]))
    open(sys.argv[1], 'w').write('\n'.join(out) + '\n')
main()
