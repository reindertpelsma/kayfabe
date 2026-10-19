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
    source = path.read_text()
    if 'X CLOCK_ONLY' in source:
        raise ValueError('native presentation clock was unavailable')
    records = []
    for line in source.splitlines():
        parts = line.split()
        if len(parts) == 4 and parts[0] == 'B':
            _, observed, presented, commits = parts
            records.append((int(observed), int(presented), int(commits)))
    selected = [row for row in records if start <= row[1] <= end]
    unique = {}
    for observed, presented, commits in selected:
        old = unique.get(presented, (observed, 0))
        unique[presented] = (min(observed, old[0]), commits + old[1])
    presentations = [(observed, presented, commits) for presented, (observed, commits) in sorted(unique.items())]
    content = [p for _, p, n in presentations if n > 0]
    gaps = [(b - a) / 1e6 for a, b in zip(content, content[1:])]
    seconds = (end - start) / 1e9
    if not gaps or seconds <= 0:
        raise ValueError('no usable new-content interval')
    return {
        'window_s': seconds, 'new_content_presents': len(content),
        'new_content_fps': len(content) / seconds,
        'repeated_presents_per_s': sum(n == 0 for _, _, n in presentations) / seconds,
        'merged_commits': sum(max(0, n - 1) for _, _, n in presentations),
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
        'unique_presented_refreshes': len(presentations),
        'duplicate_feedback_records': len(selected) - len(presentations),
        'feedback_timestamps_after_observation': sum(p > o for o, p, _ in selected),
        'max_reported_future_us': max([0] + [(p - o) / 1000 for o, p, _ in selected]),
        'nonmonotonic_present_order': sum(b[1] <= a[1] for a, b in zip(records, records[1:])),
        'limit': 'first/last boundary gaps excluded; feedback at an identical display timestamp is one refresh; inspect BPF lost-event diagnostics separately',
    }


def legacy_program(binary, pid, native_clock=False):
    """Observe production Wayland feedback and accepted commits, without a broker rebuild.

    Public 0452613b: pres_presented = VMM buffer feedback, pump_presented =
    same-buffer pump; nb_tick_refresh supplies the checked CLOCK_MONOTONIC timestamp.
    nb_tick_answer returns 0/1/2 for accepted replies and 3 for a violation.
    """
    prefix = f'uprobe:{binary}'
    ret = f'uretprobe:{binary}'
    guard = f'pid == {pid}'
    raw = (' @raw[tid] = ((((uint64)arg2 & 4294967295) << 32) | (arg3 & 4294967295)) * 1000000000 + (arg4 & 4294967295); '
           'if ((arg4 & 4294967295) >= 1000000000) { @raw[tid] = 0; }') if native_clock else ''
    invalid_clock = '@raw[tid] == 0' if native_clock else '(arg4 & 536870912) != 0'
    stamp = '@raw[tid]' if native_clock else 'arg2'
    return '\n'.join([
        'BEGIN { printf("# legacy broker-content start %llu\\n", nsecs); }',
        f'{prefix}:pres_presented /{guard}/ {{ @kind[tid] = 1; {raw} }}',
        f'{ret}:pres_presented /{guard}/ {{ delete(@kind[tid]); delete(@raw[tid]); }}',
        f'{prefix}:pump_presented /{guard}/ {{ @kind[tid] = 2; {raw} }}',
        f'{ret}:pump_presented /{guard}/ {{ delete(@kind[tid]); delete(@raw[tid]); }}',
        f'{prefix}:nb_tick_answer /{guard}/ {{ @tick[tid] = arg0; @commit[tid] = arg2 & 1; }}',
        f'{ret}:nb_tick_answer /{guard}/ {{ '
        'if (@commit[tid] && retval < 3) { @pending[@tick[tid]] = @pending[@tick[tid]] + 1; } '
        'delete(@tick[tid]); delete(@commit[tid]); }',
        f'{prefix}:nb_tick_refresh /{guard} && @kind[tid]/ {{ '
        f'if ({invalid_clock}) {{ printf("X CLOCK_ONLY %llu\\n", nsecs); }} '
        'else { $n = (uint64)0; if (@kind[tid] == 1) { $n = @pending[arg0]; @pending[arg0] = 0; } '
        f'printf("B %llu %llu %llu\\n", nsecs, {stamp}, $n); }} }}',
        'END { printf("# legacy broker-content end %llu\\n", nsecs); }',
    ])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('gen')
    p.add_argument('binary', type=Path)
    p.add_argument('pid', type=int)
    p.add_argument('offset', type=int)
    p.add_argument('--legacy', action='store_true')
    p.add_argument('--native-clock', type=int, choices=[1])
    p = sub.add_parser('analyze')
    p.add_argument('trace', type=Path)
    p.add_argument('start', type=int)
    p.add_argument('end', type=int)
    args = parser.parse_args()
    if args.command == 'gen':
        if any(character in str(args.binary) for character in ':\n"'):
            parser.error('unsafe binary path')
        if args.legacy:
            print(legacy_program(args.binary, args.pid, args.native_clock == 1))
            return
        print('BEGIN { printf("# broker-content start %llu\\n", nsecs); }')
        print(f'uprobe:{args.binary}:nb_tick_content_present /pid == {args.pid}/ {{ '
              f'$n = *(uint64*)(arg0 + {args.offset}); '
              'printf("B %llu %llu %llu\\n", nsecs, arg1, $n); }')
        print('END { printf("# broker-content end %llu\\n", nsecs); }')
    else:
        print(json.dumps(summarize(args.trace, args.start, args.end), indent=2))


if __name__ == '__main__':
    main()
