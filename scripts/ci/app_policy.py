#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Explicit non-gating app rows. CODE READ policy, 2026-11-08; never a measured verdict.

The Windows profiles and Linux known_unsupported list are the sources of truth.
Every structured report and app-stage log lists these entries, even if no app ran.
"""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]


def linux_unsupported():
    return json.loads((ROOT / 'scripts/apps/known_unsupported.json').read_text())['known_unsupported']


def rows(os_name=None):
    prof = json.loads((ROOT / 'scripts/bench/windows/appmatrix/gate_profiles.json').read_text())
    out = []
    for item in prof['manual']['items']:
        if item['id'] in prof['manual']['apps'] and item['reason'] == 'needs real display':
            out.append(dict(os='windows', name=item['id'], result='NOT_RUN', note='manual: needs real display'))
    for name in prof['known_unsupported']:
        out.append(dict(os='windows', name=name, result='NOT_RUN', note='unsupported (UVM)'))
    for name in linux_unsupported():
        out.append(dict(os='linux', name=name, result='NOT_RUN', note='unsupported (UVM)'))
    return [r for r in out if os_name is None or r['os'] == os_name]


def apply(collector):
    """Keep observations in notes; policy is never a pass or a supported-app failure."""
    for row in rows():
        store = collector.apps if row['os'] == 'linux' else collector.windows_apps
        observed = store.get(row['name'])
        note = row['note']
        if observed:
            note += '; observed %s: %s' % (observed[0], observed[2])
        store[row['name']] = ('NOT_RUN', None, note)
        key = (row['os'], 'apps_' + row['os'], row['name'])
        if key in collector.tests:
            collector.tests[key]['result'] = 'SKIP'


def main(argv):
    if len(argv) != 2 or argv[1] not in ('linux', 'windows'):
        return 2
    for row in rows(argv[1]):
        print('APP_POLICY name={name} os={os} result={result} note={note}'.format(**row))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
