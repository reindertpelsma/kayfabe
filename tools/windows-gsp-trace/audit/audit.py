#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Inventory a validated capture against the reviewed Windows-branch surface.

This is a static coverage audit, not an emulator replay or a compatibility test.
Pin the reviewed source: a new served_chain needs a new review of surface.rs.
"""
import argparse
import collections
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import decode

REVIEWED_REV = 'c50fad9ac485f53d45d4ea77a21cb7206267c65a'
REVIEWED_TRACE = '9d75e47b8ec5dd837dc64e14847779d62a16f14d92169ede8f9b62681b93de29'
RPC_NAMES = {10: 'FREE', 71: 'CONTINUATION_RECORD', 76: 'GSP_RM_CONTROL',
             103: 'GSP_RM_ALLOC', 4099: 'POST_EVENT'}
DEFINE = re.compile(r'^\s*#define\s+(NV\w*CMD\w*)\s+\(?\s*(0x[0-9a-fA-F]+)[UuLl]*\s*\)?\s*(?:/\*.*)?$')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], text=True).strip()


def names_from_sdk(sdk):
    names = collections.defaultdict(list)
    digest = hashlib.sha256()
    files = sorted((sdk / 'src/common/sdk/nvidia/inc/ctrl').rglob('*.h'))
    if not files:
        raise ValueError('no SDK control headers found')
    for path in files:
        rel = path.relative_to(sdk).as_posix()
        digest.update(rel.encode() + b'\0' + path.read_bytes() + b'\0')
        for line, value in enumerate(path.read_text().splitlines(), 1):
            match = DEFINE.match(value)
            if match:
                names[int(match[2], 16)].append(dict(name=match[1], file=rel, line=line))
    return names, dict(directory=sdk.name, header_count=len(files),
                       headers_sha256=digest.hexdigest(),
                       method='Literal numeric NV*CMD* defines in local SDK ctrl headers; absence is not proof that no public name exists elsewhere.')


def surface(repo, ids, output, target):
    rev = git(repo, 'rev-parse', 'HEAD')
    if rev != REVIEWED_REV or git(repo, 'status', '--porcelain', '--untracked-files=no'):
        raise ValueError('audit requires the clean, reviewed v3-windows revision ' + REVIEWED_REV)
    with tempfile.TemporaryDirectory(prefix='windows-surface-') as td:
        manifest = Path(td) / 'Cargo.toml'
        manifest.write_text('[package]\nname="windows-surface-audit"\nversion="0.0.0"\nedition="2024"\n'
                            '[workspace]\n[[bin]]\nname="windows-surface-audit"\npath=' +
                            json.dumps(str(HERE / 'surface.rs')) + '\n[dependencies]\n' +
                            ''.join(f'{c}={{path={json.dumps(str(repo / "crates" / c))}}}\n'
                                    for c in ('kf-rm', 'kf-abi', 'kf-arch', 'kf-chip')))
        env = dict(os.environ, CARGO_TARGET_DIR=str(target))
        with (output / 'build.stderr').open('w') as errors:
            result = subprocess.run(['cargo', 'run', '--quiet', '--manifest-path', str(manifest)],
                                    input=ids, text=True, stdout=subprocess.PIPE,
                                    stderr=errors, env=env, check=True)
    (output / 'surface.tsv').write_text(result.stdout)
    return {(p[0], int(p[1], 16)): p for line in result.stdout.splitlines()
            if (p := line.split('\t'))}


def counter(values):
    return dict(sorted(collections.Counter(values).items()))


def summary(records):
    return dict(requests=sum(r['direction'] == 'request' for r in records),
                replies=sum(r['direction'] == 'reply' for r in records),
                request_sizes=counter(r['_size'] for r in records if r['direction'] == 'request'),
                reply_sizes=counter(r['_size'] for r in records if r['direction'] == 'reply'),
                reply_statuses=counter(r['_status'] for r in records if r['direction'] == 'reply'),
                incomplete_parameter_heads=counter(r['direction'] for r in records if not r['_complete']))


def control_surface(row):
    if row[6] == 'true':
        category = 'display handler'
    elif row[10] == 'true':
        category = 'channel handler'
    elif row[3] == 'Some(GssLegacy8159)':
        category = 'empirical identity reply'
    elif row[3] == 'Some(CudartInit9001)':
        category = 'capture-derived constant reply'
    elif row[3] != 'None':
        category = 'init-table handler'
    elif row[9] != '[]':
        category = 'conditional authored host-fact query'
    elif row[8] != 'None':
        category = 'object handler'
    else:
        category = 'no specific handler'
    return dict(category=category, wanted_table=row[3], wanted_size=row[4],
                wanted_c_type=row[5], display_claims=row[6] == 'true',
                capability_permit=row[7], object_control_shape=row[8],
                gss_rows=json.loads(row[11]), channel_claims=row[10] == 'true')


def cell(value):
    return str(value).replace('|', '\\|').replace('\n', ' ')


def table(headers, rows):
    return '\n'.join(['| ' + ' | '.join(headers) + ' |',
                      '| ' + ' | '.join('---' for _ in headers) + ' |'] +
                     ['| ' + ' | '.join(cell(v) for v in row) + ' |' for row in rows])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--repo', required=True, type=Path)
    ap.add_argument('--sdk', required=True, type=Path)
    ap.add_argument('--capture', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--target-dir', required=True, type=Path)
    args = ap.parse_args()
    repo, sdk = args.repo.resolve(), args.sdk.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='windows-trace-') as td:
        capture = args.capture
        if capture.suffix == '.gz':
            capture = Path(td) / 'trace.jsonl'
            with gzip.open(args.capture, 'rb') as source, capture.open('wb') as dest:
                remaining = 140 * 1024 * 1024
                while chunk := source.read(min(1024 * 1024, remaining + 1)):
                    remaining -= len(chunk)
                    if remaining < 0:
                        raise ValueError('decompressed text exceeds 140 MiB')
                    dest.write(chunk)
        trace = decode.parse_jsonl(capture)
        text_sha = sha(capture)
    if text_sha != REVIEWED_TRACE:
        raise ValueError('this snapshot audit is reviewed for the first RTX 4070 capture only; review the policy surface and identity before adding another capture')
    records = trace['records']
    controls, classes, nested = (collections.defaultdict(list) for _ in range(3))
    for r in records:
        raw = bytes.fromhex(r['payload_hex'])
        if r['rpc_function'] == 76:
            c = r['control']
            cmd = int(c['command'], 16)
            r.update(_size=c['params_bytes_declared'], _status=c['status'],
                     _complete=c['params_complete'], _params=bytes.fromhex(c['params_hex']),
                     _rmapi_flags=struct.unpack_from('<I', raw, 100)[0])
            controls[cmd].append(r)
            # SDK ctrl5080.h: hApiHandle then cmd, independent of union alignment.
            if cmd == 0x50800101 and len(r['_params']) >= 8:
                nested[struct.unpack_from('<I', r['_params'], 4)[0]].append(r)
        elif r['rpc_function'] == 103:
            length = struct.unpack_from('<I', raw, 56)[0]
            if length < 64:
                raise ValueError('short RM_ALLOC v03_00 header')
            cl, status, size = struct.unpack_from('<3I', raw, 92)
            r.update(_size=size, _status=hex(status), _complete=size <= length - 64)
            classes[cl].append(r)
    ids = ''.join(f'control\t{i:08x}\n' for i in sorted(controls.keys() | nested.keys()))
    ids += ''.join(f'class\t{i:08x}\n' for i in sorted(classes))
    rows = surface(repo, ids, output, args.target_dir.resolve())
    names, sdk_provenance = names_from_sdk(sdk)
    report = dict(schema='kayfabe-windows-command-audit/1',
                  source_revision=REVIEWED_REV, guest_driver='580.88', guest_abi='580.65.06',
                  display_family='ADA', capture_complete=False, replay_test=False,
                  text_sha256=text_sha, source_export=trace['text_export'],
                  tool_sha256=sha(Path(__file__).resolve()), surface_helper_sha256=sha(HERE / 'surface.rs'),
                  sdk=sdk_provenance, records=len(records),
                  rpc_sequences=sorted({r['rpc_sequence'] for r in records}),
                  observed_missing=sum(r['missing_before'] for r in records),
                  unknown_prefixes=[dict(direction=r['direction'], sequence=r['queue_sequence'])
                                    for r in records if r['prefix_unknown']],
                  rpcs=[], controls=[], classes=[], deferred_controls=[])
    for fn in sorted({r['rpc_function'] for r in records}):
        rr = [r for r in records if r['rpc_function'] == fn]
        report['rpcs'].append(dict(id=fn, name=RPC_NAMES.get(fn, 'UNREVIEWED RPC'),
                                  directions=counter(r['direction'] for r in rr),
                                  reply_statuses=counter(r['rpc_status'] for r in rr if r['direction'] == 'reply')))
    for cmd, rr in sorted(controls.items()):
        s = control_surface(rows['control', cmd])
        item = dict(id=f'0x{cmd:08x}', sdk_symbols=names[cmd], **summary(rr), surface=s,
                    request_rmapi_flags=counter(hex(r['_rmapi_flags']) for r in rr if r['direction'] == 'request'))
        if s['gss_rows']:
            requests = [r for r in rr if r['direction'] == 'request']
            item['gss_input_gate'] = counter('matches' if r['_complete'] and not r['_rmapi_flags'] & 2 and
                any(len(r['_params']) == row['size'] and
                    all(r['_params'][off:off+4] == struct.pack('<I', value) for off, value in row['inputs'])
                    for row in s['gss_rows']) else 'does_not_match' for r in requests)
            offsets = sorted({o for row in s['gss_rows'] for o, _ in row['inputs']})
            variants = collections.Counter(tuple(struct.unpack_from('<I', r['_params'], o)[0] for o in offsets)
                                           for r in requests if r['_complete'] and len(r['_params']) >= max(offsets) + 4)
            item['gss_observed_input_variants'] = [dict(count=n, inputs=[[off, value] for off, value in zip(offsets, values)])
                                                  for values, n in sorted(variants.items())]
        report['controls'].append(item)
    for cl, rr in sorted(classes.items()):
        row = rows['class', cl]
        category = ('denied by capability policy' if row[4].startswith('Denied') else
                    'no allocation decoder' if row[6] == 'None' else 'allocation decoder exists')
        report['classes'].append(dict(id=f'0x{cl:08x}', names=row[2].split('|'),
                                      category=category, capability_permit=row[4],
                                      abi_shape=row[5], channel_shape=row[6], **summary(rr)))
    for cmd, rr in sorted(nested.items()):
        report['deferred_controls'].append(dict(id=f'0x{cmd:08x}', sdk_symbols=names[cmd],
                    directions=counter(r['direction'] for r in rr),
                    direct_surface=control_surface(rows['control', cmd]),
                    wrapper_supported=False,
                    note='Observed inside 0x50800101; wrapper status does not prove later execution of the deferred operation.'))
    report['totals'] = dict(control_ids=len(controls), class_ids=len(classes),
        control_categories=counter(i['surface']['category'] for i in report['controls']),
        control_ids_with_sdk_name=sum(bool(i['sdk_symbols']) for i in report['controls']),
        control_ids_without_sdk_name=sum(not i['sdk_symbols'] for i in report['controls']),
        allocation_categories=counter(i['category'] for i in report['classes']),
        native_control_error_ids=sum(any(int(s, 16) for s in i['reply_statuses']) for i in report['controls']),
        native_ok_ids_without_handler=sum(i['surface']['category'] == 'no specific handler' and
                                         '0x0' in i['reply_statuses'] for i in report['controls']),
        gfx_pool_query_records=len(controls.get(decode.QUERY, [])))
    (output / 'audit.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    text = ['# Captured Windows commands versus v3-windows\n',
            '**STATUS: RESEARCH, 2026-10-04. Static inventory, not a successful Kayfabe replay.**\n',
            f'Audited revision: `{REVIEWED_REV}`. Windows 580.88 uses ABI 580.65.06 via the source-derived Windows twin mapping; display row ADA. Names use local OGKM `{sdk.name}`.\n',
            'Generic allowlist/GSS/BinAPI rules are not handlers. A specific handler still has payload, object-state, layout and host-fact gates. No generic passthrough is counted.\n',
            '## Counts\n', table(['Measure', 'Count'], [(k, v) for k, v in report['totals'].items() if not isinstance(v, dict)]),
            '\n' + table(['Control implementation category', 'Distinct IDs'], report['totals']['control_categories'].items()),
            '\n## RPC envelopes\n', table(['Function', 'Name', 'Directions', 'Reply status counts'],
                [(i['id'], i['name'], i['directions'], i['reply_statuses']) for i in report['rpcs']]),
            '\n## All directly observed controls\n',
            'Sizes are declared parameter bytes. Status counts are independent reply observations, not inferred request/reply pairs. An SDK name says nothing about implementation coverage.\n',
            table(['ID', 'SDK name', 'Requests / replies', 'Request sizes', 'Native reply statuses', 'Kayfabe path'],
                [(i['id'], ', '.join(n['name'] for n in i['sdk_symbols']) or 'unresolved in this SDK',
                  f"{i['requests']} / {i['replies']}", i['request_sizes'], i['reply_statuses'], i['surface']['category'])
                 for i in report['controls']]),
            '\n## Conditional host-fact query input gates\n',
            'These results check size and every authored input word, plus serialization flags. A match still needs an available host answer.\n',
            table(['ID', 'Observed request gate results'],
                  [(i['id'], i['gss_input_gate']) for i in report['controls'] if 'gss_input_gate' in i]),
            '\n## Allocation classes\n',
            'A decoder is not proof of working class semantics. Native allocation status is the inner RM_ALLOC status.\n',
            table(['Class', 'Names', 'Requests / replies', 'Native statuses', 'Kayfabe'],
                  [(i['id'], ', '.join(n.removeprefix('class_ids:') for n in i['names']),
                    f"{i['requests']} / {i['replies']}", i['reply_statuses'], i['category']) for i in report['classes']]),
            '\n## Deferred wrapper contents\n',
            'NV5080_CTRL_CMD_DEFERRED_API contains another control ID. The wrapper and allocation class are unsupported; having a direct handler does not implement deferred execution.\n',
            table(['Nested ID', 'Name', 'Directions', 'Direct handler only'],
                  [(i['id'], ', '.join(n['name'] for n in i['sdk_symbols']), i['directions'],
                    i['direct_surface']['category']) for i in report['deferred_controls']]),
            '\n## Limits\n',
            '- This is one RTX 4070 / Windows 580.88 reference capture. Hardware VFIO success is not Kayfabe success.',
            f"- Unknown prefixes and {report['observed_missing']} later observed missing record remain. No GR_GFX_POOL_QUERY_SIZE was retained; its output values are still unknown.",
            '- All captured RPC sequence fields are zero; independent request/reply totals do not establish individual pairings.',
            '- Continuation records are validated but not reassembled here. Fragmented command IDs and declared sizes remain identifiable; complete body semantics do not.',
            '- Names absent from this SDK may exist in other sources. Nested commands in opaque protocols are not inferred.',
            '- Display/channel/object links are assumed seated. Handler inventories do not prove Windows submission, paging, interrupts or TDR behavior.',
            '- The empirical identity and capture-derived constant replies are counted separately: neither establishes full semantics or cross-GPU correctness.',
            '- Existing FB_GET_INFO_V2 and BUS_GET_INFO_V2 handlers already rejected Windows info indices 1 and 24 in the original Kayfabe run. Matching ID/size is insufficient.',
            '\nSee `audit.json` for every gate, header source location, provenance hash, and fragmented-head count.\n']
    (output / 'audit.md').write_text('\n'.join(text))
    print(json.dumps(report['totals'], indent=2))


if __name__ == '__main__':
    main()
