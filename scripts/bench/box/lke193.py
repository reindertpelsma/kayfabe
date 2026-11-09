#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Issue #305: a read-only, evidence-seeded count ratchet and native RM die identification.

STATUS: LIVE, 2026-11-09 (GPU-free tests). Owner keeps the ratchet report-only by default;
explicit gate mode retains the old ceiling enforcement. No baseline is learned or raised by a run.
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
        if row.get('count_unit') != 'distinct_reports':
            raise ValueError('ratchet limit must use distinct_reports, not event entries')
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


def decide(die, count, table=None, mode='report', entries=None, unresolved=None, resolved=None):
    table = load_ratchet() if table is None else validate_table(table)
    if type(count) is not int or not 0 <= count <= 0x7fffffff:
        raise ValueError('missing or invalid live dump count')
    for value in (entries, unresolved, resolved):
        if value is not None and (type(value) is not int or not 0 <= value <= 0x7fffffff):
            raise ValueError('invalid raw or unresolved entry count')
    if entries is not None and (entries < count or (unresolved is not None and unresolved > entries)):
        raise ValueError('report/entry counts are inconsistent')
    legacy_lower = max(0, count - (unresolved or 0))
    if resolved is not None and not legacy_lower <= resolved <= count:
        raise ValueError('resolved count is inconsistent with report/entry counts')
    lower = legacy_lower if resolved is None else resolved
    row = table.get(die)
    maximum = row['max_count'] if row else None
    excess = 'FAIL' if mode == 'gate' else 'WARN'
    if row is None:
        verdict = 'UNKNOWN'
    elif lower > maximum:
        verdict = excess
    elif unresolved or lower < count:
        verdict = 'UNKNOWN'
    else:
        verdict = 'PASS'
    return dict(die=die, count=count, max_count=maximum, verdict=verdict, unit='distinct_reports',
                entries=entries, unresolved_entries=unresolved,
                resolved_count=resolved, resolved_lower_bound=lower,
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
    check.add_argument('--mode',choices=('report','gate'),default='report')
    check.add_argument('--entries',default='-'); check.add_argument('--unresolved',default='-')
    check.add_argument('--resolved',default='-')
    args = p.parse_args(argv)
    try:
        if args.command == 'identify':
            print(identify(Path(args.probe).read_text(),args.root)); return 0
        if not re.fullmatch(r'(?:[A-Z]{2}[0-9]{3}|unknown)',args.die): raise ValueError('invalid die')
        if not re.fullmatch(r'[0-9]{1,10}',args.count): raise ValueError('invalid count')
        for value in (args.entries,args.unresolved,args.resolved):
            if value != '-' and not re.fullmatch(r'[0-9]{1,10}',value): raise ValueError('invalid entry count')
        row = decide(args.die,int(args.count),load_ratchet(args.table),mode=args.mode,
                     entries=None if args.entries=='-' else int(args.entries),
                     unresolved=None if args.unresolved=='-' else int(args.unresolved),
                     resolved=None if args.resolved=='-' else int(args.resolved))
        Path(args.out).write_text(json.dumps(row,sort_keys=True)+'\n')
        print('GATE_LKE_RATCHET die=%s count=%d max=%s verdict=%s' %
              (row['die'],row['count'],row['max_count'] if row['max_count'] is not None else '-',row['verdict']))
        print('GATE_LKE_COUNTS reports=%d entries=%s unresolved=%s resolved=%s unit=distinct_reports' %
              (row['count'],row['entries'] if row['entries'] is not None else '-',
               row['unresolved_entries'] if row['unresolved_entries'] is not None else '-',
               row['resolved_count'] if row['resolved_count'] is not None else '-'))
        return 1 if row['verdict'] == 'FAIL' else 0
    except (OSError,ValueError,KeyError) as error:
        print('GATE_LKE_RATCHET_ERROR '+str(error),file=sys.stderr); return 2


if __name__ == '__main__': sys.exit(main())
