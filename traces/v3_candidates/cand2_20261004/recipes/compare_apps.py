#!/usr/bin/env python3
"""Compare saved candidate-2 app evidence with candidate 1, including digests."""
from collections import Counter
from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]
base = root.parent / 'cand1_20261004/apps'
new = root / 'apps_guest/workspace/apps/results/cand2'


def rows(path):
    return {r['app']: r for line in path.read_text().splitlines()
            if line.startswith('APPRES ')
            for r in [dict(re.findall(r'(\w+)=((?:(?! \w+=).)*)', line[7:].strip()))]}


def digs(path):
    return {(m[2], m[3]): m[4] for line in path.read_text().splitlines()
            if (m := re.match(r'APPDIG side=(\S+) app=(\S+) (?:OUTSHA|DIGEST) (\S+) (\S+)', line))}


for filename in ['host.res', 'guest.res', 'guest_isolated.res']:
    before, after = rows(base/filename), rows(new/filename)
    assert before.keys() == after.keys(), (filename, before.keys() ^ after.keys())
    differences = [(app, before[app]['verdict'], after[app]['verdict'])
                   for app in before if before[app]['verdict'] != after[app]['verdict']]
    print(filename, 'counts', dict(Counter(r['verdict'] for r in after.values())),
          'verdict_changes', differences)
    assert not differences, differences
    for app, row in after.items():
        if filename.startswith('guest'):
            assert row.get('guest_xid', '0') == '0', (app, row)

host, guest = digs(new/'host.res'), digs(new/'guest.dig')
assert host.keys() <= guest.keys(), host.keys() - guest.keys()
assert all(guest[k] == value for k, value in host.items()), (host, guest)
assert digs(base/'guest.dig') == guest, (digs(base/'guest.dig'), guest)
print('Host/guest digests match:', len(host), '; guest digests unchanged:', len(guest))
# A misleading success string is an existing managed-memory limitation, not correctness.
for directory in [base, new]:
    text = (directory/'conjugateGradientUM.guest.log').read_text()
    assert 'Error amount = 1.000000, result = SUCCESS' in text, text
print('Known conjugateGradientUM wrong-answer/success defect persists in both candidates.')
print('APP_COMPARISON PASS unchanged baseline; not an all-app correctness claim')
