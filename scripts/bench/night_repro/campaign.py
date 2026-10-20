#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Hold the shared lock, consume a reviewable arm queue, then restore the owner.

Collector errors preserve raw payloads and invalidate the run; repair is authorized.
Host safety failures stop the campaign and restore the owner. A missing next arm
leaves the lease held while the operator reviews results. A finish marker or
hard deadline ends the campaign. Restoration hands the lease to the standard
owner launcher; no QEMU is force-killed.
"""
import datetime as dt
import json
import importlib
import os
from pathlib import Path
import re
import subprocess as sp
import sys
import time

import run


def restore():
    # Any still-live N1 QEMU must first pass the same clean shutdown.
    for f in sorted(run.ROOT.glob('*-*/run.json')):
        number=json.loads(f.read_text())['run']
        state=run.W/f'windows-broker-run{number}.state'
        if not state.exists():
            continue
        pid=int(re.search(r'^QPID=(\d+)$',state.read_text(),re.M).group(1))
        if run.alive(pid):
            run.stop_session(pid, f.parent/'launch.log')
    for f in run.ROOT.glob('*-*/collectors.json'):
        for pid in json.loads(f.read_text()):
            if run.alive(pid):
                # Only a recorded collector started by our runner, never QEMU.
                cmd=Path(f'/proc/{pid}/cmdline').read_bytes()
                if b'nvidia-smi' in cmd or b'userd_probe' in cmd:
                    os.kill(pid, 15)
    n=run.next_run()
    out=run.ROOT/f'owner-morning-{n}.log'
    run.STOP.unlink(missing_ok=True)
    env=os.environ.copy()
    # Match the morning instruction literally; the original launcher clears KF3_*.
    env.update(REV='0d7ba743', N=str(n),
               WIN_FLAGS='KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0')
    env.pop('N1_QBIN', None)
    p=sp.Popen(['flock','-w','90','-o',os.environ['N1_LOCK'],'bash',str(run.LAUNCHER)],
          env=env, stdin=sp.DEVNULL, stdout=out.open('w'), stderr=sp.STDOUT, start_new_session=True)
    receipt=dict(run=n, log=str(out), flock_pid=p.pid, rev='0d7ba743', flags=env['WIN_FLAGS'],
                 status='owner launcher queued for lock handoff')
    (run.ROOT/'restoration.json').write_text(json.dumps(receipt,indent=2))
    run.log('RESTORATION_HANDOFF '+json.dumps(receipt))


def main():
    run.verify_lock()
    finished=set()
    history=run.ROOT/'finished-v2.json'
    if history.exists(): finished=set(json.loads(history.read_text()))
    deadline=dt.datetime.fromisoformat(os.environ['N1_DEADLINE']).timestamp()
    try:
        while time.time()<deadline and not (run.ROOT/'finish').exists():
            queue=json.loads((run.ROOT/'queue.json').read_text())
            pending=[j for j in queue if j['id'] not in finished]
            if not pending:
                time.sleep(1)
                continue
            job=pending[0]
            sys.argv=[str(run.SCRIPTS/'run.py')]+job['args']
            importlib.reload(run)
            run.main()
            finished.add(job['id'])
            history.write_text(json.dumps(sorted(finished)))
    except Exception as e:
        (run.ROOT/'STOPPED.txt').write_text(f'{type(e).__name__}: {e}\n')
        run.log('STOPPED '+repr(e))
        raise
    finally:
        restore()


if __name__=='__main__':
    main()
