#!/usr/bin/env python3
"""Per-control reply comparison of one die against the AD103 reference (2026-10-14 measurement lane).

Inputs are kf-census outputs only: PREFIX.controls.tsv (counts, sizes, statuses) and PREFIX.bodies.tsv
(the bodies a `kf-census scrub` kept; `kf-census --bodies` refuses unscrubbed inputs). No capture file is read.

usage: compare_values.py DIE_PREFIX REF_PREFIX ROWS.txt OUT.tsv [--hex]
  ROWS.txt: one control id per line (# comments), the rows lane C asked for.
  --hex: also print the die's distinct reply bodies (allow-listed bytes only) to stdout.
Equality is over the SET of distinct reply bodies (status direction) and, separately, the set of request
bodies; 'yes' means the two sets are identical byte for byte. 'n/a' = body not kept by the scrub
(not allow-listed) or the control is absent on one side.
"""
import sys


def controls(prefix):
    rows = {}
    with open(prefix + '.controls.tsv') as f:
        hdr = f.readline().rstrip('\n').split('\t')
        for line in f:
            c = dict(zip(hdr, line.rstrip('\n').split('\t')))
            rows[int(c['cmd'], 16) if c['cmd'].startswith('0x') else int(c['cmd'])] = c
    return rows


def bodies(prefix):
    out = {}
    try:
        f = open(prefix + '.bodies.tsv')
    except FileNotFoundError:
        return out
    with f:
        f.readline()
        for line in f:
            cmd, d, st, size, n, sha, hx = line.rstrip('\n').split('\t')
            out.setdefault((int(cmd, 16), d), {})[sha] = (int(n), st, int(size), hx)
    return out


def main():
    a = sys.argv[1:]
    show_hex = '--hex' in a
    a = [x for x in a if x != '--hex']
    die_p, ref_p, rows_f, out_f = a
    dc, rc = controls(die_p), controls(ref_p)
    db, rb = bodies(die_p), bodies(ref_p)
    ids = []
    for line in open(rows_f):
        t = line.split('#')[0].strip()
        if t:
            ids.append(int(t, 16))
    cols = ['cmd', 'name', 'die_present', 'die_calls', 'die_req_sizes', 'die_reply_sizes', 'die_statuses',
            'ref_present', 'ref_calls', 'ref_req_sizes', 'ref_reply_sizes', 'ref_statuses', 'sizes_equal',
            'body_kept', 'die_distinct_replies', 'ref_distinct_replies', 'reply_bytes_equal', 'request_bytes_equal']
    lines = ['\t'.join(cols)]
    for i in ids:
        d, r = dc.get(i), rc.get(i)
        name = (d or r or {}).get('name', '') or ''
        def g(x, k):
            return x[k] if x else '-'
        sizes_eq = 'n/a'
        if d and r:
            sizes_eq = 'yes' if (d['req_sizes'], d['reply_sizes']) == (r['req_sizes'], r['reply_sizes']) else 'no'
        kept = (i, 'status') in db or (i, 'status') in rb or (i, 'cmd') in db or (i, 'cmd') in rb
        def eq(direction):
            if not (d and r):
                return 'n/a'
            x, y = db.get((i, direction)), rb.get((i, direction))
            if x is None and y is None:
                return 'n/a'
            if x is None or y is None:
                return 'n/a'
            return 'yes' if set(x) == set(y) else 'no'
        lines.append('\t'.join(str(v) for v in [
            '%#010x' % i, name, 'yes' if d else 'no', g(d, 'requests'), g(d, 'req_sizes'), g(d, 'reply_sizes'),
            g(d, 'statuses'), 'yes' if r else 'no', g(r, 'requests'), g(r, 'req_sizes'), g(r, 'reply_sizes'),
            g(r, 'statuses'), sizes_eq, 'yes' if kept else 'no', len(db.get((i, 'status'), {})) if d else '-',
            len(rb.get((i, 'status'), {})) if r else '-', eq('status'), eq('cmd')]))
        if show_hex and (i, 'status') in db:
            for sha, (n, st, size, hx) in db[(i, 'status')].items():
                inref = 'same-as-ref' if sha in rb.get((i, 'status'), {}) else 'differs'
                print('%#010x status=%s size=%d n=%d %s %s' % (i, st, size, n, inref, hx))
    open(out_f, 'w').write('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
