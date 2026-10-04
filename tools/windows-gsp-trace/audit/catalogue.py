#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Recompute the bounded Linux counterexample census and render the call catalogue.

Source interpretations live in reviewed descriptions.json/source-index.json.
This does not infer OS exclusivity from absence, nor equate ioctl and GSP planes.
"""
import argparse
import collections
import gzip
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess

HERE = Path(__file__).resolve().parent
EVIDENCE = HERE.parent / 'evidence/2026-10-04-rtx4070-580.88'
CAT = EVIDENCE / 'command-catalogue'
BASELINE = 'traces/v3_windows/discovery_20261004/linux_baseline/run_lin_b1t_580.65.06_qemu.trimmed.log'
SERIES = {
    'ga102_native_ioctls': ('I102', 'traces/v3_refusal_audit/ga102_vrf/host/*.jsonl.zst'),
    'ga104_native_gfx_ioctls': ('I104', 'traces/v3_gfxset/diag2/nvdiff/*.host_nvdiff.jsonl.zst'),
    'gb203_native_ioctls': ('I203', 'traces/v3_blackwell/t0_nvdiff_ce_bare_gb203.jsonl.zst'),
    'ga106_native_gsp': ('G106', 'traces/real_ga106/rpc_transcript_real_ga106.txt'),
    'ga106_native_ioctl': ('I106', 'traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt'),
    'ga106_linux5806506_to_kayfabe': ('K106', BASELINE),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def census(repo, windows_repo, ids):
    by_id, sources = {}, []
    for series, (_, pattern) in SERIES.items():
        root = windows_repo if series.endswith('to_kayfabe') else repo
        files = sorted(root.glob(pattern))
        if not files:
            raise ValueError(f'missing evidence: {root / pattern}')
        for path in files:
            data = (subprocess.check_output(['zstd', '-dc', str(path)]).decode()
                    if path.suffix == '.zst' else path.read_text())
            total, matched = 0, 0
            for line_num, line in enumerate(data.splitlines(), 1):
                total += 1
                extra = {}
                if path.suffix == '.zst':
                    r = json.loads(line)
                    if r.get('t') != 'ioctl' or r.get('nr') != 42 or r.get('dev') != 'nvidiactl':
                        continue
                    pre, post = bytes.fromhex(r['hpre']), bytes.fromhex(r['hpost'])
                    if len(pre) < 32 or len(post) < 32:
                        raise ValueError(f'short NVOS54 in {path}:{line_num}')
                    cmd, size = struct.unpack_from('<I', pre, 8)[0], struct.unpack_from('<I', pre, 24)[0]
                    status = hex(struct.unpack_from('<I', post, 28)[0])
                    extra = {'rc': r['rc'], 'record_i': r.get('i')}
                elif series == 'ga106_native_gsp':
                    m = re.search(r'KAYFABE-RPC: cmd=(0x[0-9a-f]+) psize=(\d+) gspst=(0x[0-9a-f]+)', line)
                    if not m:
                        continue
                    cmd, size, status = int(m[1], 16), int(m[2]), hex(int(m[3], 16))
                elif series.endswith('to_kayfabe'):
                    m = re.search(r'kf-rm: rpc-trace fn=76 \w+ seq=\d+ cmd=(0x[0-9a-f]+).*? result=(\S+)', line)
                    if not m:
                        continue
                    cmd, size, status = int(m[1], 16), None, m[2]
                else:
                    m = re.search(r'CTRL cmd=(0x[0-9a-f]+).*? size=(\d+) status=(0x[0-9a-f]+)', line)
                    if not m:
                        continue
                    cmd, size, status = int(m[1], 16), int(m[2]), hex(int(m[3], 16))
                key = f'0x{cmd:08x}'
                if key not in ids:
                    continue
                matched += 1
                item = by_id.setdefault(key, {}).setdefault(series, {'count': 0, 'statuses': {}, 'examples': []})
                item['count'] += 1
                item['statuses'][status] = item['statuses'].get(status, 0) + 1
                if len(item['examples']) < 3:
                    item['examples'].append({'file': str(path.relative_to(root)), 'line': line_num,
                                             'size': size, 'status': status, **extra})
            sources.append({'file': str(path.relative_to(root)), 'series': series,
                            'sha256': digest(path), 'records': total, 'matching_records': matched})
    return {'by_id': by_id, 'sources': sources}


def table(headers, rows):
    def cell(v):
        return str(v).replace('|', '\\|').replace('\n', ' ')
    return '\n'.join(['| ' + ' | '.join(headers) + ' |', '| ' + ' | '.join(['---'] * len(headers)) + ' |'] +
                     ['| ' + ' | '.join(map(cell, row)) + ' |' for row in rows])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--repo', type=Path, required=True)
    ap.add_argument('--windows-repo', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    args = ap.parse_args()
    audit = json.loads((EVIDENCE / 'command-audit/audit.json').read_text())
    index = json.loads((CAT / 'source-index.json').read_text())
    descriptions = json.loads((CAT / 'descriptions.json').read_text())
    controls = audit['controls']
    ids = {r['id'] for r in controls}
    assert len(ids) == len(controls) == 129
    linux = census(args.repo, args.windows_repo, ids)
    # Independently check the allocation body limitation against retained frames.
    alloc, absent = collections.Counter(), collections.Counter()
    with gzip.open(EVIDENCE / 'gsp.jsonl.gz', 'rt') as capture:
        for line in capture:
            rec = json.loads(line)
            if rec.get('kind') != 'record' or rec.get('rpc_function') != 103:
                continue
            raw = bytes.fromhex(rec['payload_hex'])
            direction = {0: 'request', 1: 'reply'}[rec['direction']]
            alloc[(direction, len(raw))] += 1
            assert len(raw) == 112
            declared = struct.unpack_from('<I', raw, 100)[0]
            if declared:
                absent[direction] += 1
    # gzip text mode normalizes CRLF; hash the original bytes for evidence identity.
    with gzip.open(EVIDENCE / 'gsp.jsonl.gz', 'rb') as capture:
        assert hashlib.sha256(capture.read()).hexdigest() == audit['text_sha256']
    assert sum(alloc.values()) == 472, alloc
    assert absent == {'request': 179, 'reply': 179}, absent
    rows, rendered = [], []
    for original in controls:
        key = original['id']
        row = dict(original)
        source = index['selected'].get(key)
        matches = linux['by_id'].get(key, {})
        description = descriptions.get(key)
        if description is None:
            permit = original['surface']['capability_permit']
            family = 'BinAPI' if key.startswith('0x2081') else ('GSS-legacy' if 'Gss' in permit else 'unresolved')
            description = (f'Opaque {family} control. No semantic definition recovered in the searched sources; '
                           'inputs, outputs, units and state effects remain unresolved. The observed length/status '
                           'and Linux occurrences do not justify inventing a response or treating it as a no-op.')
        row.update(description=description, preferred_source=source, linux=matches,
                   windows_only='not established')
        rows.append(row)
        name = f"[{source['symbol']}]({source['url']})" if source else 'Unresolved'
        sizes = ','.join(original['request_sizes'])
        statuses = ', '.join(f'{s}×{n}' for s, n in original['reply_statuses'].items())
        observed = f"{original['requests']}/{original['replies']}; {sizes} B; {statuses}"
        if original['incomplete_parameter_heads']:
            observed += '; fragmented body'
        found = ', '.join(f'{SERIES[k][0]}×{v["count"]}' for k, v in sorted(matches.items())) or 'Not seen in these Linux samples'
        layout = source.get('layout') if source else None
        schema = f"{layout['sizeof']} B (`{layout['type'] or 'no parameters'}`)" if layout else 'Unresolved'
        if key == '0x00730122':
            schema = '**Historical 16 B ≠ observed 8 B**'
        if key == '0x00730282':
            schema = '**Newer 2608 B ≠ observed 2600 B**'
        rendered.append([f'`{key}`<br>{name}', description, observed, schema, found])
    result = {'schema': 1, 'audit_text_sha256': audit['text_sha256'], 'windows_only_confirmed': [],
              'linux_observed_ids': len(linux['by_id']), 'controls': rows,
              'allocation_frame_bytes': 112, 'allocation_frames': 472,
              'allocation_nonempty_declared_but_absent': dict(absent)}
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'linux-evidence.json').write_text(json.dumps(linux, indent=2, sort_keys=True) + '\n')
    (args.output / 'catalogue.json').write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    (args.output / 'controls.md').write_text('# Captured control catalogue\n\n**STATUS: RESEARCH, 2026-10-04.** Generated by `audit/catalogue.py`.\n\n'
        'Read [methodology, source versions and Linux-series legend](README.md) before interpreting this table. '
        'All 129 direct IDs are included once. **No row is established as Windows-only.** '
        'Q/R counts are independent; lengths are declared parameter lengths. Reply statuses are inner RM status, '
        'not outer RPC status. Source structure sizes come from the selected public version, not inference from the capture.\n\n'
        + table(['ID / preferred public definition', 'Meaning and remaining limits', 'Windows Q/R; bytes; reply status counts',
                 'Public layout', 'Linux occurrence (see legend)'], rendered) + '\n')
    print(json.dumps({'controls': len(rows), 'linux_ids': len(linux['by_id']),
                      'native_linux_ids': sum(any(not s.endswith('to_kayfabe') for s in v) for v in linux['by_id'].values()),
                      'native_gsp_ids': sum('ga106_native_gsp' in v for v in linux['by_id'].values()),
                      'linux_guest_gsp_ids': sum('ga106_linux5806506_to_kayfabe' in v for v in linux['by_id'].values()),
                      'allocation_nonempty_absent': dict(absent)}, indent=2))


if __name__ == '__main__':
    main()
