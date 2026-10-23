#!/usr/bin/env python3
"""Stamp SPECviewperf result tables as non-compliant research numbers.

Owner decision 2026-10-23: scores from our virtualised runs may be public only when every
number is labelled "(nc)" and the page carries the disclaimer below. Result tables must come
from this module, never from hand-typed text:

  render_csv / render_markdown / render_comparison   write a stamped table
  check_text / require_valid                         refuse a table that is not stamped
  python3 spec_stamp.py check FILE...                exit 1 if any file is not stamped
  python3 spec_stamp.py compare --release R --config C --arm NAME=scores.csv ...
                                                     [--ratios --baseline A --candidate B]

The SPECviewperf release and the configuration are always passed in; nothing is guessed. A
ratio column exists only with the explicit flag and is headed as a research comparison.
"""
import argparse
import csv
import io
import math
from pathlib import Path
import re
import sys

DISCLAIMER = (
    '(nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and '
    'not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance '
    'Evaluation Corporation, https://www.spec.org')
NC = '(nc)'
RESEARCH = '(nc) research comparison, not a SPEC-ranked result'
RELEASE_RE = re.compile(
    r'^SPECviewperf \d+\.(?:\d+|x)(?:\.\d+)?(?: \(patch level not recorded\))?$')

RATIO_HEADER_RE = re.compile(r'(?i)\bratio\b|\bover\b')
SCORE_HEADER_RE = re.compile(r'(?i)composite|fps|score|ratio|\bover\b')
NUMBER_RE = re.compile(r'(?<![\w.\-])-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?')
LABEL_TAIL_RE = re.compile(r'[%x]? ' + re.escape(NC))
VERSION_RE = re.compile(r'\d+\.\d+\.\d+')
TIME_RE = re.compile(
    r'\d{4}-\d\d-\d\d(?:[T ]\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)?)?'
    r'|\b\d\d:\d\d:\d\d(?:\.\d+)?')
RELEASE_LINE_RE = re.compile(r'^[#> ]*Release:\s*(.+?)\s*$', re.MULTILINE)


class StampError(ValueError):
    """A table is not stamped as a non-compliant research number table."""


def check_release(release):
    """The release must be passed in as an exact name; guessing is refused."""
    if not isinstance(release, str) or not RELEASE_RE.match(release):
        raise ValueError(
            'SPECviewperf release must be given exactly, e.g. "SPECviewperf 15.0.1" or '
            '"SPECviewperf 15.x (patch level not recorded)", got %r' % (release,))
    return release


def check_config(config):
    if not isinstance(config, str) or not config.strip() or '\n' in config:
        raise ValueError('the configuration statement must be one non-empty line')
    return config.strip()


def label(value):
    """A score with the non-compliance suffix; a missing score stays blank."""
    if value is None:
        return ''
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError('score must be a number, got %r' % (value,))
    if not math.isfinite(value):
        raise ValueError('score must be finite')
    return '%s %s' % (value, NC)


def header_lines(release, config):
    return ['Release: ' + check_release(release), 'Configuration: ' + check_config(config),
            DISCLAIMER]


def _cells(row, fields, score_fields):
    return [label(row.get(f)) if f in score_fields else
            ('' if row.get(f) is None else str(row.get(f))) for f in fields]


def render_csv(rows, fields, score_fields, release, config):
    out = io.StringIO()
    for line in header_lines(release, config):
        out.write('# %s\n' % line)
    writer = csv.writer(out, lineterminator='\n')
    writer.writerow(fields)
    for row in rows:
        writer.writerow(_cells(row, fields, score_fields))
    return out.getvalue()


def _md_cell(text):
    return text.replace('|', '\\|')


def render_markdown(title, rows, fields, score_fields, release, config):
    lines = ['## ' + title, '']
    lines += ['> ' + line for line in header_lines(release, config)]
    lines += ['', '| ' + ' | '.join(_md_cell(f) for f in fields) + ' |',
              '|' + '|'.join(' --- ' for _ in fields) + '|']
    for row in rows:
        cells = [_md_cell(c) for c in _cells(row, fields, score_fields)]
        lines.append('| ' + ' | '.join(cells) + ' |')
    return '\n'.join(lines) + '\n'


def ratio_header(candidate, baseline):
    return 'ratio %s/%s -- %s' % (candidate, baseline, RESEARCH)


