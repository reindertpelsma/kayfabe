#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Summarize a private NSI dump, explicitly retaining trace/attribution limits."""
import argparse
import collections
import json
from pathlib import Path
import re


def analyze(path, calibration=None, window=None):
    text = path.read_text()
    if text.count('DIAG_NSI_TRACE_BEGIN') != 1 or not text.endswith('kf3: DIAG_NSI_TRACE_END\n'):
        raise ValueError('incomplete or repeated diagnostic dump')
    header = re.search(r'DIAG_NSI_TRACE_BEGIN (.*)', text)[1]
    counts = {key: int(value) for key, value in re.findall(r'(seen|retained|overwritten|contended|accounting_delta)=(\d+)', header)}
    records = []
    for line in text.splitlines():
        if 'DIAG_NSI_EDGE ' not in line:
            continue
        m = re.search(r'seq=(\d+) ns=(\d+) source=(.*?) slot=(.*?) vector=(.*?) armed=(.*?) verdict=(.*)$', line)
        if not m:
            raise ValueError('unrecognized diagnostic record')
        records.append({'seq': int(m[1]), 'ns': int(m[2]), 'source': m[3],
                        'slot': m[4], 'vector': m[5], 'armed': m[6], 'verdict': m[7]})
    if len(records) != counts['retained']:
        raise ValueError('record/header count mismatch')
    selected = records
    offset = None
    if calibration:
        clock = json.loads(calibration.read_text())['analysis']['channel_clock_offset_ns']
        if not clock:
            raise ValueError('no channel-relative clock calibration')
        offset = clock['median']
        for row in records:
            row['host_ns'] = int(row['ns'] + offset)
    if window:
        if offset is None:
            raise ValueError('a host window requires channel clock calibration')
        selected = [r for r in records if window[0] <= r['host_ns'] <= window[1]]
    combinations = collections.Counter((r['source'], r['vector'], r['armed'], r['verdict']) for r in selected)
    sequence = sorted(r['seq'] for r in records)
    duplicates = len(sequence) - len(set(sequence))
    holes = (sequence[-1] - sequence[0] + 1 - len(set(sequence))) if sequence else 0
    return {'trace': counts, 'selected_records': len(selected),
            'retained_relative_ns': [min(r['ns'] for r in records), max(r['ns'] for r in records)] if records else None,
            'selected_relative_ns': [min(r['ns'] for r in selected), max(r['ns'] for r in selected)] if selected else None,
            'duplicate_sequences': duplicates,
            'holes_between_retained_sequences': holes,
            'complete_retained_census': counts['contended'] == 0 and counts['accounting_delta'] == 0 and duplicates == 0 and holes == 0,
            'decisions': [dict(source=s, vector=v, armed=a, verdict=d, count=n)
                          for (s, v, a, d), n in combinations.most_common()],
            'limit': 'bounded tail; completion-fd mixes FIFO and configured host CE; host readiness may coalesce physical edges; trailing raises carry earlier sources'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('dump', type=Path)
    parser.add_argument('--calibration', type=Path)
    parser.add_argument('--window', nargs=2, type=int)
    args = parser.parse_args()
    print(json.dumps(analyze(args.dump, args.calibration, args.window), indent=2))


if __name__ == '__main__':
    main()
