#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""One fresh-boot measurement. Caller supplies private host paths via environment.

Refuses to run without the exclusive shared flock. All waits have bounds;
shutdown refuses force-kill. The owner launcher is copied, never edited.
"""
import argparse
import base64
import datetime as dt
import gzip
import hashlib
import json
import importlib
import traceback
import os
from pathlib import Path
import re
import subprocess as sp
import time

import metric
import identity
import archive

ROOT = Path(os.environ['N1_ROOT'])
W = Path(os.environ['N1_WINDOWS'])
LAUNCHER = Path(os.environ['N1_LAUNCHER'])
AGENT = os.environ['N1_AGENT']
STOP = Path(os.environ['N1_STOP'])
SCRIPTS = Path(__file__).resolve().parent


class HostSafetyError(RuntimeError):
    pass


def log(s):
    print(dt.datetime.now().astimezone().isoformat(), s, flush=True)


def run(argv, timeout=90, check=True):
    return sp.run(argv, capture_output=True, text=True, timeout=timeout, check=check)


def agent(script, timeout=90):
    # Keep a literal script file receipt; no shell interpolation of PowerShell.
    (ROOT/'last-agent.ps1').write_text(script)
    return run(['python3', AGENT, 'run', script, str(timeout)], timeout=timeout+5).stdout


def alive(pid):
    try:
        return Path(f'/proc/{pid}/stat').read_text().split()[2] != 'Z'
    except FileNotFoundError:
        return False


def wait_for(f, seconds, what):
    until = time.monotonic()+seconds
    while time.monotonic() < until:
        if f():
            return
        time.sleep(1)
    raise RuntimeError('REFUSED: bounded wait failed: '+what)


def stop_session(pid, launch_log):
    if alive(pid):
        metadata=json.loads((Path(launch_log).parent/'run.json').read_text())
        argv=Path(f'/proc/{pid}/cmdline').read_bytes().decode().strip('\0').split('\0')
        if not identity.is_qemu(argv,metadata['run']):
            raise HostSafetyError('recorded QEMU PID identity changed; no action taken')
        log(f'clean shutdown requested for owned/authorized QEMU {pid}')
        agent('shutdown.exe /s /t 5; "N1_SHUTDOWN_REQUESTED"', 30)
        wait_for(lambda: not alive(pid), 180, 'clean QEMU exit (no force-kill permitted)')
    STOP.touch()
    wait_for(lambda: 'OWNER_SESSION_EXIT=' in Path(launch_log).read_text(errors='replace'), 30,
             'OWNER_SESSION_EXIT after clean shutdown')
    log('clean stop complete')


def next_run():
    n = max([3094]+[int(p.name.rsplit('-', 1)[1]) for p in W.glob('boundary-kayfabe-*')
                   if p.name.rsplit('-', 1)[1].isdigit()])+1
    return n


def snapshot(path):
    d = run(['dmesg'], check=False).stdout
    path.write_text('\n'.join(l for l in d.splitlines() if 'NVRM' in l)+'\n')
    return d.count('NVRM: Xid')


def copy_launcher(out, drop):
    s = LAUNCHER.read_text()
    # The copy's helper keeps all launch behavior, but refuses unclean cleanup.
    helper = re.search(r'^B=(.+)$', s, re.M).group(1)
    b = Path(helper).read_text()
    bad = 'kill "$q"; wait_gone "$q" 10 || kill -9 "$q" 2>/dev/null; how=UNCLEAN-kill'
    if b.count(bad) != 1:
        raise RuntimeError('REFUSED: unrecognized helper cleanup; review required')
    b = b.replace(bad, 'die "N1 REFUSED: no clean shutdown; force kill forbidden"')
    if 'QBIN=$W/kf3-bins/$KF3_REV/qemu-system-x86_64' not in b:
        raise RuntimeError('REFUSED: unknown binary selection')
    # Optional G binary may be under the diagnostic lane's immutable output.
    b = b.replace('QBIN=$W/kf3-bins/$KF3_REV/qemu-system-x86_64',
                  'QBIN=${N1_QBIN:-$W/kf3-bins/$KF3_REV/qemu-system-x86_64}')
    (out/'broker.sh').write_text(b)
    s = re.sub(r'^B=.+$', 'B='+str(out/'broker.sh'), s, flags=re.M)
    if drop:
        s = s.replace('WIN_DROP="KF3_TIMER_MAP"', 'WIN_DROP="KF3_TIMER_MAP '+drop+'"')
    s = s.replace('test -x "$W/kf3-bins/$REV/qemu-system-x86_64"',
                  'test -x "${N1_QBIN:-$W/kf3-bins/$REV/qemu-system-x86_64}"')
    (out/'launcher.sh').write_text(s)


def verify_lock():
    probe = run(['flock', '-n', os.environ['N1_LOCK'], 'true'], check=False)
    if probe.returncode == 0:
        raise RuntimeError('REFUSED: exclusive shared lock not held')
    # run.sh owns the flock; this process is its child (flock -o).
    cmd = Path(f'/proc/{os.getppid()}/cmdline').read_bytes().split(b'\0')
    if not any(b'flock' in c for c in cmd):
        raise RuntimeError('REFUSED: use run.sh under the shared flock')


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--arm', required=True)
    p.add_argument('--rev', required=True)
    p.add_argument('--minutes', type=float, default=12)
    p.add_argument('--flags', default='KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0')
    p.add_argument('--drop', default='')
    p.add_argument('--qbin')
    p.add_argument('--transport-preflight',action='store_true')
    p.add_argument('--pair', choices=['heaven-gravity','heaven-heaven','heaven-blender','heaven-edge'], default='heaven-gravity')
    args = p.parse_args()
    if not 0 < args.minutes <= 15:
        raise RuntimeError('REFUSED: duration outside (0,15] minutes')
    verify_lock()
    # The exact deadline is explicit (CEST has UTC+02 on this date).
    deadline = dt.datetime.fromisoformat(os.environ['N1_DEADLINE']).timestamp()
    if time.time()+args.minutes*60+600 >= deadline:
        raise RuntimeError('REFUSED: insufficient time for run and clean restoration')
    free = os.statvfs(W).f_bavail*os.statvfs(W).f_frsize
    if free <= 53687091200+500_000_000:
        raise HostSafetyError('REFUSED: insufficient host disk restoration margin')
    # Never start against another VM using the persistent owner disk.
    for cmd in Path('/proc').glob('[0-9]*/cmdline'):
        try:
            if b'qemu-system' in cmd.read_bytes() and b'file='+str(W).encode()+b'/boundary-kayfabe-' in cmd.read_bytes():
                raise RuntimeError('REFUSED: another Windows VM is still running')
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            pass
    n = next_run()
    out = ROOT/f'{args.arm}-{n}'
    out.mkdir()
    copy_launcher(out, args.drop)
    before = snapshot(out/'nvrm-before.txt')
    meta = vars(args) | dict(run=n, started_utc=dt.datetime.now(dt.timezone.utc).isoformat(),
            source_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            gpu=run(['nvidia-smi','--query-gpu=name,pci.device_id,driver_version','--format=csv,noheader']).stdout.strip(),
            host_module=run(['modinfo','-F','license','nvidia']).stdout.strip())
    (out/'run.json').write_text(json.dumps(meta, indent=2))
    env = os.environ | dict(REV=args.rev, N=str(n), WIN_FLAGS=args.flags)
    if args.qbin:
        env['N1_QBIN'] = args.qbin
    STOP.unlink(missing_ok=True)
    stream = (out/'launch.log').open('w')
    launcher = sp.Popen(['bash', str(out/'launcher.sh')], env=env, stdout=stream, stderr=sp.STDOUT)
    log(f'BOOT arm={args.arm} rev={args.rev} run={n} dir={out}')
    wait_for(lambda: 'OWNER_SESSION_READY=' in (out/'launch.log').read_text(), 90, 'launcher ready')
    state = (W/f'windows-broker-run{n}.state').read_text()
    pid = int(re.search(r'^QPID=(\d+)$', state, re.M).group(1))
    (out/'qpid').write_text(str(pid))
    # One queued readiness command; no retries that leave stale commands for later boots.
    ready = agent('"N1_AGENT_READY"; (Get-CimInstance Win32_OperatingSystem).LastBootUpTime', 180)
    if 'N1_AGENT_READY' not in ready:
        raise RuntimeError('REFUSED: guest agent unavailable')
    guest_root = f'C:\\kf\\n1\\run{n}'
    installer = ['$ErrorActionPreference="Stop"', 'New-Item -ItemType Directory -Force C:\\kf\\n1 | Out-Null']
    for name in ['load.ps1','post_boot.ps1','collect.ps1','transport.ps1','health.ps1','viewport.py','webgl.html','test_transport.ps1','preflight.ps1','archive.ps1']:
        data = base64.b64encode((SCRIPTS/name).read_bytes()).decode()
        installer.append(f'[IO.File]::WriteAllBytes("C:\\kf\\n1\\{name}",[Convert]::FromBase64String("{data}"))')
    if args.transport_preflight:
        installer.append(f'& C:\\kf\\n1\\preflight.ps1 -Scripts C:\\kf\\n1 -Root {guest_root}\\transport-preflight')
    installer.append(f'& C:\\kf\\n1\\post_boot.ps1 -Root {guest_root} -Seconds {int(args.minutes*60)} -Pair {args.pair}')
    boot = agent('\n'.join(installer), 90)
    (out/'post-boot.txt').write_text(boot)
    for tag in ['GREEN','MUTANT']:
        match=re.search(r'PREFLIGHT_'+tag+r'=([A-Za-z0-9+/=]+)',boot)
        if match: (out/f'transport-{tag.lower()}.json').write_bytes(base64.b64decode(match.group(1)))
    if 'POST_BOOT_OK' not in boot:
        raise RuntimeError('REFUSED: post-boot instrumentation/load setup failed')
    actual = json.loads((W/f'boundary-kayfabe-{n}'/'command.json').read_text())
    (out/'command.json').write_text(json.dumps(actual, indent=2))
    probe = None
    if os.environ.get('N1_PROBE'):
        probe = sp.Popen(['python3', os.environ['N1_PROBE'], '--name',f'boundary-kayfabe-{n}',
             '--watch', str(int(args.minutes*60)+15), '--tsv', str(out/'probe.tsv'),
             '--dump-file', str(out/'stuck.txt')], stdout=(out/'probe.log').open('w'), stderr=sp.STDOUT)
    gpu = sp.Popen(['nvidia-smi','--query-gpu=timestamp,utilization.gpu,power.draw,memory.used',
                    '--format=csv','-l','1'], stdout=(out/'gpu.csv').open('w'), stderr=sp.STDOUT)
    (out/'collectors.json').write_text(json.dumps([gpu.pid]+([probe.pid] if probe else [])))
    log('LOAD_STARTED '+str(out))
    until=time.monotonic()+args.minutes*60
    last_safety=0
    while time.monotonic() < until:
        if not alive(pid):
            raise RuntimeError('REFUSED: test VM exited during load')
        if time.time() >= deadline:
            raise RuntimeError('REFUSED: hard stop reached')
        if time.monotonic()-last_safety>=5:
            last_safety=time.monotonic()
            if snapshot(out/'nvrm-live.txt')>before:
                raise HostSafetyError('new host Xid; stop experiments and restore owner')
            fs=os.statvfs(W)
            if fs.f_bavail*fs.f_frsize<=53687091200+200_000_000:
                raise HostSafetyError('host disk restoration reserve reached')
        time.sleep(min(1,max(0,until-time.monotonic())))
    try:
        payload = agent(f'& C:\\kf\\n1\\collect.ps1 -Root {guest_root}', 90)
        (out/'guest-payload.json').write_text(payload)
        pack=json.loads(payload.lstrip('\ufeff'))
        (out/'guest.json').write_text(json.dumps(pack['meta'], indent=2))
        (out/'guest-state.json').write_text(json.dumps({k:v for k,v in pack.items() if k!='files'}, indent=2))
        for name, data in pack['files'].items():
            if Path(name).name != name:
                raise RuntimeError('REFUSED: unsafe guest evidence filename')
            (out/name).write_bytes(gzip.decompress(base64.b64decode(data)))
        importlib.reload(metric)
        result = metric.analyze(out)
    except Exception as error:
        (out/'COLLECTOR_ERROR.txt').write_text(traceback.format_exc())
        log('COLLECTOR_ERROR retained for test-first repair: '+repr(error))
        result=dict(schema=2,load_present=False,metric_status='COLLECTOR_ERROR',error=repr(error),apps={},classes=None)
    after = snapshot(out/'nvrm-after.txt')
    if after>before:
        raise HostSafetyError('new host Xid at collection; restore owner')
    qlog=W/f'boundary-kayfabe-{n}'/'qemu.log'
    counter_lines=[]; diag=[]
    with qlog.open(errors='replace') as f:
        for line in f:
            if re.search(r'USER_RING|BACKSTOP|HELD.REPLY|LIFECYCLE', line):
                counter_lines.append(line.rstrip())
                counter_lines=counter_lines[-200:]
            if 'USER_RING_STALL_DIAG' in line or re.search(r'SEMAPHORE|SEM_ACQ',line,re.I):
                diag.append(line.rstrip())
    (out/'host-counters.txt').write_text('\n'.join(counter_lines)+'\n')
    (out/'stall-diag.txt').write_text('\n'.join(diag)+'\n')
    result.update(pair=args.pair,arm=args.arm, rev=args.rev, run=n, flags=actual['flags'], xid_before=before,
                  xid_after=after, xid_delta=after-before, duration_requested_min=args.minutes,
                  gpu=meta['gpu'], host_module=meta['host_module'])
    (out/'RESULT.json').write_text(json.dumps(result, indent=2))
    log('RESULT '+json.dumps({a:{k:r[k] for k in ['count','longest_s','right_censored']} for a,r in result['apps'].items()}))
    if (out/'trig.log').exists():
        try:
            result['trace_archives']=archive.preserve(out,guest_root,agent)
            for earlier in sorted(ROOT.glob('*-*/RESULT.json')):
                previous=earlier.parent
                if previous==out or not (previous/'trig.log').exists() or not (previous/'guest.json').exists(): continue
                info=json.loads((previous/'guest.json').read_text())
                archive.preserve(previous,info['root'],agent)
            (out/'RESULT.json').write_text(json.dumps(result,indent=2))
        except Exception:
            (out/'ARCHIVE_ERROR.txt').write_text(traceback.format_exc())
            log('ARCHIVE_ERROR: original guest captures retained; repair required')
    gpu.terminate(); gpu.wait(timeout=10)
    if probe and probe.poll() is None:
        probe.terminate(); probe.wait(timeout=10)
    stop_session(pid, out/'launch.log')
    launcher.wait(timeout=30)
    final_xid=snapshot(out/'nvrm-after-stop.txt')
    result['xid_after_clean_stop']=final_xid
    with qlog.open(errors='replace') as f:
        summaries=[line.rstrip() for line in f if 'USER_RING_LOOP SUMMARY' in line]
    (out/'ring-summary-final.txt').write_text('\n'.join(summaries)+'\n')
    (out/'RESULT.json').write_text(json.dumps(result,indent=2))
    if final_xid>before:
        raise HostSafetyError('new host Xid during cleanup; restore owner')
    log('RUN_COMPLETE '+str(out))


if __name__=='__main__':
    main()
