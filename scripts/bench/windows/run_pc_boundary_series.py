#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Controller for the fixed borrowed-PC, six-run boundary comparison.

Only SSH to the pinned Linux host; no credentials enter the guest. Unique run
directories and per-run status records preserve interrupted/failed experiments.
"""
import base64
import datetime
import json
from pathlib import Path
import shlex
import subprocess
import time

HERE = Path(__file__).resolve().parent
ROOT = Path('/data/kayfabe-runtime/windows-boundary-20261005')
REMOTE = '/var/lib/kf-windows-20261005'
SSH = ['ssh', '-F', '/dev/null', '-o', 'BatchMode=yes', '-o', 'IdentitiesOnly=yes',
       '-o', 'IdentityAgent=none', '-o', 'ForwardAgent=no', '-o', 'StrictHostKeyChecking=yes',
       '-o', 'ConnectTimeout=8', '-i', '/root/.ssh/id_ed25519', 'root@172.22.1.20']
SCP = ['scp', '-F', '/dev/null', '-o', 'BatchMode=yes', '-o', 'IdentitiesOnly=yes',
       '-o', 'IdentityAgent=none', '-o', 'ForwardAgent=no', '-o', 'StrictHostKeyChecking=yes',
       '-i', '/root/.ssh/id_ed25519']


def remote(argv, timeout=60, check=True):
    return subprocess.run(SSH+[shlex.join(map(str, argv))], capture_output=True,
                          text=True, timeout=timeout, check=check)


def guest(work, script):
    encoded = base64.b64encode(script.encode('utf-16-le')).decode()
    return remote(['python3', REMOTE+'/boundary-tools/qmp.py', work+'/qga.sock',
                   'qga-exec', r'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe', '-NoProfile', '-NonInteractive',
                   '-EncodedCommand', encoded], timeout=150)


def fetch(work, local, names):
    for name in names:
        subprocess.run(SCP+[f'root@172.22.1.20:{work}/{name}', str(local/name)],
                       check=True, capture_output=True, timeout=180)


def log(message):
    print(datetime.datetime.now(datetime.timezone.utc).isoformat(), message, flush=True)


def main():
    ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    script = (HERE/'boundary_status.ps1').read_text()
    for arm, number in [('vfio', 4), ('kayfabe', 1), ('vfio', 5),
                        ('kayfabe', 2), ('vfio', 6), ('kayfabe', 3)]:
        name = f'boundary-{arm}-{number}'
        local = ROOT/name
        local.mkdir(mode=0o700)
        work = REMOTE+'/'+name
        unit = 'kf-'+name
        log('START '+name)
        remote(['systemd-run', '--unit='+unit, '--property=RuntimeMaxSec=900',
                '--property=KillMode=mixed', '--property=TimeoutStopSec=180',
                '/usr/bin/python3', '-u', REMOTE+'/boundary-tools/pc_boundary_experiment.py',
                '--arm', arm, '--run', number, '--no-mmio-trace'])
        started = time.monotonic()
        try:
            deadline = time.monotonic()+300
            while True:
                ready = remote(['python3', REMOTE+'/boundary-tools/qmp.py', work+'/qga.sock',
                                'qga-ping'], timeout=20, check=False)
                if ready.returncode == 0:
                    break
                state = remote(['systemctl', 'is-active', unit], check=False)
                if state.stdout.strip() != 'active' or time.monotonic() >= deadline:
                    raise RuntimeError('Guest agent did not become ready; unit='+state.stdout.strip())
                time.sleep(5)
            # Observe after at least90s of guest uptime and require two stable
            # status observations. Do not mistake an early basic-adapter state
            # for the completed driver initialization outcome.
            deadline = time.monotonic()+180
            previous = None
            observation = 0
            while True:
                result = guest(work, script)
                (local/f'status-{observation}.json').write_text(result.stdout)
                (local/f'status-{observation}.stderr').write_text(result.stderr)
                status = json.loads(result.stdout.lstrip('\ufeff'))
                outcome = (status['driver_sha256'], status['display'], status['nvidia_smi']['exit_code'])
                if status['uptime_seconds'] >= 90 and previous == outcome:
                    (local/'status.json').write_text(json.dumps(status, indent=2)+'\n')
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError('Startup status did not settle')
                previous = outcome
                observation += 1
                time.sleep(10)
            capture = remote(['python3', REMOTE+'/boundary-tools/capture_display_caps.py', work])
            (local/'capture.stdout').write_text(capture.stdout)
            log(name+' status='+str(status['nvidia_smi']['exit_code'])+' caps='+capture.stdout.strip())
            fetch(work, local, ['command.json', 'display-caps.bin', 'display-caps.json'])
            guest(work, '& shutdown.exe /s /t 0; if ($LASTEXITCODE) { exit $LASTEXITCODE }')
            deadline = time.monotonic()+120
            while remote(['systemctl', 'is-active', unit], check=False).stdout.strip() == 'active':
                if time.monotonic() >= deadline:
                    raise RuntimeError('Clean shutdown did not complete')
                time.sleep(3)
            result = remote(['systemctl', 'show', unit, '-p', 'Result', '-p', 'ExecMainStatus'])
            (local/'unit-result.txt').write_text(result.stdout)
            if 'ExecMainStatus=0' not in result.stdout:
                raise RuntimeError('Runner/host restoration failed: '+result.stdout)
            fetch(work, local, ['qemu.log', 'serial.log'])
            if arm == 'vfio':
                fetch(work, local, ['vfio-state.json'])
            health = remote(['nvidia-smi', '--query-gpu=name,driver_version', '--format=csv,noheader'])
            (local/'host-health.txt').write_text(health.stdout)
            (local/'complete.json').write_text(json.dumps(dict(complete=True, elapsed=time.monotonic()-started))+'\n')
            log('COMPLETE '+name+'; host GPU restored/healthy')
        except Exception as e:
            detail = type(e).__name__+': '+str(e)[:300]
            if isinstance(e, subprocess.CalledProcessError):
                (local/'command-error.stdout').write_text(e.stdout or '')
                (local/'command-error.stderr').write_text(e.stderr or '')
                detail = f'Command exited {e.returncode}; see command-error.stderr'
            (local/'controller-error.txt').write_text(detail+'\n')
            log('FAILED '+name+': '+detail)
            # The remote bounded supervisor owns shutdown and restoration. Stop
            # its service once, then record result; never start another GPU run.
            remote(['systemctl', 'stop', unit], timeout=180, check=False)
            journal = remote(['journalctl', '-u', unit, '-n', '60', '--no-pager'], check=False)
            (local/'failure-journal.txt').write_text(journal.stdout)
            raise


if __name__ == '__main__':
    main()
