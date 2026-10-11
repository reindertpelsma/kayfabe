#!/usr/bin/env python3
"""Bench-only lifetime discriminator plus bounded read-only RC journal queries.
Caller owns serial GPU lock. Raw records stay private. Never a product fix.
"""
import datetime
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path('/var/lib/kf-windows-20261005/codex-private-gravity-20261011/native-hold')
PROBE = '/tmp/kf-deferred-update-hold-probe'
QUERY = '/tmp/kf-rc-journal-probe-428ad66d7'
LAYOUT = '/tmp/kf-rc-layout-595.91.07.txt'
PREFIX = ['setpriv', '--reuid=65534', '--regid=65534', '--clear-groups']

def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M:%S.%f UTC')

assert subprocess.run(['pgrep', 'qemu-system'], capture_output=True).returncode == 1
assert Path('/sys/kernel/iommu_groups/11/type').read_text().strip() == 'DMA-FQ'
ROOT.mkdir(mode=0o700, exist_ok=False)
print('NATIVE_HOLD_START', utc(), flush=True)
results = []
for mode in ['nop', 'update', 'update-hold']:
    case = ROOT / mode
    case.mkdir(mode=0o700)
    start = utc()
    with (case / 'probe.log').open('w') as log:
        p = subprocess.Popen(PREFIX + [PROBE, mode], stdout=log, stderr=subprocess.STDOUT)
        try:
            rc = p.wait(timeout=30)
        except subprocess.TimeoutExpired:
            p.kill(); p.wait(timeout=10); rc = 124
    process_end = utc()
    time.sleep(2)  # Observe delayed error delivery only; never releases product state.
    end = utc()
    subprocess.run(['python3', '/tmp/kf-capture-xids.py', str(case), start, end], check=True)
    meta = json.loads((case / 'kernel-xids.json').read_text())
    raw = Path(tempfile.mkdtemp(prefix='kf-rc-hold-', dir='/var/tmp'))
    os.chown(raw, 65534, 65534)
    with (case / 'query.log').open('w') as log:
        # Global diagnostic journal is permission-gated (unprivileged probe refused).
        # Root READ only: no privileged channel, GPU work, or product dependency.
        q = subprocess.run([QUERY, '595.91.07', LAYOUT, str(raw / 'records')],
                           stdout=log, stderr=subprocess.STDOUT, timeout=30)
    os.chown(raw, 0, 0)
    raw.rename(case / 'raw')
    result = dict(mode=mode, pid=p.pid, rc=rc, query_rc=q.returncode,
                  start_utc=start, process_end_utc=process_end, end_utc=end,
                  xid_records=meta['xid_records'],
                  verdict='PASS' if rc == 0 and meta['xid_records'] == 0 else 'FAIL',
                  scope='same-config native lifetime diagnostic, not Windows repair')
    (case / 'verdict.json').write_text(json.dumps(result, indent=2) + '\n')
    results.append(result)
    print('NATIVE_HOLD_CASE', json.dumps(result), flush=True)
(ROOT / 'verdict.json').write_text(json.dumps(results, indent=2) + '\n')
print('NATIVE_HOLD_EXIT', utc(), flush=True)
