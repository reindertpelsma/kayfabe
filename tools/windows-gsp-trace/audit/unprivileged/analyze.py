#!/usr/bin/env python3
"""Positive-only correlation of ordinary-user ioctls with actual native GSP sends."""
import argparse
import collections
import gzip
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import sys

REPO = Path(__file__).resolve().parents[4]


def read(path):
    return gzip.open(path, 'rt').read() if path.suffix == '.gz' else path.read_text()


def fields(line):
    return {k: int(v, 16 if k.startswith('cap') else 0)
            for k, v in re.findall(r'(\w+)=(-?\w+)', line)}


def source_exports(root, ids):
    out = {}
    revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
    for path in sorted((root / 'src/nvidia/generated').glob('g_*_nvoc.c')):
        s = path.read_text()
        for m in re.finditer(r'\{\s*/\*\s*\[\d+\]\s*\*/(.*?)\n\s*\}', s, re.S):
            values = {}
            for field in ('methodId', 'flags', 'accessRight'):
                v = re.search(r'/\*\s*' + field + r'\s*=\s*\*/\s*(0x[0-9a-fA-F]+)', m[1])
                if v:
                    values[field] = int(v[1], 16)
            if 'methodId' not in values or values['methodId'] not in ids:
                continue
            flags = values['flags']
            permission = ('internal' if flags & 0x80 else 'admin' if flags & 4 else
                          'user' if flags & 8 else 'kernel')
            if flags & 0x20:
                permission += '; admin when access rights disabled'
            item = dict(flags=hex(flags), rights=hex(values['accessRight']), permission=permission,
                        file=str(path.relative_to(root)), line=s.count('\n', 0, m.start()) + 1)
            out.setdefault(f'0x{values["methodId"]:08x}', []).append(item)
    return dict(revision=revision, exports=out)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('evidence', type=Path)
    ap.add_argument('--ogkm575', type=Path, required=True)
    ap.add_argument('--ogkm580', type=Path, required=True)
    args = ap.parse_args()
    p = args.evidence
    catalogue = json.loads((REPO / 'tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/command-catalogue/catalogue.json').read_text())
    ids = {int(r['id'], 16) for r in catalogue['controls']}
    assert len(ids) == 129
    sources = {'575.51.03': source_exports(args.ogkm575, ids | {0x2080121f, 0x2080012d}),
               '580.65.06': source_exports(args.ogkm580, ids | {0x2080121f, 0x2080012d})}
    postures = {}
    for path in sorted(p.glob('*.posture')):
        s = path.read_text()
        assert 'POSTURE_PASS exec' in s and 'POSTURE_FAIL' not in s
        f = fields(s.splitlines()[0])
        assert all(f[k] == 65534 for k in ('uid', 'euid', 'suid', 'gid', 'egid', 'sgid'))
        assert len(re.findall(r'^Cap\w+:\s+0+$', s, re.M)) == 5
        assert re.search(r'^NoNewPrivs:\s+1$', s, re.M)
        postures[path.stem] = f['pid']
    assert len(postures) == 7
    raw = [json.loads(l) for l in read(p / 'native-gsp.jsonl.gz').splitlines()]
    manifest = raw.pop(0)
    blob = bytearray()
    for r in raw:
        assert r['kind'] == 'chunk' and r['offset'] == len(blob)
        blob.extend(bytes.fromhex(r['hex']))
    assert len(blob) == manifest['source_bytes']
    assert hashlib.sha256(blob).hexdigest() == manifest['source_sha256']
    sys.path.insert(0, str(REPO / 'scripts/rpctrace'))
    import decode_rpctrace as decoder
    header, records = decoder.verify_and_parse(blob, expect_version='575.51.03')
    assert header == manifest['recorder_header']
    assert not read(p / 'open-ioctl-events.err').strip()
    events = read(p / 'open-ioctl-events.log')
    assert not re.search(r'lost|error', events, re.I)
    entered, intervals = {}, collections.defaultdict(list)
    for n, line in enumerate(events.splitlines(), 1):
        if not line.startswith('IOCTL_'):
            continue
        f = fields(line)
        assert f['uid'] == 65534
        key = f['tid'], f['seq']
        f['line'] = n
        if line.startswith('IOCTL_ENTER'):
            assert key not in entered
            entered[key] = f
        else:
            enter = entered.pop(key)
            assert enter['pid'] == f['pid'] and enter['ns'] <= f['ns']
            intervals[f['tid']].append((enter, f))
    assert not entered
    stamps, sends = {}, {}
    for n, line in enumerate(read(p / 'instrumented-kernel.log').splitlines(), 1):
        if 'KFU_' not in line:
            continue
        f = fields(line)
        f['line'] = n
        dest = stamps if 'KFU_RPC' in line else sends
        assert f['seq'] not in dest
        dest[f['seq']] = f
    proven, outside_ioctl = [], []
    for seq, stamp in sorted(stamps.items()):
        r = records[seq]
        assert (seq, stamp['ns'], stamp['dir'], stamp['fn']) == (r['seq'], r['ts_ns'], r['dir'], r['rpc_fn'])
        assert stamp['uid'] == stamp['euid'] == 65534
        assert stamp['capeff'] == stamp['capprm'] == 0 and stamp['nnp'] == 1
        if r['dir'] != 0:
            continue
        if seq not in sends or sends[seq]['status'] != 0 or r['flags'] & decoder.F_NOT_SENT:
            continue
        send = sends[seq]
        assert (send['pid'], send['tid']) == (stamp['pid'], stamp['tid'])
        assert r['outcome'] == 0
        workload = [k for k, pid in postures.items() if k.startswith('open-') and pid == stamp['pid']]
        assert len(workload) == 1
        matches = [(a, z) for a, z in intervals[stamp['tid']] if a['ns'] <= stamp['ns'] <= z['ns']]
        if not matches:
            outside_ioctl.append(seq)
            continue
        assert len(matches) == 1
        a, z = matches[0]
        assert a['pid'] == stamp['pid']
        outer_control = a['cmd'] if a['req'] & 65535 == 0x462a else None
        cmd = struct.unpack_from('<I', r['body'], 88)[0] if r['rpc_fn'] == 76 else None
        proven.append(dict(record_seq=seq, record_offset=r['off'], ns=r['ts_ns'],
                           pid=stamp['pid'], tid=stamp['tid'], workload=workload[0],
                           credential_line=stamp['line'], send_line=send['line'],
                           ioctl_enter=a, ioctl_exit=z, rpc_function=r['rpc_fn'],
                           control=None if cmd is None else f'0x{cmd:08x}',
                           relation='direct control' if cmd is not None and cmd == outer_control else 'indirect'))
    ioctls = []
    for path in sorted(p.glob('*-ioctl.jsonl.gz')):
        workload = path.name.removesuffix('-ioctl.jsonl.gz')
        assert workload in postures
        # The shim appends. Restrict an open run to TIDs observed in this run's
        # syscall trace with the guarded TGID; old diagnostic attempts remain
        # in the raw log but are not evidence for this posture. Closed-module
        # tests have no syscall recorder, so count only the guarded main TID.
        tids = ({tid for tid, spans in intervals.items()
                 if any(a['pid'] == postures[workload] for a, _ in spans)}
                if workload.startswith('open-') else {postures[workload]})
        for line, text in enumerate(read(path).splitlines(), 1):
            r = json.loads(text)
            if r.get('tid') not in tids:
                continue
            if r.get('t') != 'ioctl' or r.get('nr') != 42 or r.get('dev') != 'nvidiactl':
                continue
            pre, post = bytes.fromhex(r['hpre']), bytes.fromhex(r['hpost'])
            assert len(pre) == len(post) == 32
            cmd, size = struct.unpack_from('<I', pre, 8)[0], struct.unpack_from('<I', pre, 24)[0]
            status = struct.unpack_from('<I', post, 28)[0]
            ioctls.append(dict(file=path.name, line=line, workload=workload, record_i=r['i'],
                               tid=r['tid'], control=f'0x{cmd:08x}', params_size=size,
                               rc=r['rc'], status=hex(status)))
    rows = []
    for cmd in sorted(ids | {0x2080121f, 0x2080012d}):
        key = f'0x{cmd:08x}'
        xs = [r for r in proven if r['control'] == key]
        ys = [r for r in ioctls if r['control'] == key]
        rows.append(dict(id=key, in_windows_direct_inventory=cmd in ids,
                         source={v: s['exports'].get(key, []) for v, s in sources.items()},
                         gsp_direct=[r['record_seq'] for r in xs if r['relation'] == 'direct control'],
                         gsp_indirect=[r['record_seq'] for r in xs if r['relation'] == 'indirect'],
                         ioctls=ys))
    inventory = [r for r in rows if r['in_windows_direct_inventory']]
    summary = dict(windows_ids=129, native_records=len(records), matched_sends=len(proven),
                   credentialed_sends_outside_ioctl=len(outside_ioctl),
                   direct_ids=sum(bool(r['gsp_direct']) for r in inventory),
                   indirect_ids=sum(bool(r['gsp_indirect']) for r in inventory),
                   union_ids=sum(bool(r['gsp_direct'] or r['gsp_indirect']) for r in inventory),
                   rpc_functions=dict(collections.Counter(str(r['rpc_function']) for r in proven)))
    report = dict(schema=1, summary=summary, native_header=header, manifest=manifest,
                  postures=postures, sources=sources, controls=rows, proven_sends=proven,
                  credentialed_sends_without_ioctl=outside_ioctl)
    (p / 'privileges.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    lines = ['# Ordinary-user reachability of the Windows control inventory', '',
             '**STATUS: RESEARCH, 2026-10-04. Linux reachability; no Windows caller-privilege claim.**', '',
             'See [method and limits](README.md). Full record numbers, ioctl intervals, credentials and source flags: [privileges.json](privileges.json).', '',
             f"Of 129 Windows IDs, **{summary['direct_ids']}** were sent to native GSP by matching ordinary-user control ioctls; **{summary['indirect_ids']}** were sent internally during ordinary-user ioctls (**{summary['union_ids']}** distinct IDs combined). A send does not imply firmware success or safe forwarding.", '',
             'Source column: exported-method privilege gate in OGKM 580.65.06. This alone does not prove live emission. `unresolved` means no matching ordinary export was found; generic GSS/BinAPI routes may apply. Source data for the exact tested 575.51.03 release is also retained.', '',
             '| Control | Source gate | Native GSP direct / indirect sends | Accepted direct ioctl: open / closed 575 |',
             '|---|---|---|---|']
    for r in rows:
        source = '; '.join(sorted({x['permission'] for x in r['source']['580.65.06']})) or 'unresolved ordinary export'
        accepted = [sum(x['workload'].startswith(v + '-') and x['rc'] == 0 and x['status'] == '0x0' for x in r['ioctls']) for v in ('open', 'closed')]
        extra = '' if r['in_windows_direct_inventory'] else ' (additional)'
        lines.append(f"| `{r['id']}`{extra} | {source} | {len(r['gsp_direct'])} / {len(r['gsp_indirect'])} | {accepted[0]} / {accepted[1]} |")
    (p / 'privileges.md').write_text('\n'.join(lines) + '\n')
    print(json.dumps(summary, sort_keys=True))


if __name__ == '__main__':
    main()
