#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Generate a read-only broker uprobe or summarize its new-content timestamps.

gen BROKER PID OFFSET: OFFSET is compiler-derived offsetof(struct nb_tick,
commits_since_present) from this broker's public header. Requires --stats-line.
analyze TRACE START_MONOTONIC_NS END_MONOTONIC_NS: no guest-present substitution.
"""
import argparse
import json
from pathlib import Path


def percentile(values, fraction):
    values = sorted(values)
    position = (len(values) - 1) * fraction
    low = int(position)
    high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (position - low)


def summarize(path, start, end):
    records = []
    for line in path.read_text().splitlines():
        parts = line.split()
        if len(parts) == 4 and parts[0] == 'B':
            _, observed, presented, commits = parts
            records.append((int(observed), int(presented), int(commits)))
    selected = [row for row in records if start <= row[1] <= end]
    content = sorted(p for _, p, n in selected if n > 0)
    gaps = [(b - a) / 1e6 for a, b in zip(content, content[1:])]
    seconds = (end - start) / 1e9
    if not gaps or seconds <= 0:
        raise ValueError('no usable new-content interval')
    return {
        'window_s': seconds, 'new_content_presents': len(content),
        'new_content_fps': len(content) / seconds,
        'repeated_presents_per_s': sum(n == 0 for _, _, n in selected) / seconds,
        'merged_commits': sum(max(0, n - 1) for _, _, n in selected),
        'one_percent_low_fps': 1000 / percentile(gaps, .99),
        'point_one_percent_low_fps': 1000 / percentile(gaps, .999),
        'new_content_gap_p99_ms': percentile(gaps, .99),
        'new_content_gap_p999_ms': percentile(gaps, .999),
        'new_content_gap_max_ms': max(gaps),
        'new_content_gaps_per_minute': {
            str(threshold): sum(gap > threshold for gap in gaps) * 60 / seconds
            for threshold in [33, 50, 100]
        },
        'trace_records_total': len(records), 'trace_records_selected': len(selected),
        'nonmonotonic_present_order': sum(b[1] <= a[1] for a, b in zip(records, records[1:])),
        'limit': 'first/last boundary gaps excluded; inspect BPF lost-event diagnostics separately',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('gen')
    p.add_argument('binary', type=Path)
    p.add_argument('pid', type=int)
    p.add_argument('offset', type=int)
    p = sub.add_parser('analyze')
    p.add_argument('trace', type=Path)
    p.add_argument('start', type=int)
    p.add_argument('end', type=int)
    args = parser.parse_args()
    if args.command == 'gen':
        if any(character in str(args.binary) for character in ':\n"'):
            parser.error('unsafe binary path')
        print('BEGIN { printf("# broker-content start %llu\\n", nsecs); }')
        print(f'uprobe:{args.binary}:nb_tick_content_present /pid == {args.pid}/ {{ '
              f'$n = *(uint64*)(arg0 + {args.offset}); '
              'printf("B %llu %llu %llu\\n", nsecs, arg1, $n); }')
        print('END { printf("# broker-content end %llu\\n", nsecs); }')
    else:
        print(json.dumps(summarize(args.trace, args.start, args.end), indent=2))


if __name__ == '__main__':
    main()
