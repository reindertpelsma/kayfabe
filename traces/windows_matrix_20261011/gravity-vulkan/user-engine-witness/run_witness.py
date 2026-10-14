#!/usr/bin/env python3
"""Native private-storage and HOST-consumption witness measurements. Caller holds the project's shared GPU lock.

An observation timeout never authorizes memory revoke or a fabricated GPU tail.
The held case needs a zero reference before its explicit signal, then real GPU proof.
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
          scope='native C56F/C997 engine-held unique PB processing witness, no VM/migration implementation'))
    (args.root / 'START').write_text(utc() + '\n')
    print('USER_ENGINE_WITNESS_START', utc(), 'source', revision, flush=True)
    rows = []
    for mode in ['engine-witness-sync']:
        if any(r['verdict'] != 'PASS' for r in rows):
            rows.append(dict(mode=mode, verdict='NOT_RUN', reason='positive prerequisite failed'))
            break
        case = args.root / mode
        case.mkdir()
        start = utc()
        with (case / 'probe.log').open('w') as log:
            p = subprocess.Popen(['setpriv', '--reuid=65534', '--regid=65534', '--clear-groups',
                                  str(args.probe), mode], stdout=log, stderr=subprocess.STDOUT)
            print('USER_ENGINE_WITNESS_CASE_START', mode, 'pid', p.pid, start, flush=True)
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
        observation = re.search(r'USER_ENGINE_WITNESS_OBSERVATION finished=(true|false) reference=(\d+) engine_release=(\d+) host_marker=(\d+) GP_GET=(\d+) elapsed_us=(\d+)', text)
        facts = None
        if observation:
            a=observation.groups()
            facts=dict(finished=a[0]=='true',reference=int(a[1]),engine_release=int(a[2]),host_marker=int(a[3]),gp_get=int(a[4]),elapsed_us=int(a[5]))
        held = re.search(r'USER_ENGINE_WITNESS_HELD samples=(\d+) cursor=Some\((\d+)\) GP_GET=(\d+) early_engine=(true|false) early_reference=(true|false) acquire=(\d+) engine_release=(\d+) host_marker=(\d+)',text)
        held_facts = None
        if held:
            a=held.groups()
            held_facts=dict(samples=int(a[0]),cursor=int(a[1]),gp_get=int(a[2]),early_engine=a[3]=='true',early_reference=a[4]=='true',acquire=int(a[5]),engine_release=int(a[6]),host_marker=int(a[7]))
        setup=re.search(r'witness_va=(0x[0-9a-f]+) witness_bytes=(\d+)',text)
        witness=int(setup[1],16) if setup else None
        initial_zero='USER_ENGINE_WITNESS_INITIAL cursor=Some(0)' in text
        attributed=[s for s in xids if re.search(rf'\bpid={p.pid}\b',s)]
        channel_user=('PRIVILEGED_CHANNEL=0 privilege=USER' in text and 'USER_ENGINE_WITNESS_SETUP' in text and 'USER_ENGINE_WITNESS_IDENTITY uid=65534' in text)
        cleanup_ok='USER_ENGINE_WITNESS_CLEANUP result=Ok(()) released=true' in text
        hold_ok=(held_facts and witness is not None and initial_zero
            and held_facts['samples']>0 and held_facts['gp_get']==2
            and witness<held_facts['cursor']<=witness+int(setup[2])
            and not held_facts['early_engine'] and not held_facts['early_reference']
            and held_facts['acquire']==0 and held_facts['engine_release']==0 and held_facts['host_marker']==1)
        verdict='PASS' if (rc==0 and journal.returncode==0 and not xids and not killed
            and channel_user and cleanup_ok and hold_ok and facts and facts['finished']
            and facts['reference']==1 and facts['engine_release']==1 and facts['host_marker']==1 and facts['gp_get']==2) else 'FAIL'
        row = dict(mode=mode, verdict=verdict, pid=p.pid, start_utc=start,
                   process_end_utc=process_end, end_utc=end, rc=rc, killed=killed,
                   journal_query_exit=journal.returncode, journal_stderr=journal.stderr.strip(),
                   xid_records=len(xids) if journal.returncode == 0 else None,
                   pid_attributed_xids=len(attributed), asserted_user_channel=channel_user,
                   cleanup_ok=cleanup_ok, observation=facts, held_observation=held_facts)
        write(case / 'verdict.json', row)
        rows.append(row)
        print('USER_ENGINE_WITNESS_CASE_END', json.dumps(row), flush=True)
        # Never queue another GPU job behind an externally killed native probe.
        if killed or not cleanup_ok:
            break
    write(args.root / 'verdict.json', rows)
    (args.root / 'EXIT').write_text('recording complete ' + utc() + '\n')
    print('USER_ENGINE_WITNESS_EXIT recording complete', utc(), flush=True)

if __name__ == '__main__':
    main()
