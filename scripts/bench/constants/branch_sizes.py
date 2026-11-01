#!/usr/bin/env python3
"""paramsSize of every control in two kf-census runs (different GSP branches), side by side (lane C, 2026-10-14).

usage: branch_sizes.py LABEL_A=PREFIX_A LABEL_B=PREFIX_B OUT.tsv
Reads PREFIX.controls.tsv only.  Columns: cmd, name, ogkm basis in run A / B, request size set A / B, 'changed' when the SET of request
sizes differs.  The Windows-only controls (no ogkm tag defines them) change size between firmware branches exactly like the
defined ones do; the defined ones kayfabe takes from its generated per-tag tables, the others only from `kf_abi::gssreplay` rows.
"""
import csv, sys

def load(p):
    return {r['cmd']: r for r in csv.DictReader(open(p + '.controls.tsv'), delimiter='\t')}

def sizes(r):
    return sorted({int(x.split(':')[0]) for x in r['req_sizes'].split(',') if x})

def main():
    (la, pa), (lb, pb) = (a.split('=', 1) for a in sys.argv[1:3])
    A, B = load(pa), load(pb)
    out = ['\t'.join(['cmd', 'name', f'ogkm_{la}', f'ogkm_{lb}', f'sizes_{la}', f'sizes_{lb}', 'changed'])]
    for k in sorted(set(A) | set(B)):
        a, b = A.get(k), B.get(k)
        sa = sizes(a) if a else []
        sb = sizes(b) if b else []
        name = next((x['name'] for x in (a, b) if x and x['name'] != '-'), '-')
        ch = 'absent_' + (lb if not b else la) if not (a and b) else ('yes' if sa != sb else 'no')
        out.append('\t'.join([k, name, a['ogkm'] if a else '-', b['ogkm'] if b else '-',
                              ','.join(map(str, sa)) or '-', ','.join(map(str, sb)) or '-', ch]))
    open(sys.argv[3], 'w').write('\n'.join(out) + '\n')

if __name__ == '__main__':
    main()
