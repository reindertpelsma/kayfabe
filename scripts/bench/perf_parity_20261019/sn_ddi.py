#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Decode profiler entries/exits in CSV order over the same graphics-test window.

TID zero is processor-local interrupt context. Nonzero threads may migrate.
Entry counts do not depend on successful duration pairing; all losses remain visible.
"""
import collections
import json
from pathlib import Path
import pickle
import sys

base, tag = Path(sys.argv[1]), sys.argv[2]
if not (base / f'{tag}_ev.pkl').is_file():
    base /= tag
ev, _ = pickle.loads((base / f'{tag}_ev.pkl').read_bytes())
order = ev.get('_p1_profiler_order')
if order is not None:
    events = []
    for ordinal, (entered, index) in enumerate(order):
        e = ev['105' if entered else '106'][index]
        events.append((e[0], ordinal, int(entered), e[1], e[2], e[3], e[4]))
else:
    events = [(e[0], ordinal, entered, e[1], e[2], e[3], e[4])
              for ordinal, (entered, e) in enumerate(
                  [(1, e) for e in ev.get('105', [])] + [(0, e) for e in ev.get('106', [])])]
events.sort(key=lambda e: (e[0], e[1]))
metrics_path = base / f'{tag}_metrics.json'
metrics = json.loads(metrics_path.read_text()) if metrics_path.is_file() else {}
t0, t1 = metrics.get('window_filetime_100ns', [events[0][0] + 3e7, events[-1][0] - 3e7])
seconds = (t1 - t0) / 1e7
stacks = collections.defaultdict(list)
durations = collections.defaultdict(list)
entries = collections.Counter()
unmatched = mismatched = unmatched_window = mismatched_window = 0
for t, ordinal, entered, pid, tid, cpu, fn in events:
    context = (pid, tid, cpu if tid == 0 else None)
    stack = stacks[context]
    in_window = t0 <= t <= t1
    if entered:
        stack.append((t, fn))
        if in_window:
            entries[fn] += 1
    elif stack:
        start, function = stack.pop()
        if function != fn:
            mismatched += 1
            mismatched_window += in_window
            stack.clear()
        elif t0 <= start <= t1:
            durations[fn].append((t - start) / 10.0)
    else:
        unmatched += 1
        unmatched_window += in_window


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * fraction))]


functions = {}
for fn in sorted(set(entries) | set(durations), key=lambda f: -sum(durations[f])):
    values = durations[fn]
    frames = metrics.get('frames')
    functions[fn] = {
        'calls': len(values), 'entry_calls': entries[fn],
        'calls_per_s': round(len(values) / seconds, 1),
        'calls_per_frame': len(values) / frames if frames else None,
        'entry_calls_per_frame': entries[fn] / frames if frames else None,
        'med_us': round(percentile(values, .5), 2) if values else None,
        'p95_us': round(percentile(values, .95), 2) if values else None,
        'p99_us': round(percentile(values, .99), 2) if values else None,
        'max_us': round(max(values), 1) if values else None,
        'total_ms_per_s': round(sum(values) / 1e3 / seconds, 2),
    }
result = {'window_s': seconds, 'unmatched_exits': unmatched, 'mismatched_exits': mismatched,
          'unmatched_exits_in_window': unmatched_window, 'mismatched_exits_in_window': mismatched_window,
          'unclosed_entries': sum(map(len, stacks.values())),
          'unclosed_entries_in_window': sum(t0 <= t <= t1 for stack in stacks.values() for t, _ in stack),
          'csv_profiler_order_preserved': order is not None,
          'pairing': 'pid/thread; thread zero additionally keyed by processor; equal ticks retain CSV order',
          'functions': functions}
(base / f'{tag}_ddi.json').write_text(json.dumps(result, indent=1))
print('DDI_CONTEXT', json.dumps({k: v for k, v in result.items() if k != 'functions'}))
for fn, row in list(functions.items())[:14]:
    print(fn, row)