def render_comparison(arms, release, config, kind='md', ratios=False, baseline=None,
                      candidate=None, title='Composite comparison'):
    """Arms are {name: {workload: composite or None}}. Ratios only with ratios=True."""
    if ratios:
        if baseline not in arms or candidate not in arms or baseline == candidate:
            raise ValueError('--ratios needs two different arms: baseline and candidate')
    names = list(arms)
    workloads = sorted({w for arm in arms.values() for w in arm})
    fields = ['workload'] + ['%s composite' % n for n in names]
    if ratios:
        fields.append(ratio_header(candidate, baseline))
    rows = []
    for workload in workloads:
        row = {'workload': workload}
        for name in names:
            row['%s composite' % name] = arms[name].get(workload)
        if ratios:
            base, cand = arms[baseline].get(workload), arms[candidate].get(workload)
            usable = base is not None and cand is not None and base > 0
            row[fields[-1]] = round(cand / base, 2) if usable else None
        rows.append(row)
    score_fields = fields[1:]
    if kind == 'csv':
        return render_csv(rows, fields, score_fields, release, config)
    return render_markdown(title, rows, fields, score_fields, release, config)


def _table_rows(text, kind):
    """The header row and body rows of the table, without the stamped header block."""
    if kind == 'csv':
        body = [ln for ln in text.splitlines() if not ln.startswith('#')]
        table = list(csv.reader(body))
    else:
        table = []
        for line in text.splitlines():
            if line.startswith('|'):
                cells = [c.strip() for c in re.split(r'(?<!\\)\|', line.strip())[1:-1]]
                if not all(re.fullmatch(r':?-{3,}:?', c) for c in cells):
                    table.append(cells)
    return (table[0], table[1:]) if table else ([], [])


def check_text(text, kind):
    """Problems that make a table unfit to publish; an empty list means stamped."""
    problems = []
    if DISCLAIMER not in text:
        problems.append('the disclaimer is missing or altered')
    found = RELEASE_LINE_RE.search(text)
    try:
        check_release(found.group(1) if found else None)
    except ValueError:
        problems.append('no exact SPECviewperf release line')
    header, body = _table_rows(text, kind)
    for column in header:
        if RATIO_HEADER_RE.search(column) and RESEARCH not in column:
            problems.append('ratio column %r is not headed as a research comparison' % column)
    for row in body:
        for index, cell in enumerate(row):
            head = header[index] if index < len(header) else ''
            if VERSION_RE.fullmatch(cell):
                continue
            cell = TIME_RE.sub('TIME', cell)
            for found_number in NUMBER_RE.finditer(cell):
                token = found_number.group(0)
                floaty = '.' in token or 'e' in token.lower()
                labelled = LABEL_TAIL_RE.match(cell[found_number.end():]) is not None
                if (floaty or SCORE_HEADER_RE.search(head)) and not labelled:
                    problems.append('unlabelled number %s in column %r' % (token, head))
    return problems


def require_valid(text, kind):
    problems = check_text(text, kind)
    if problems:
        raise StampError('; '.join(problems))
    return text


def read_scores(path):
    """Mean completed composite per workload from a stamped scores CSV."""
    lines = [ln for ln in Path(path).read_text().splitlines() if not ln.startswith('#')]
    values = {}
    for row in csv.DictReader(lines):
        text = (row.get('composite') or '').replace(' ' + NC, '').strip()
        if row.get('workload_status') == 'Completed' and text:
            values.setdefault(row['workload'], []).append(float(text))
    return {w: round(sum(v) / len(v), 4) for w, v in values.items()}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    check = sub.add_parser('check', help='fail unless every file is stamped')
    check.add_argument('files', nargs='+', type=Path)
    compare = sub.add_parser('compare', help='stamped composite comparison of stamped scores')
    compare.add_argument('--release', required=True)
    compare.add_argument('--config', required=True)
    compare.add_argument('--arm', action='append', required=True, metavar='NAME=SCORES.csv')
    compare.add_argument('--ratios', action='store_true',
                         help='add a ratio column, headed as a non-compliant research comparison')
    compare.add_argument('--baseline')
    compare.add_argument('--candidate')
    compare.add_argument('--csv', action='store_true', help='write csv instead of markdown')
    compare.add_argument('--output', type=Path)
    args = parser.parse_args(argv)
    if args.command == 'check':
        status = 0
        for path in args.files:
            kind = 'csv' if path.suffix == '.csv' else 'md'
            for problem in check_text(path.read_text(), kind):
                print('%s: %s' % (path, problem))
                status = 1
        return status
    arms = {}
    for spec in args.arm:
        name, _, path = spec.partition('=')
        arms[name] = read_scores(path)
    text = render_comparison(arms, args.release, args.config, 'csv' if args.csv else 'md',
                             args.ratios, args.baseline, args.candidate)
    if args.output:
        args.output.write_text(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
