#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Name display-capability words using compiler-derived class TSV, offline only."""
import argparse
import json
from pathlib import Path
import re
import struct
import sys

# Reuse bounded reads, JSON output escaping and evidence errors, not product APIs.
import compare

DEFAULT_TABLE = Path(__file__).resolve().parents[2] / 'crates/kf-disp/data/classes-580.65.06.tsv'


def catalogue(path, class_id):
    raw = compare.read_bounded(Path(path), 4 * 1024 * 1024)
    prefix = 'NV' + class_id.upper().removeprefix('0X') + '_'
    compare.require(re.fullmatch(r'NV[0-9A-F]{4}_', prefix) is not None, 'class must be four hexadecimal digits')
    values, fields, arrays, field_arrays = {}, {}, {}, {}
    version = None
    seen = set()
    for number, line in enumerate(raw.decode('utf-8').splitlines(), 1):
        if line.startswith('#') or not line:
            continue
        parts = line.split('\t')
        if parts[0] == 'VERSION':
            compare.require(len(parts) == 2 and version is None, 'bad/duplicate VERSION')
            version = parts[1]
            continue
        compare.require(len(parts) >= 3, 'invalid table row')
        kind, name = parts[:2]
        if not name.startswith(prefix):
            continue
        compare.require(re.fullmatch(r'[A-Z0-9_]+', name) is not None, 'invalid macro name')
        compare.require(name not in seen, f'duplicate macro {name}')
        seen.add(name)
        try:
            nums = tuple(int(x) for x in parts[2:])
        except ValueError as error:
            raise compare.InvalidEvidence('invalid compiler-table integer') from error
        compare.require(all(0 <= n <= 0xffffffffffffffff for n in nums), 'compiler value outside uint64')
        counts = {'V': 1, 'F': 2, 'A': 2, 'FA': 3, 'A2': 3}
        compare.require(kind in counts and len(nums) == counts[kind], 'unsupported compiler row')
        if kind == 'V':
            values[name] = nums[0]
        elif kind == 'F':
            fields[name] = (nums[0], nums[1], number)
        elif kind == 'A':
            arrays[name] = (nums[0], nums[1], number)
        elif kind == 'FA':
            field_arrays[name] = (*nums, number)
        # Two-dimensional methods are not capability-page registers.
    compare.require(version is not None and fields, 'class has no compiler-derived fields')
    # A register is an offset macro with subordinate field definitions, not an enum/default.
    candidates = set(values) | set(arrays)
    register_fields = {}
    for name, (high, low, line) in fields.items():
        parents = [p for p in candidates if name.startswith(p + '_')]
        if not parents:
            continue
        parent = max(parents, key=len)
        compare.require(0 <= low <= high < 32, 'capability field exceeds one word')
        register_fields.setdefault(parent, []).append(dict(name=name, high=high, low=low, table_line=line))
    # Some classes expose indexed field names alongside or instead of expanded aliases.
    for name, (high, low, stride, line) in field_arrays.items():
        parents = [p for p in candidates if name.startswith(p + '_')]
        if not parents:
            continue
        parent = max(parents, key=len)
        count = values.get(name + '__SIZE_1')
        compare.require(count is not None and 1 <= count <= 32, 'missing/oversized indexed field count')
        for index in range(count):
            hi, lo = high + index * stride, low + index * stride
            compare.require(0 <= lo <= hi < 32, 'indexed capability field exceeds one word')
            register_fields.setdefault(parent, []).append(dict(name=f'{name}({index})', high=hi, low=lo, table_line=line))
    registers = {}
    for name, members in register_fields.items():
        if name in arrays:
            base, stride, _ = arrays[name]
            count = values.get(name + '__SIZE_1')
            compare.require(count is not None and 1 <= count <= 1024 and stride >= 4, 'missing/invalid register array bounds')
        else:
            base, stride, count = values[name], 0, 1
        compare.require(base % 4 == stride % 4 == 0 and base + (count - 1) * stride + 4 <= 4096,
                        'selected class is not a bounded capability-page layout')
        for member in members:
            member['enum_values'] = {n: v for n, v in values.items() if n.startswith(member['name'] + '_')}
        for index in range(count):
            offset = base + index * stride
            registers.setdefault(offset, []).append(dict(name=name, index=index if name in arrays else None,
                                                        fields=sorted(members, key=lambda f: (f['low'], f['name']))))
    compare.require(registers, 'no bounded capability registers for selected class')
    return dict(version=version, class_id=prefix[2:-1], table_sha256=compare.digest(raw),
                registers=registers, table_name=Path(path).name)


