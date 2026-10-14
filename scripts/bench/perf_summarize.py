#!/usr/bin/env python3
"""perf_summarize.py - parse the raw output of scripts/bench/perf_compare.sh into numbers.

  perf_summarize.py extract  <outdir>              raw/* -> metrics.tsv   (lane metric rep value unit)
  perf_summarize.py summary  <outdir>              metrics.tsv -> summary.tsv + summary.md
  perf_summarize.py compare  <outdir> <basedir>    summary.tsv of both -> a diff table on stdout

Only the stdlib is used; run it as `python3 -I`. Parsing is by `key=value` tokens, so a new field
in a line never breaks an old parser. A metric that cannot be parsed is absent, never zero.

MEASURED vs DERIVED: every metric whose name starts with `derived.` is computed here from measured
ones (launch rate = 1000 / batch_per_launch_ms, transfer GB/s = bytes / ms); nothing else is.
"""
import os
import re
import statistics
import sys

KV = re.compile(r'([A-Za-z0-9_]+)=(\S+)')


def kvs(line):
    return {k: v for k, v in KV.findall(line)}


def num(s):
    try:
        return float(s)
    except (TypeError, ValueError):
        return None


class Out:
    def __init__(self):
        self.rows = []

    def add(self, lane, metric, rep, value, unit=''):
        if value is None:
            return
        self.rows.append((lane, metric, str(rep), repr(float(value)) if not isinstance(value, str) else value, unit))


def rd(path):
    try:
        with open(path, 'r', errors='replace') as f:
            return f.read().splitlines()
    except OSError:
        return []


def rep_of(name):
    m = re.search(r'_r(\d+)(?:_|\.|$)', name)
    return int(m.group(1)) if m else 1


def parse_fast(o, raw):
    for fn in sorted(os.listdir(raw)):
        if not fn.startswith('fast_r') or not fn.endswith('.out'):
            continue
        rep = rep_of(fn)
        sum_c = sum_w = 0.0
        npass = n = 0
        for ln in rd(os.path.join(raw, fn)):
            if ln.startswith('FAST_CELL_ARM'):
                d = kvs(ln)
                a = d.get('arm', '?')
                n += 1
                npass += d.get('verdict') == 'PASS'
                c, w = num(d.get('client_ms')), num(d.get('secs'))
                o.add('fast', f'client_ms.{a}', rep, c, 'ms')
                o.add('fast', f'wall_s.{a}', rep, w, 's')
                sum_c += c or 0
                sum_w += w or 0
        if n:
            o.add('fast', 'SUM.client_ms', rep, sum_c, 'ms')
            o.add('fast', 'SUM.wall_s', rep, sum_w, 's')
            o.add('fast', 'arms_pass', rep, npass, 'count')
            o.add('fast', 'arms_total', rep, n, 'count')


def parse_bare(o, raw):
    for fn in sorted(os.listdir(raw)):
        if not fn.startswith('bare_r') or not fn.endswith('.out'):
            continue
        rep = rep_of(fn)
        tot = 0.0
        npass = n = 0
        for ln in rd(os.path.join(raw, fn)):
            if ln.startswith('BARE_CELL_ARM'):
                d = kvs(ln)
                n += 1
                npass += d.get('verdict') == 'PASS'
                ms = num(d.get('ms'))
                o.add('bare', f"ms.{d.get('arm', '?')}", rep, ms, 'ms')
                tot += ms or 0
        if n:
            o.add('bare', 'SUM.ms', rep, tot, 'ms')
            o.add('bare', 'arms_pass', rep, npass, 'count')
            o.add('bare', 'arms_total', rep, n, 'count')


BSUM_FIELDS = ('first_ms', 'med_ms', 'p90_ms', 'min_ms', 'gflops', 'batch_per_launch_ms', 'batch_gflops',
               'h2d_ms', 'd2h_med_ms', 'submit_med_ms', 'sync_med_ms')
UNITS = {'gflops': 'GFLOPS', 'batch_gflops': 'GFLOPS'}


