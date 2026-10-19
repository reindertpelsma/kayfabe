#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Summarize arms/*/result.json: per arm median [min,max] of Steel Nomad score (fps = score/100) and tiny-probe medians, slopes."""
import json, statistics, sys
from pathlib import Path
root = Path(sys.argv[1] if len(sys.argv) > 1 else '/data/claude-trapexp-20261019/arms')
rows = {}
for d in sorted(root.iterdir()):
    f = d / 'result.json'
    if f.exists(): rows[d.name] = json.load(open(f))
def m(a): return statistics.median(a) if a else None
print('| arm | rev | flags (experiment) | SN n | SN score median [min,max] | fps (score/100) | DIRECT / COMPUTE / COPY us (n) | Xid delta | 4101/153 | error |')
print('|---|---|---|---|---|---|---|---|---|---|')
for a, r in rows.items():
    s = r.get('sn_scores') or []
    fl = ' '.join(x for x in r['flags'].split() if x.startswith('KF3_DIAG')) or '-'
    p = r.get('probe_median_us') or {}
    if p and None in p.values(): p = {}
    h = (r.get('health1') or '').replace('HEALTH ', '')
    hh = ' '.join(x for x in h.split() if x.startswith(('tdr', 'gpu', 'app')))
    print('| %s | %s | %s | %d | %s | %s | %s (%s) | %s | %s | %s |' % (
        a, r['rev'], fl, len(s),
        ('%g [%d,%d]' % (m(s), min(s), max(s))) if s else '-',
        ('%.2f' % (m(s) / 100)) if s else '-',
        ' / '.join('%.0f' % p[k] for k in ('DIRECT', 'COMPUTE', 'COPY')) if p else '-', r.get('probe_n'),
        r.get('xid_delta'), hh, r.get('error') or ''))
def slope(xs, ys):
    n = len(xs); mx = sum(xs) / n; my = sum(ys) / n
    den = sum((x - mx) ** 2 for x in xs)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den if den else None
for name, arms in (('interrupt-tree delay', [('control1', 0), ('intr10', 10), ('intr20', 20), ('intr100', 100)]), ('doorbell delay', [('control1', 0), ('db10', 10), ('db20', 20), ('db20b', 20)])):
    pts = [(x, rows[a]) for a, x in arms if a in rows and rows[a].get('sn_scores')]
    if len(pts) >= 2:
        xs = [x for x, _ in pts]
        print('\n%s: SN score slope %.2f per added us (fps/us = %.4f)' % (name, slope(xs, [m(r['sn_scores']) for _, r in pts]), slope(xs, [m(r['sn_scores']) / 100 for _, r in pts])))
        for k in ('DIRECT', 'COMPUTE', 'COPY'):
            ys = [r['probe_median_us'][k] for _, r in pts if r.get('probe_median_us')]
            if len(ys) == len(xs): print('  tiny probe %s slope %.2f us per added us' % (k, slope(xs, ys)))
