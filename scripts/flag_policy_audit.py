#!/usr/bin/env python3
"""Audit all KF3 lexical names, including indirect env reads, against the reviewed policy.

The superset deliberately includes protocol constants and historical launcher names.
Those rows are labelled separately; they are not counted as live environment reads.
"""
import csv
from pathlib import Path
import re
import sys


def names_in(text):
    return set(re.findall(r'\bKF3_[A-Z0-9_]+\b', text))


def missing(names, registered):
    return set(names) - set(registered)


def read_table(path):
    with Path(path).open() as stream:
        rows = list(csv.DictReader(stream, delimiter='\t'))
    table = {row['name']: row for row in rows}
    if len(table) != len(rows):
        raise ValueError('duplicate flag-policy name')
    return table


def inventory(root):
    root = Path(root)
    paths = list(root.glob('crates/kf-*/src/**/*.rs'))
    paths += list((root / 'qemu/hw/misc/kf3').glob('*.[ch]'))
    paths += list((root / 'broker').rglob('*.[ch]'))
    paths += [p for p in (root / 'scripts').rglob('*')
              if p.suffix in {'.sh', '.py'} and 'tests' not in p.parts
              and p.name not in {'flag_policy_audit.py', 'mutation_result.py'}]
    found = {}
    for path in sorted(set(paths)):
        for number, line in enumerate(path.read_text(errors='replace').splitlines(), 1):
            for name in names_in(line):
                found.setdefault(name, []).append(f'{path.relative_to(root)}:{number}')
    return found


def main():
    root = Path(__file__).resolve().parents[1]
    table = read_table(root / 'docs/design/V3_FLAG_POLICY_BETA_20261020.tsv')
    unknown = missing(inventory(root), table)
    if unknown:
        print('FAIL unregistered KF3 names: ' + ', '.join(sorted(unknown)))
        return 1
    print(f'PASS {len(table)} policy names cover source and launchers')
    return 0


if __name__ == '__main__':
    sys.exit(main())