def parse_ladder(o, raw, mode):
    lane = f'ladder_{mode}'
    pref = f'cl_{mode}_'
    for fn in sorted(os.listdir(raw)):
        if not (fn.startswith(pref) and fn.endswith('.out')):
            continue
        rep = rep_of(fn)
        for ln in rd(os.path.join(raw, fn)):
            if ln.startswith('CL_ROW'):
                d = kvs(ln)
                if mode == 'host':
                    rep = int(d.get('rep', rep))
                r = d.get('rung', '?')
                o.add(lane, f'pass.{r}', rep, 1.0 if d.get('verdict') == 'PASS' else 0.0, 'bool')
                if mode == 'host':
                    o.add(lane, f'wall_ms.{r}', rep, num(d.get('wall_ms')), 'ms')
                else:
                    o.add(lane, f'trapped.{r}', rep, num(d.get('trapped')), 'count')
                    o.add(lane, f'wall_s.{r}', rep, num(d.get('CUP8_WALL_S') or d.get('BENCH_measure_WALL_S')), 's')
            elif ln.startswith('CL_BENCH'):
                d = kvs(ln)
                if mode == 'host':
                    rep = int(d.get('rep', rep))
                for k in ('BENCH_INIT_MS', 'BENCH_CTX_MS', 'BENCH_MODULE_MS'):
                    for kk in (k, 'GUEST_' + k):
                        if kk in d:
                            o.add(lane, 'cup8bench.' + k[6:].lower(), rep, num(d[kk]), 'ms')
                if 'BSUM' in ln:
                    n = d.get('N')
                    for f in BSUM_FIELDS:
                        v = num(d.get(f))
                        o.add(lane, f'cup8bench.N{n}.{f}', rep, v, UNITS.get(f, 'ms'))
                    b, h, dd = num(d.get('batch_per_launch_ms')), num(d.get('h2d_ms')), num(d.get('d2h_med_ms'))
                    if n and n.isdigit():
                        mat = int(n) * int(n) * 4
                        if int(n) >= 1024 and h and h > 0:
                            o.add(lane, f'derived.N{n}.h2d_GBps', rep, 2 * mat / (h / 1000.0) / 1e9, 'GB/s')
                        if int(n) >= 1024 and dd and dd > 0:
                            o.add(lane, f'derived.N{n}.d2h_GBps', rep, mat / (dd / 1000.0) / 1e9, 'GB/s')


def parse_batch_totals(o, raw):
    """BATCH_TOTAL_MS / BATCH from the raw cup8bench log: 3 decimals of a total over BATCH launches
    resolve 1/BATCH of a microsecond, where the BSUM line's per-launch figure resolves 1 us."""
    for fn in sorted(os.listdir(raw)):
        host = fn.startswith('cl_perf_') and '_host_cup8bench_' in fn and fn.endswith('.log')
        guest = fn.startswith('cl_guest_r') and fn.endswith('_cup8bench_probe.log')
        if not (host or guest):
            continue
        rep = rep_of(fn) if guest else int(re.search(r'cup8bench_(\d+)\.log$', fn).group(1))
        lane = 'ladder_host' if host else 'ladder_guest'
        tot, bat = {}, {}
        for ln in rd(os.path.join(raw, fn)):
            m = re.match(r'^(?:GUEST_)?B(\d+)_(BATCH_TOTAL_MS|BATCH)=([0-9.]+)\s*$', ln)
            if m:
                (tot if m.group(2) == 'BATCH_TOTAL_MS' else bat)[m.group(1)] = float(m.group(3))
        for n, t in tot.items():
            if bat.get(n):
                us = t / bat[n] * 1000.0
                o.add(lane, f'derived.N{n}.batch_launch_us', rep, us, 'us')
                if n == '16' and us > 0:
                    o.add(lane, 'derived.launch_rate_per_s.batched_precise', rep, 1e6 / us, 'launches/s')


DBX = re.compile(r'^DBX_(\w+?)(?:_r(\d+))? .*')


def parse_exit(o, raw):
    for fn in sorted(os.listdir(raw)):
        if not (fn.startswith('exit_') and fn.endswith('_probe.log')):
            continue
        mode = re.match(r'exit_(on|off)_', fn)
        if not mode:
            continue
        mode = mode.group(1)
        boot = rep_of(fn)
        for ln in rd(os.path.join(raw, fn)):
            m = re.match(r'^DBX_(probe|unknown|dummy_\w+)(?:_r(\d+))?\s', ln)
            if not m:
                continue
            d = kvs(ln)
            for q in ('p50_ns', 'p90_ns', 'p99_ns', 'mean_ns'):
                o.add('exit_' + mode, f'{m.group(1)}.{q}', boot, num(d.get(q)), 'ns')


