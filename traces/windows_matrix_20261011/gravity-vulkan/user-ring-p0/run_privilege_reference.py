#!/usr/bin/env python3
"""Bench-only UID discriminator; never authorizes privileged product calls.
Caller holds /tmp/kayfabe-fastguest.lock. Exact existing native probe, no VM.
"""
import datetime
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path('/var/lib/kf-windows-20261005/codex-ring-priv-reference-b081f705')
PROBE = Path('/data/kf-fifo-refusal-target-b081f705/release/kf-deferred-update-probe')
TREE = Path('/data/kf-fifo-refusal-host-b081f705')
REV = 'b081f705a7502380d584f37ab7da80e668f5018f'
SHA = '9acb5976725bbc0fd8a57685038840f121cca4b7950097b7f8452544d61c224c'

def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M:%S.%f UTC')

def write(p, value):
    p.write_text(json.dumps(value, indent=2) + '\n')

assert subprocess.run(['pgrep', 'qemu-system'], capture_output=True).returncode == 1
assert Path('/sys/kernel/iommu_groups/11/type').read_text().strip() == 'DMA-FQ'
assert shutil.disk_usage('/var/lib').free > 100 * 2**30
assert subprocess.check_output(['git', '-C', str(TREE), 'rev-parse', 'HEAD'], text=True).strip() == REV
assert hashlib.sha256(PROBE.read_bytes()).hexdigest() == SHA
ROOT.mkdir(exist_ok=False)
write(ROOT / 'source.json', dict(revision=REV, binary_sha256=SHA, binary=str(PROBE),
      workload='update-hold', variable='registrant process UID/GID65534 versus0',
      purpose='privileged reference measurement only; GPU channels must remain USER'))
print('PRIV_REFERENCE_START', utc(), flush=True)
rows = []
for uid in [65534, 0]:
    case = ROOT / ('uid' + str(uid))
    case.mkdir()
    start = utc()
    killed = False
    with (case / 'probe.log').open('w') as log:
        p = subprocess.Popen(['setpriv', f'--reuid={uid}', f'--regid={uid}', '--clear-groups',
                              str(PROBE), 'update-hold'], stdout=log, stderr=subprocess.STDOUT)
        print('PRIV_REFERENCE_CASE_START', uid, 'pid', p.pid, start, flush=True)
        try:
            rc = p.wait(timeout=30)
        except subprocess.TimeoutExpired:
            p.kill(); p.wait(timeout=10); rc = 124; killed = True
    process_end = utc()
    time.sleep(2)  # Journal delivery only, never a product completion/release.
    end = utc()
    j = subprocess.run(['journalctl', '-k', '--since', start, '--until', end,
         '--utc', '--no-pager', '-o', 'short-iso-precise'], capture_output=True, text=True)
    xids = [s for s in j.stdout.splitlines() if 'NVRM: Xid' in s]
    (case / 'kernel-xids.txt').write_text('\n'.join(xids) + ('\n' if xids else ''))
    text = (case / 'probe.log').read_text()
    user = 'PRIVILEGED_CHANNEL=0 privilege=USER' in text
    tail = 'DEFERRED_UPDATE GPU_TAIL fence=1' in text
    hold = 'DEFERRED_UPDATE_HOLD_BEGIN' in text and 'DEFERRED_UPDATE_HOLD_END' in text
    cleanup = 'DEFERRED_UPDATE_CLEANUP channel=Ok(Some(())) view=Ok(()) unmap=Ok(()) memory=Ok(())' in text
    attributed = [s for s in xids if re.search(rf'\bpid={p.pid}\b', s)]
    row = dict(uid=uid, gid=uid, pid=p.pid, start_utc=start, process_end_utc=process_end,
               end_utc=end, rc=rc, killed=killed, asserted_user_channel=user,
               gpu_tail=tail, hold_complete=hold, cleanup_ok=cleanup,
               journal_query_exit=j.returncode, journal_stderr=j.stderr.strip(),
               xid_records=len(xids) if j.returncode == 0 else None,
               pid_attributed_xids=len(attributed),
               verdict='PASS' if rc == 0 and user and tail and hold and cleanup and j.returncode == 0 and not xids else 'FAIL')
    write(case / 'verdict.json', row)
    rows.append(row)
    print('PRIV_REFERENCE_CASE_END', json.dumps(row), flush=True)
    if killed or not cleanup: break
write(ROOT / 'verdict.json', rows)
print('PRIV_REFERENCE_EXIT recording complete', utc(), flush=True)
