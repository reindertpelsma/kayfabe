#!/usr/bin/env python3
"""Request-size set of every control across several kf-census runs (one column per run).  (lane C, 2026-10-14)
usage: sizes_matrix.py OUT.tsv LABEL=PREFIX ...   Reads PREFIX.controls.tsv only.  'changes' = number of distinct size sets over the runs."""
import csv, sys
runs = [a.split('=', 1) for a in sys.argv[2:]]
data = []
for l, p in runs:
    data.append({r['cmd']: r for r in csv.DictReader(open(p + '.controls.tsv'), delimiter='\t')})
cmds = sorted(set().union(*[set(d) for d in data]))
def sz(r):
    return '/'.join(str(x) for x in sorted({int(s.split(':')[0]) for s in r['req_sizes'].split(',') if s})) if r else '-'
out = ['\t'.join(['cmd', 'name'] + [l for l, _ in runs] + ['distinct_size_sets'])]
for c in cmds:
    cols = [sz(d.get(c)) for d in data]
    name = next((d[c]['name'] for d in data if c in d and d[c]['name'] != '-'), '-')
    out.append('\t'.join([c, name] + cols + [str(len(set(cols)))]))
open(sys.argv[1], 'w').write('\n'.join(out) + '\n')