def extract(outdir):
    raw = os.path.join(outdir, 'raw')
    o = Out()
    if os.path.isdir(raw):
        parse_fast(o, raw)
        parse_bare(o, raw)
        parse_ladder(o, raw, 'host')
        parse_ladder(o, raw, 'guest')
        parse_batch_totals(o, raw)
        parse_exit(o, raw)
    with open(os.path.join(outdir, 'metrics.tsv'), 'w') as f:
        f.write('lane\tmetric\trep\tvalue\tunit\n')
        for r in o.rows:
            f.write('\t'.join(r) + '\n')
    print(f'extract: {len(o.rows)} values -> {outdir}/metrics.tsv')


def load_metrics(outdir):
    d = {}
    for i, ln in enumerate(rd(os.path.join(outdir, 'metrics.tsv'))):
        if i == 0:
            continue
        p = ln.split('\t')
        if len(p) < 5:
            continue
        v = num(p[3])
        if v is not None:
            d.setdefault((p[0], p[1], p[4]), []).append(v)
    return d


def summary(outdir):
    d = load_metrics(outdir)
    rows = []
    for (lane, metric, unit), vs in sorted(d.items()):
        rows.append((lane, metric, unit, len(vs), statistics.median(vs), min(vs), max(vs)))
    with open(os.path.join(outdir, 'summary.tsv'), 'w') as f:
        f.write('lane\tmetric\tunit\tn\tmedian\tmin\tmax\n')
        for r in rows:
            f.write('\t'.join([r[0], r[1], r[2], str(r[3])] + [f'{x:.6g}' for x in r[4:]]) + '\n')
    with open(os.path.join(outdir, 'summary.md'), 'w') as f:
        cur = None
        for r in rows:
            if r[0] != cur:
                cur = r[0]
                f.write(f'\n### {cur}\n\n| metric | unit | n | median | min | max |\n|---|---|---|---|---|---|\n')
            f.write(f'| {r[1]} | {r[2]} | {r[3]} | {r[4]:.6g} | {r[5]:.6g} | {r[6]:.6g} |\n')
    print(f'summary: {len(rows)} metrics -> {outdir}/summary.tsv, summary.md')


def load_summary(outdir):
    d = {}
    for i, ln in enumerate(rd(os.path.join(outdir, 'summary.tsv'))):
        if i == 0:
            continue
        p = ln.split('\t')
        if len(p) == 7:
            d[(p[0], p[1])] = (p[2], int(p[3]), float(p[4]), float(p[5]), float(p[6]))
    return d


def compare(outdir, basedir, thresh=10.0):
    a, b = load_summary(basedir), load_summary(outdir)
    print(f'compare: BASE={basedir}  NEW={outdir}   (delta = NEW/BASE - 1 on medians; '
          f'"OUT" = also outside BASE min..max by >2%; listed only if |delta| >= {thresh:g}%)')
    print(f'{"lane":<14}{"metric":<44}{"unit":<10}{"base med":>12}{"new med":>12}{"delta%":>9}  note')
    shown = 0
    for k in sorted(set(a) | set(b)):
        if k not in a or k not in b:
            print(f'{k[0]:<14}{k[1]:<44}{"":<10}{"-" if k not in a else "":>12}{"-" if k not in b else "":>12}{"":>9}  '
                  f'{"only in NEW" if k not in a else "only in BASE"}')
            continue
        u, _, bm, bmin, bmax = a[k]
        _, _, nm, nmin, nmax = b[k]
        dl = (nm / bm - 1.0) * 100.0 if bm else float('nan')
        # OUT = the new median lies outside the baseline's min..max by more than 2% of the baseline median
        # (a zero-width baseline range would otherwise flag every last-digit difference)
        out = nm < bmin - 0.02 * abs(bm) or nm > bmax + 0.02 * abs(bm)
        if dl == dl and abs(dl) >= thresh:
            note = 'OUT' if out else 'within baseline spread'
            print(f'{k[0]:<14}{k[1]:<44}{u:<10}{bm:>12.5g}{nm:>12.5g}{dl:>8.1f}%  {note}')
            shown += 1
    print(f'compare: {shown} metric(s) listed of {len(set(a) & set(b))} common')


def main(argv):
    if len(argv) >= 3 and argv[1] == 'extract':
        extract(argv[2])
    elif len(argv) >= 3 and argv[1] == 'summary':
        summary(argv[2])
    elif len(argv) >= 4 and argv[1] == 'compare':
        compare(argv[2], argv[3], float(os.environ.get('PERF_DELTA_PCT', '10')))
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
