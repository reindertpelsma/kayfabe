#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Serial Steel Nomad captures, per-run health, scores and reproducible archives."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import re
from pathlib import Path
import subprocess
import sys
import time
import zipfile

import host_measure


def health(root, tag):
    response = host_measure.agent(root, (root / 'health.ps1').read_text())
    state = json.loads(response)
    active = json.loads((root / 'active-arm.json').read_text())
    kernel = subprocess.check_output(['journalctl', '-k', '--after-cursor=' + active['journal_cursor_before'], '--no-pager'], text=True)
    state['new_host_xid'] = [line for line in kernel.splitlines() if 'NVRM: Xid' in line]
    (root / (tag + '-health.json')).write_text(json.dumps(state, indent=2))
    if state['tdr_4101'] or state['gpu_153'] or state['app_crashes'] or state['new_host_xid']:
        raise RuntimeError('new or unexplained guest/host failure: stop; see ' + tag + '-health.json')


def score(root, tag):
    host_measure.ui(root, 'shot', 0.1786, 0.845)
    name = 'p1_' + tag + '_score'
    host_measure.agent(root, '& ' + host_measure.GUEST + '\\sn_upload.ps1 -File ' + host_measure.GUEST + '\\shot.png -Name ' + host_measure.quote(name))
    raw = (root.parent / 'kfagent' / 'r' / (name + '.txt')).read_bytes()
    if not raw.startswith(b'\x89PNG\r\n\x1a\n'):
        raise RuntimeError('score upload is not a PNG')
    (root / (tag + '-score.png')).write_bytes(raw)


def archive(root, tag):
    r = root / tag
    source = r / (tag + '.zip')
    with zipfile.ZipFile(source) as z:
        members = {i.filename: {'size': i.file_size, 'crc': i.CRC} for i in z.infolist()}
    # extractall already checked every CRC in the capture. Keep the exact archive;
    # only remove expanded/derived files reproducible from its CSV and these tools.
    previous = json.loads((r / 'archive.json').read_text()) if (r / 'archive.json').exists() else {}
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    if previous and previous['sha256'] != digest:
        raise RuntimeError('retained archive changed')
    evidence = {'sha256': digest, 'members': members, 'pruned': previous.get('pruned', [])}
    for name in (tag + '_dxg.csv', tag + '_ev.pkl'):
        p = r / name
        if p.exists():
            if name.endswith('_dxg.csv') and members.get(name, {}).get('size') != p.stat().st_size:
                raise RuntimeError('expanded CSV differs from archive member')
            evidence['pruned'].append({'name': name, 'size': p.stat().st_size})
            p.unlink()
    (r / 'archive.json').write_text(json.dumps(evidence, indent=2))
    names = [name for name in members if name.startswith(tag) and Path(name).name == name]
    if len(names) != len(members):
        raise RuntimeError('archive has unexpected guest filenames; no guest cleanup')
    paths = [host_measure.GUEST + '\\' + name for name in names + [tag + '.zip']]
    cleanup = '\n'.join('Remove-Item -LiteralPath ' + host_measure.quote(name) + ' -Force -ErrorAction Stop' for name in paths)
    host_measure.agent(root, cleanup + "\n'P1_ARCHIVED_GUEST_FILES_REMOVED'")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--clock-map', type=Path, required=True)
    p.add_argument('--prefix', required=True)
    p.add_argument('--start', type=int, default=1)
    p.add_argument('--runs', type=int, default=3)
    p.add_argument('--first-home', action='store_true')
    p.add_argument('--seconds', type=int, default=105)
    a = p.parse_args()
    r = a.root.resolve(strict=True)
    if r != Path('/data/codex-P1') or not re.fullmatch(r'[A-Za-z0-9_-]+', a.prefix):
        p.error('P1 directory required')
    host_measure.require_lease(r)
    active = json.loads((r / 'active-arm.json').read_text())
    qpid = int((r / 'active-qpid').read_text())
    bpid = int((r / 'active-bpid').read_text())
    broker = Path(f'/proc/{bpid}/exe').resolve(strict=True)
    log = Path(f'/proc/{bpid}/fd/1').resolve(strict=True)
    deadline = datetime(2026, 10, 19, 4, 10, tzinfo=timezone.utc).timestamp()
    health(r, a.prefix + '-series-before')
    for index in range(a.start, a.start + a.runs):
        if time.time() >= deadline:
            raise RuntimeError('no new capture this close to the hardware deadline')
        tag = a.prefix + '-sn-r' + str(index)
        command = [sys.executable, str(host_measure.HERE / 'capture_broker.py'), '--root', str(r),
                   '--broker', str(broker), '--broker-pid', str(bpid), '--qpid', str(qpid),
                   '--clock-map', str(a.clock_map), '--offset', '0', '--legacy', '--broker-log', str(log),
                   '--seconds', str(a.seconds)]
        if a.first_home and index == a.start:
            command += ['--fx', '0.3786', '--fy', '0.5852']
        command.append(tag)
        print('P1_SERIES_RUN_START', tag, active['rev'], active['flags'], flush=True)
        with (r / (tag + '.out')).open('x') as output:
            subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, check=True)
        health(r, tag)
        score(r, tag)
        archive(r, tag)
        print('P1_SERIES_RUN_COMPLETE', tag, flush=True)


if __name__ == '__main__':
    main()
