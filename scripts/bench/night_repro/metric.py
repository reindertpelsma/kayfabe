#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""PresentMon event-QPC silence metric, separated by process lifetime.

A silence includes the whole gap from last present to next present (>=4s).
Arming requires >=100 presents in the preceding 30s, matching pmmon.cs.
Receipt timestamps are deliberately not treated as GPU present timestamps.
"""
import argparse
import bisect
import csv
import json
from collections import defaultdict
from pathlib import Path


def read_csv(lines):
    header = None
    rows, skipped = [], 0
    for row in csv.reader(lines):
        if not row:
            continue
        if row[0].lstrip('\ufeff') == 'Application':
            header = [v.lstrip('\ufeff') for v in row]
            continue
        if header is None:
            raise ValueError('PresentMon header absent')
        if len(row) != len(header) + 1:
            skipped += 1
            continue
        try:
            fields = dict(zip(header, row[1:]))
            rows.append((int(fields['TimeInQPC']), fields['Application'], int(fields['ProcessID'])))
        except (ValueError, KeyError):
            skipped += 1
    if header is None:
        raise ValueError('PresentMon header absent')
    return rows, skipped


def silences(rows, frequency, start_s, end_s, process_ends):
    if frequency <= 0 or end_s <= start_s:
        raise ValueError('invalid QPC window')
    per_process = defaultdict(list)
    for tick, app, pid in rows:
        t = tick / frequency
        if start_s <= t <= end_s:
            points = per_process[(app, pid)]
            if points and t < points[-1]:
                raise ValueError('nonmonotonic present QPC within one process')
            points.append(t)
    result = {}
    for (app, pid), points in per_process.items():
        r = result.setdefault(app, dict(count=0, total_s=0., longest_s=0., right_censored=0,
                                       presents=0, pids=[], intervals=[]))
        r['presents'] += len(points)
        r['pids'].append(pid)
        stop = min(end_s, process_ends.get(pid, end_s))
        for i, begin in enumerate(points):
            finish = min(points[i + 1], stop) if i + 1 < len(points) else stop
            duration = finish - begin
            if duration >= 4 and i + 1 - bisect.bisect_left(points, begin - 30, 0, i + 1) >= 100:
                censored = i + 1 == len(points) and pid not in process_ends
                r['count'] += 1
                r['total_s'] += duration
                r['longest_s'] = max(r['longest_s'], duration)
                r['right_censored'] += int(censored)
                r['intervals'].append(dict(pid=pid, begin_qpc=round(begin * frequency),
                    end_qpc=round(finish * frequency), duration_s=duration, right_censored=censored))
    return result


def analyze(directory):
    root = Path(directory)
    meta = json.loads((root / 'guest.json').read_text(encoding='utf-8-sig'))
    start = meta['start_qpc'] / meta['frequency']
    end = meta['end_qpc'] / meta['frequency']
    ends = {}
    for f in root.glob('life-*.tsv'):
        for line in f.read_text(encoding='utf-8-sig').splitlines():
            parts = line.split('\t')
            if len(parts) >= 3 and parts[0] == 'END':
                ends[int(parts[1])] = int(parts[2]) / meta['frequency']
    with (root / 'pm_live.csv').open(encoding='utf-8-sig', errors='replace') as f:
        rows, skipped = read_csv(f)
    apps = silences(rows, meta['frequency'], start, end, ends)
    expected = ['Heaven.exe', 'GravityMark.exe']
    valid = all(apps.get(a, {}).get('presents', 0) >= 100 for a in expected)
    return dict(schema=1, label='MEASURED', duration_s=end-start, apps=apps,
                expected_apps=expected, load_present=valid, malformed_rows=skipped,
                trigger_lines=(root/'trig.log').read_text(errors='replace').splitlines(),
                warning='Presentation silence is an observation, not proof of a GPU deadlock; terminal intervals are censored.')


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('directory')
    args = p.parse_args()
    print(json.dumps(analyze(args.directory), indent=2))
