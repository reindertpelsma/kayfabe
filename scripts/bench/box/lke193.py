#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Issue #305: a read-only, evidence-seeded count ratchet and native RM die identification.

STATUS: LIVE, 2026-11-08 (GPU-free tests). No baseline is learned or raised by a run.
An unseeded die is UNKNOWN, not an assertion that its dumps are harmless.
"""
import argparse
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent


def validate_table(table):
    if not isinstance(table, dict):
        raise ValueError('ratchet must be a die map')
    for die, row in table.items():
        if not re.fullmatch(r'[A-Z]{2}[0-9]{3}', die) or not isinstance(row, dict):
            raise ValueError('invalid die or entry')
        count = row.get('max_count')
        evidence = row.get('source_evidence', {})
        if type(count) is not int or not 0 <= count <= 0x7fffffff:
            raise ValueError('invalid max_count')
        if not (isinstance(evidence, dict) and isinstance(evidence.get('path'), str)
                and evidence['path'] and not Path(evidence['path']).is_absolute()
                and '..' not in Path(evidence['path']).parts
                and re.fullmatch(r'[0-9a-f]{7,40}', str(evidence.get('revision','')))
                and re.fullmatch(r'\d{4}-\d{2}-\d{2}', str(evidence.get('date','')))):
            raise ValueError('ratchet entry needs source evidence, revision and date')
    return table


def load_ratchet(path=None):
    return validate_table(json.loads(Path(path or HERE / 'known_live_dumps_193.json').read_text()))


def decide(die, count, table=None):
    table = load_ratchet() if table is None else validate_table(table)
    if type(count) is not int or not 0 <= count <= 0x7fffffff:
        raise ValueError('missing or invalid live dump count')
    row = table.get(die)
    maximum = row['max_count'] if row else None
    verdict = 'UNKNOWN' if row is None else ('FAIL' if count > maximum else 'PASS')
    return dict(die=die, count=count, max_count=maximum, verdict=verdict,
                source_evidence=row['source_evidence'] if row else None)


def identify(probe, root):
    # Same unprivileged native tuple as kf-chip::DieId, not a marketing-name/PCI table.
    lines = re.findall(r'^host_driver=\d+\.\d+\.\d+ arch=\((\d+), (\d+), (\d+)\)$', probe, re.M)
    if sum(line.startswith('host_driver=') for line in probe.splitlines()) != len(lines):
        raise ValueError('malformed native identity')
    if len(lines) != 1 or probe.splitlines().count('PASS native read-only timer map/read/drop x3') != 1:
        raise ValueError('one complete native RM identity required')
    arch, implementation, revision = map(int, lines[0])
    if max(arch, implementation, revision) > 0xffffffff:
        raise ValueError('native tuple exceeds u32')
    root = Path(root)
    arch_text = (root / 'crates/kf-chip/src/lib.rs').read_text()
    die_text = (root / 'crates/kf-chip/src/die.rs').read_text()
    arches = {name:int(value,16) for name,value in re.findall(r'pub const (\w+): u32 = (0x[0-9A-Fa-f]+);',arch_text)}
    block = re.search(r'pub const DIE_NAMES:.*?= &\[(.*?)\n\];',die_text,re.S)
    if block is None: raise ValueError('derived die table missing')
    entry = r'\(arch::(\w+), (0x[0-9a-f]+), "([A-Z0-9]+)"\)'
    entries = re.findall(entry,block[1])
    if not entries or re.sub(entry,'',block[1]).replace(',','').strip():
        raise ValueError('derived die table not fully parsed')
    names = {}
    for family, imp, name in entries:
        key = (arches[family],int(imp,16))
        if key in names: raise ValueError('ambiguous derived die tuple')
        names[key] = name
    return names.get((arch,implementation),'unknown')


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    subs = p.add_subparsers(dest='command',required=True)
    ident = subs.add_parser('identify'); ident.add_argument('probe'); ident.add_argument('root')
    check = subs.add_parser('check'); check.add_argument('--die',required=True); check.add_argument('--count',required=True)
    check.add_argument('--table'); check.add_argument('--out',required=True)
    args = p.parse_args(argv)
    try:
        if args.command == 'identify':
            print(identify(Path(args.probe).read_text(),args.root)); return 0
        if not re.fullmatch(r'(?:[A-Z]{2}[0-9]{3}|unknown)',args.die): raise ValueError('invalid die')
        if not re.fullmatch(r'[0-9]{1,10}',args.count): raise ValueError('invalid count')
        row = decide(args.die,int(args.count),load_ratchet(args.table))
        Path(args.out).write_text(json.dumps(row,sort_keys=True)+'\n')
        print('GATE_LKE_RATCHET die=%s count=%d max=%s verdict=%s' %
              (row['die'],row['count'],row['max_count'] if row['max_count'] is not None else '-',row['verdict']))
        return 1 if row['verdict'] == 'FAIL' else 0
    except (OSError,ValueError,KeyError) as error:
        print('GATE_LKE_RATCHET_ERROR '+str(error),file=sys.stderr); return 2


if __name__ == '__main__': sys.exit(main())
