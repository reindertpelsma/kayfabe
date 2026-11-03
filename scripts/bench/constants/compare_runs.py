#!/usr/bin/env python3
"""Compare the scrubbed reply bodies of several kf-census runs, control by control (lane C, 2026-10-14).

usage: compare_runs.py OUT.tsv LABEL=PREFIX [LABEL=PREFIX ...]
For each control with a kept body in any run: per run the number of calls, the distinct reply bodies (by sha256) and request sizes,
then for the FIRST run versus each other: 'same' (identical set of distinct reply bodies), 'diff', or 'n/a' (not kept / absent).
Reads census text only.  Run-to-run noise rows (see the AD103 baseline: 0x20809004, 0x2080b201, 0x20801111, 0x2080a088 request)
are expected to show 'diff' between two boots of the SAME die.
"""
import csv, sys

def load(prefix):
    ctl = {r['cmd']: r for r in csv.DictReader(open(prefix + '.controls.tsv'), delimiter='\t')}
    bod = {}
    for r in csv.DictReader(open(prefix + '.bodies.tsv'), delimiter='\t'):
        if r['dir'] == 'status':
            bod.setdefault(r['cmd'], {})[r['sha256']] = r
    return ctl, bod

def main():
    out = sys.argv[1]
    runs = [(a.split('=', 1)[0], load(a.split('=', 1)[1])) for a in sys.argv[2:]]
    cmds = sorted(set(k for _, (c, b) in runs for k in b))
    head = ['cmd', 'name']
    for l, _ in runs:
        head += [f'{l}_calls', f'{l}_size', f'{l}_distinct']
    head += [f'{runs[0][0]}_vs_{l}' for l, _ in runs[1:]]
    lines = ['\t'.join(head)]
    for k in cmds:
        row = [k, next((c[k]['name'] for _, (c, b) in runs if k in c and c[k]['name'] != '-'), '-')]
        for l, (c, b) in runs:
            r = c.get(k)
            row += [r['requests'] if r else '-', r['reply_sizes'] if r else '-', str(len(b.get(k, {}))) if k in b else '-']
        a = runs[0][1][1].get(k)
        for l, (c, b) in runs[1:]:
            o = b.get(k)
            row.append('n/a' if a is None or o is None else ('same' if set(a) == set(o) else 'diff'))
        lines.append('\t'.join(row))
    open(out, 'w').write('\n'.join(lines) + '\n')

if __name__ == '__main__':
    main()