def annotate(table, pages):
    compare.require(1 <= len(pages) <= 32, 'expected 1..32 pages')
    samples, names = [], set()
    for name, path in pages:
        compare.require(re.fullmatch(r'[A-Za-z0-9_.-]{1,64}', name) and name not in names, 'invalid/duplicate page label')
        names.add(name)
        raw = compare.read_bounded(Path(path), 4096)
        compare.require(len(raw) == 4096, 'capability page must be exactly 4096 bytes')
        samples.append(dict(id=name, path=str(path), sha256=compare.digest(raw), words=struct.unpack('<1024I', raw)))
    words = []
    for index in range(1024):
        offset = index * 4
        vals = {p['id']: p['words'][index] for p in samples}
        decoded, mask = [], 0
        for register in table['registers'].get(offset, []):
            fields = []
            for field in register['fields']:
                bitmask = ((1 << (field['high'] - field['low'] + 1)) - 1) << field['low']
                mask |= bitmask
                values = {name: (value & bitmask) >> field['low'] for name, value in vals.items()}
                labels = {name: [key for key, encoded in field['enum_values'].items() if encoded == value]
                          for name, value in values.items()}
                fields.append(dict(name=field['name'], high=field['high'], low=field['low'],
                                   table_line=field['table_line'], values=values, value_names=labels,
                                   different=len(set(values.values())) > 1))
            decoded.append(dict(name=register['name'], index=register['index'], fields=fields))
        words.append(dict(offset=f'0x{offset:04x}', values={k: f'0x{v:08x}' for k, v in vals.items()},
                          different=len(set(vals.values())) > 1, registers=decoded,
                          uncovered_mask=f'0x{(~mask & 0xffffffff):08x}',
                          uncovered_values={k: f'0x{(v & ~mask):08x}' for k, v in vals.items()}))
    return dict(schema='kayfabe-display-caps-annotations/1', source={k: v for k, v in table.items() if k != 'registers'},
                pages=[{k: v for k, v in p.items() if k != 'words'} for p in samples], words=words,
                note='Compiler-derived names/masks only. Unknown bits remain raw. INIT is a header default, not a required/valid runtime value. No inference that a difference is wrong or variable data is noise.')


def markdown(report):
    out = ['# Display capability annotations', '', '**STATUS: RESEARCH, 2026-10-05.** Offline source-named observations.', '',
           f"Class `{report['source']['class_id']}`, source version `{report['source']['version']}`; table SHA256 `{report['source']['table_sha256']}`.", '',
           report['note'], '', '| Byte offset | Source register/field | Raw field values by run |', '|---|---|---|']
    changed = [w for w in report['words'] if w['different']]
    for word in changed[:128]:
        named = False
        for reg in word['registers']:
            for field in reg['fields']:
                if field['different']:
                    index = '' if reg['index'] is None else f"[{reg['index']}]"
                    out.append(f"| {word['offset']} | {compare.md(field['name'] + index)} | {compare.md(compare.canonical(field['values']))} |")
                    named = True
        if not named or len(set(word['uncovered_values'].values())) > 1:
            out.append(f"| {word['offset']} | uncovered bits (mask {word['uncovered_mask']}) | {compare.md(compare.canonical(word['uncovered_values']))} |")
    out += ['', f'{len(changed)} differing words. First 128 shown; JSON retains every offset, named field and raw per-run value.', '']
    return '\n'.join(out)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--class', dest='class_id', required=True)
    parser.add_argument('--table', type=Path, default=DEFAULT_TABLE)
    parser.add_argument('--page', action='append', required=True, metavar='LABEL=PATH')
    parser.add_argument('--json', type=Path, dest='json_path', required=True)
    parser.add_argument('--markdown', type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        pages = []
        for item in args.page:
            compare.require('=' in item, '--page requires LABEL=PATH')
            name, path = item.split('=', 1)
            pages.append((name, Path(path)))
        report = annotate(catalogue(args.table, args.class_id), pages)
        inputs = {args.table.resolve(), *(p.resolve() for _, p in pages)}
        outputs = (args.json_path, args.markdown)
        compare.require(len({p.resolve() for p in outputs}) == 2 and
                        all(p.resolve() not in inputs and not p.exists() and not p.is_symlink() for p in outputs),
                        'outputs must be new paths distinct from inputs')
        with args.json_path.open('x', encoding='utf-8') as stream:
            json.dump(report, stream, indent=2, sort_keys=True)
            stream.write('\n')
        with args.markdown.open('x', encoding='utf-8') as stream:
            stream.write(markdown(report))
    except (compare.InvalidEvidence, OSError, UnicodeError) as error:
        parser.exit(2, f'error: {error}\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
