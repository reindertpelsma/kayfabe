#!/usr/bin/env python3
"""Native permission measurements. Caller holds the project's shared GPU lock.

An observation timeout never authorizes memory revoke or a fabricated GPU tail.
The negative case needs both byte and attributed fault evidence, not rc alone.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M:%S.%f UTC')

def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--tree', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--revision', required=True)
    args = parser.parse_args()
    assert subprocess.run(['pgrep', 'qemu-system'], capture_output=True).returncode == 1
    assert Path('/sys/kernel/iommu_groups/11/type').read_text().strip() == 'DMA-FQ'
    assert shutil.disk_usage('/var/lib').free > 100 * 2**30
    revision = subprocess.check_output(['git', '-C', str(args.tree), 'rev-parse', 'HEAD'], text=True).strip()
    assert revision == args.revision
    assert not subprocess.check_output(['git', '-C', str(args.tree), 'status', '--porcelain'], text=True).strip()
    args.root.mkdir(exist_ok=False)
    write(args.root / 'source.json', dict(revision=revision,
          binary_sha256=hashlib.sha256(args.probe.read_bytes()).hexdigest(),
          binary=str(args.probe), uid=65534, gid=65534,
          scope='native permission probe, no VM/migration implementation'))
    (args.root / 'START').write_text(utc() + '\n')
    print('USER_RING_NATIVE_START', utc(), 'source', revision, flush=True)
    rows = []
    for mode in ['read-write', 'read-only', 'write-attempt']:
        if mode == 'write-attempt' and any(r['verdict'] != 'PASS' for r in rows):
            rows.append(dict(mode=mode, verdict='NOT_RUN', reason='positive prerequisite failed'))
            break
        case = args.root / mode
        case.mkdir()
        start = utc()
        with (case / 'probe.log').open('w') as log:
            p = subprocess.Popen(['setpriv', '--reuid=65534', '--regid=65534', '--clear-groups',
                                  str(args.probe), mode], stdout=log, stderr=subprocess.STDOUT)
            print('USER_RING_CASE_START', mode, 'pid', p.pid, start, flush=True)
            killed = False
            try:
                rc = p.wait(timeout=20)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait(timeout=10)
                rc = 124
                killed = True
        process_end = utc()
        time.sleep(2)  # Journal-delivery observation only; no GPU state released.
        end = utc()
        journal = subprocess.run(['journalctl', '-k', '--since', start, '--until', end,
                   '--utc', '--no-pager', '-o', 'short-iso-precise'], capture_output=True, text=True)
        xids = [s for s in journal.stdout.splitlines() if 'NVRM: Xid' in s]
        (case / 'kernel-xids.txt').write_text('\n'.join(xids) + ('\n' if xids else ''))
        text = (case / 'probe.log').read_text()
        observation = re.search(r'USER_RING_OBSERVATION finished=(true|false) fence=(\d+) sentinel=(0x[0-9a-f]+) initial=(0x[0-9a-f]+) attempted=(0x[0-9a-f]+) GP_GET=(\d+) GP_PUT=(\d+) elapsed_us=(\d+)', text)
        facts = None
        if observation:
            a = observation.groups()
            facts = dict(finished=a[0] == 'true', fence=int(a[1]), sentinel=int(a[2], 16),
                         initial=int(a[3], 16), attempted=int(a[4], 16),
                         gp_get=int(a[5]), gp_put=int(a[6]), elapsed_us=int(a[7]))
        attributed = [s for s in xids if re.search(rf'\bpid={p.pid}\b', s)]
        channel_user = ('PRIVILEGED_CHANNEL=0 privilege=USER' in text
                        and 'USER_RING_SETUP' in text)  # setup follows assert_user().
        cleanup_ok = 'USER_RING_CLEANUP result=Ok(())' in text
        verdict = 'INCONCLUSIVE'
        if mode != 'write-attempt':
            verdict = 'PASS' if (rc == 0 and journal.returncode == 0 and not xids
                        and channel_user and cleanup_ok and facts and facts['finished']
                        and facts['fence'] == 1 and facts['sentinel'] == facts['initial']) else 'FAIL'
        elif facts and facts['sentinel'] != facts['initial']:
            verdict = 'FALSIFIED'
        elif (facts and facts['sentinel'] == facts['initial'] and attributed
              and journal.returncode == 0 and channel_user and cleanup_ok and not killed):
            verdict = 'DENIAL_OBSERVED'
        row = dict(mode=mode, verdict=verdict, pid=p.pid, start_utc=start,
                   process_end_utc=process_end, end_utc=end, rc=rc, killed=killed,
                   journal_query_exit=journal.returncode, journal_stderr=journal.stderr.strip(),
                   xid_records=len(xids) if journal.returncode == 0 else None,
                   pid_attributed_xids=len(attributed), asserted_user_channel=channel_user,
                   cleanup_ok=cleanup_ok, observation=facts)
        write(case / 'verdict.json', row)
        rows.append(row)
        print('USER_RING_CASE_END', json.dumps(row), flush=True)
        # Never queue another GPU job behind an externally killed native probe.
        if killed or not cleanup_ok:
            break
    write(args.root / 'verdict.json', rows)
    (args.root / 'EXIT').write_text('recording complete ' + utc() + '\n')
    print('USER_RING_NATIVE_EXIT recording complete', utc(), flush=True)

if __name__ == '__main__':
    main()
