#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Read an existing VFIO-format trace (including kf3 diagnostic traces). No VM actions."""
import argparse
import collections
import gzip
import json
import pathlib
import re

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('trace', type=pathlib.Path)
parser.add_argument('output', type=pathlib.Path)
args = parser.parse_args()
open_trace = gzip.open if args.trace.suffix == '.gz' else open
counts = collections.Counter()
windows = collections.Counter()
current = []
with open_trace(args.trace, 'rt') as source:
    for line in source:
        match = re.search(r'vfio_region_(read|write)\s+\([^)]*:region0\+0x(b81[0-9a-f]{3}), (.*)', line)
        if not match:
            continue
        kind, offset, rest = match.groups()
        value = re.search(r'= 0x([0-9a-f]+)', rest) if kind == 'read' else re.match(r'0x([0-9a-f]+)', rest)
        if value is None:
            raise ValueError('malformed register value: ' + line)
        counts[(kind, offset)] += 1
        event = (kind, '0x' + offset, hex(int(value[1], 16)))
        if event == ('write', '0xb81408', '0x101'):
            if current:
                windows[tuple(current)] += 1
            current = []
        if current or event == ('write', '0xb81408', '0x101'):
            current.append(event)
result = {
    'source': str(args.trace),
    'records': sum(counts.values()),
    'counts': [{'kind': k, 'offset': '0x'+o, 'n': n} for (k, o), n in sorted(counts.items())],
    'common_delimiter_windows': [{'n': n, 'events': events} for events, n in windows.most_common(5)],
    'limits': ['VFIO-format event names do not establish a hardware VFIO source',
               'global EN_CLEAR(2) delimiters may interleave guest CPUs; last partial window excluded',
               'absence of a read is not proof that the driver cannot read that register']
}
args.output.write_text(json.dumps(result, indent=2) + '\n')
