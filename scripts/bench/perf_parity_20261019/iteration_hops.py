#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Join tiny-probe iterations by fence VALUE, with no latency/gap selection.

The latest-changing candidate at the same fence value is the completion-cell
hypothesis (the earlier one is a submit-side copy in prior evidence). Report all
candidate lags and coverage; no fixed IRQ, irqfd, doorbell gap or period window.
The raw host fence observer is still an observation, not a GPU execution clock.
"""
import argparse
import bisect
import collections
import csv
import json
from pathlib import Path
import re
import statistics


def stats(values):
    if not values:
        return None
    values = sorted(values)
    return {'n': len(values), 'median': statistics.median(values),
            'min': values[0], 'max': values[-1],
            'p99': values[min(len(values) - 1, int(len(values) * .99))]}


def analyze(run, trace):
    with (run / 'raw.csv').open(encoding='utf-8-sig') as source:
        guest = list(csv.DictReader(source))
    total = len(guest)
    if any(int(row['i']) != i for i, row in enumerate(guest)):
        raise ValueError('guest raw iteration indices are incomplete')
    guest_log = (run / 'guest.txt').read_text()
    base_match = re.search(r'T_FENCE_BASE 0x([0-9a-fA-F]+)', guest_log)
    if not base_match:
        raise ValueError('guest fence base was not recorded')
    fence_base = int(base_match[1], 16)
    fences = collections.defaultdict(dict)
    for line in (trace / 'fence.txt').read_text().splitlines():
        p = line.split()
        if len(p) == 4 and p[0] == 'F':
            value = int(p[3], 16)
            i = value - fence_base - 201
            if 0 <= i < total:
                fences[int(p[2])].setdefault(i, int(p[1]))
    if not fences:
        raise ValueError('no fence values match the probe')
    eligible = {cell: values for cell, values in fences.items() if len(values) >= max(100, total * .9)}
    if not eligible:
        raise ValueError('no candidate has 90% fence-value coverage')
    common = set.intersection(*(set(v) for v in eligible.values()))
    if len(common) < 100:
        raise ValueError('too little common fence-value coverage')
    lag = {cell: statistics.median(v[i] - min(other[i] for other in eligible.values()) for i in common)
           for cell, v in eligible.items()}
    cell = max(lag, key=lambda c: (lag[c], -c))
    observed = fences[cell]
    frequency = re.search(r'QPCfreq (\d+)', guest_log)
    if not frequency:
        raise ValueError('guest QPC frequency was not recorded')
    frequency = int(frequency[1])
    offsets = [observed[i] - int(guest[i]['qpc_fence_seen']) * 1_000_000_000 / frequency
               for i in observed if i >= 5]
    mmio, markers, gpu, writes, latch_writes, channel_offsets = [], [], [], [], [], []
    for line in (trace / 'trace.txt').read_text().splitlines():
        p = line.split()
        if not p:
            continue
        if p[0] == 'M' and p[3] == '2':
            mmio.append((int(p[1]), int(p[4], 16) & 0xffffff, int(p[2])))
        elif p[0] == 'U' and p[3:5] == ['pub_marker', 'E']:
            markers.append(int(p[1]))
        elif p[0] == 'Q' and any('nvidia' in s.lower() for s in p[4:]):
            gpu.append(int(p[1]))
        elif p[0] == 'J':
            latch_writes.append(int(p[3].split('=')[1]))
        elif p[0] == 'K':
            writes.append((int(p[1]), int(p[-2].split('=')[1]), p[3]))
        elif p[0] == 'T':
            channel_offsets.append(int(p[1]) - int(p[3].split('=')[1]))
    irqfds = set(latch_writes)
    if len(irqfds) != 1:
        raise ValueError(f'latch-scoped irqfd was not unique: {sorted(irqfds)}')
    irqfd = next(iter(irqfds))
    mmio.sort(); markers.sort(); gpu.sort(); writes.sort()
    mt = [m[0] for m in mmio]
    rows = collections.defaultdict(list)
    exclusions = collections.Counter()
    def between(sequence, low, high):
        return sequence[bisect.bisect_right(sequence, low):bisect.bisect_right(sequence, high)]
    for i in range(5, total):
        if i - 1 not in observed or i not in observed:
            exclusions['missing_adjacent_fence_value'] += 1
            continue
        low, high = observed[i - 1], observed[i]
        if high <= low:
            exclusions['nonmonotonic_fence_observation'] += 1
            continue
        segment = mmio[bisect.bisect_right(mt, low):bisect.bisect_right(mt, high)]
        db = [t for t, offset, _ in segment if offset == 0xbb0090]
        if not db:
            exclusions['no_doorbell_between_adjacent_fence_values'] += 1
            continue
        d0 = db[0]
        interrupts = [t for t, offset, _ in segment if offset >> 12 == 0xb81]
        gi = between(gpu, d0, high)
        pm = between(markers, d0, high)
        irq = [(t, comm) for t, fd, comm in writes if fd == irqfd and d0 <= t <= high]
        rows['doorbells_per_iteration'].append(len(db))
        rows['interrupt_tree_writes_per_iteration'].append(len(interrupts))
        rows['isr_cycles_per_iteration'].append(sum(offset == 0xb81408 for _, offset, _ in segment))
        rows['irqfd_writes_per_iteration'].append(len(irq))
        rows['worker_irqfd_writes_per_iteration'].append(sum(comm.startswith('kf3-worker') for _, comm in irq))
        rows['fence_to_fence_us'].append((high - low) / 1000)
        rows['last_doorbell_us_from_first'].append((db[-1] - d0) / 1000)
        if pm:
            rows['last_marker_us_from_first_doorbell'].append((pm[-1] - d0) / 1000)
        else:
            exclusions['accepted_iteration_without_marker'] += 1
        if gi and pm:
            rows['first_gpu_irq_minus_last_marker_us'].append((gi[0] - pm[-1]) / 1000)
            rows['host_gpu_irqs_per_iteration'].append(len(gi))
        else:
            exclusions['accepted_iteration_without_gpu_irq_or_marker'] += 1
        if interrupts:
            rows['isr_chain_us'].append((interrupts[-1] - interrupts[0]) / 1000)
    return {'iterations_total': total, 'warmup_iterations_excluded': 5,
            'completion_candidate': cell, 'candidate_common_values': len(common),
            'candidate_median_lag_ns': lag,
            'candidate_values': {c: len(v) for c, v in fences.items()},
            'actual_irqfd': irqfd, 'exclusions': dict(exclusions),
            'guest_qpc_frequency': frequency,
            'fence_base': hex(fence_base),
            'host_minus_guest_qpc_ns': stats(offsets),
            'channel_clock_offset_ns': stats(channel_offsets),
            'metrics': {name: stats(values) for name, values in rows.items()},
            'limit': 'GPU IRQ is host-wide; last-marker/first-IRQ may overlap; no GPU-start timestamp'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run', type=Path)
    parser.add_argument('trace', type=Path)
    args = parser.parse_args()
    print(json.dumps(analyze(args.run, args.trace), indent=2))


if __name__ == '__main__':
    main()
