#!/usr/bin/env python3
"""One trusted-host VFIO session, launched under flock -o, with desktop restoration.

No observer or device-emulation changes. The caller supplies the existing owner's
command.json and a lane directory. Never run against an active owner VM.
"""
import argparse
import datetime as dt
import json
import os
import re
import select
from pathlib import Path
import signal
import socket
import subprocess
import time

GPU = '0000:01:00.0'
AUDIO = '0000:01:00.1'
PCI = Path('/sys/bus/pci/devices')



DESKTOP_NAMES = {'gnome-shell', 'Xwayland', 'Xorg', 'nvidia-smi', 'nvtop',
                 'nvkvm-display-b', 'nvkvm-display-br', 'firefox', 'firefox-bin'}


def cleanup_names(holders):
    """The owner override covers desktop names, never unknown/compute holders."""
    for pid, name in holders.items():
        if (pid == 1 and name == 'systemd') or name == 'systemd-logind':
            continue  # Release logind's DRM descriptors through its service.
        if name not in DESKTOP_NAMES:
            raise RuntimeError(f'PROTECTED_GPU_USER pid={pid} name={name}')
    return sorted({name for pid, name in holders.items()
                   if pid != 1 and name != 'systemd-logind'})


def open_holders(nodes):
    if not nodes:
        return {}
    r = subprocess.run(['fuser', '-v', *nodes], text=True, capture_output=True, timeout=20)
    print('FUSER_LIST_BEFORE_ACTION', r.stderr.strip(), r.stdout.strip(), flush=True)
    holders = {}
    for token in re.findall(r'\b[0-9]+\b', r.stdout):
        pid = int(token)
        try:
            name = (Path('/proc') / str(pid) / 'comm').read_text().strip()
            holders[pid] = name
            print('GPU_HOLDER', pid, name, flush=True)
        except FileNotFoundError:
            pass
    return holders


def close_desktop_users():
    for unit in ['gdm', 'gdm3']:
        run('systemctl', 'stop', unit, check=False)
    run('loginctl', 'terminate-user', 'ubuntu', check=False)
    run('systemctl', 'stop', 'nvidia-persistenced', check=False)
    nodes = sorted({*gpu_nodes(), *(str(p) for p in Path('/dev/dri').glob('*') if not p.is_dir())})
    holders = open_holders(nodes)
    names = cleanup_names(holders)  # Refuse the entire kill set if any holder is protected.
    waiters = select.poll()
    handles = []
    for pid, name in holders.items():
        if name not in names:
            continue
        try:
            fd = os.pidfd_open(pid)
            handles.append(fd)
            waiters.register(fd, select.POLLIN)
        except ProcessLookupError:
            pass
    try:
        for name in names:
            print('AUTHORIZED_DESKTOP_TERM_BY_NAME', name, flush=True)
            run('pkill', '-TERM', '-x', '--', name, check=False)
        pending = set(handles)
        until = time.monotonic() + 30
        while pending and time.monotonic() < until:
            for fd, event in waiters.poll(max(1, int((until-time.monotonic())*1000))):
                pending.discard(fd)
                waiters.unregister(fd)
        if pending:
            raise RuntimeError('SAFETY_REFUSED: authorized desktop process did not exit')
    finally:
        for fd in handles:
            os.close(fd)
    # logind and PID 1 may retain DRM fdstore references after the desktop exits.
    # systemctl documents stopping the unit and cleaning only its fdstore.
    run('systemctl', 'stop', 'systemd-logind')
    run('systemctl', 'clean', '--what=fdstore', 'systemd-logind')
    remaining = open_holders(nodes)
    if remaining:
        raise RuntimeError('SAFETY_REFUSED: GPU nodes remain open after authorized cleanup')


def make_argv(owner, root):
    """Retain CPU, memory, disk options and identity; replace writable paths/GPU."""
    result = [owner[0]]
    tokens = iter(owner[1:])
    for option in tokens:
        if option in {'-nodefaults', '-no-user-config', '-no-shutdown', '-no-reboot', '-enable-kvm'}:
            result.append(option)
            continue
        value = next(tokens)
        if option == '-name':
            value = 'codex-VF1-specviewperf'
        elif option == '-device' and value.startswith('kf3-gpu,'):
            result += ['-device', f'vfio-pci,host={GPU},bus=pci.0,addr=0x6.0,multifunction=on',
                       '-device', f'vfio-pci,host={AUDIO},bus=pci.0,addr=0x6.1']
            continue
        elif option == '-drive':
            fields = value.split(',')
            if 'unit=1' in fields and 'if=pflash' in fields:
                fields = [f'file={root}/OVMF_VARS.fd' if x.startswith('file=') else x for x in fields]
            elif 'id=disk0' in fields:
                fields = [f'file={root}/vfio-overlay.qcow2' if x.startswith('file=') else x for x in fields]
            value = ','.join(fields)
        elif option == '-qmp':
            value = f'unix:{root}/qmp.sock,server=on,wait=off'
        elif option == '-serial':
            value = f'file:{root}/serial.log'
        elif option == '-chardev' and value.startswith('socket,id=qga0,'):
            value = f'socket,id=qga0,path={root}/qga.sock,server=on,wait=off'
        result += [option, value]
    if any('kf3-gpu' in x for x in result):
        raise ValueError('kayfabe device remained in native argv')
    if sum(x.startswith('vfio-pci,host=') for x in result) != 2:
        raise ValueError('native argv requires both GPU functions')
    return result


def abort_reason(*, xid_before, xid_now, now, deadline, progress_at, alive, stop):
    if xid_now != xid_before:
        return 'NEW_HOST_XID'
    if now >= deadline:
        return 'RESTORE_DEADLINE'
    if stop:
        return 'REQUESTED_STOP'
    if not alive:
        return 'QEMU_EXIT'
    if now - progress_at >= 900:
        return 'NO_PROGRESS_15_MIN'
    return None


def run(*args, check=True, timeout=45):
    r = subprocess.run(args, text=True, stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, timeout=timeout)
    if r.stdout.strip():
        print(r.stdout.strip(), flush=True)
    if check and r.returncode:
        raise RuntimeError(f'command failed ({r.returncode}): {args[0]}')
    return r


def xid_count():
    return subprocess.check_output(['dmesg'], text=True).count('NVRM: Xid')


def driver(bdf):
    link = PCI / bdf / 'driver'
    return link.resolve().name if link.exists() else 'unbound'


def write(path, value):
    Path(path).write_text(value + '\n')


def bind(bdf, target):
    write(PCI / bdf / 'driver_override', target)
    if driver(bdf) != 'unbound':
        write(PCI / bdf / 'driver/unbind', bdf)
    write('/sys/bus/pci/drivers_probe', bdf)
    if driver(bdf) != target:
        raise RuntimeError(f'{bdf} did not bind {target}')


def qmp(root, command):
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(10)
        sock.connect(str(root / 'qmp.sock'))
        f = sock.makefile('rwb', buffering=0)
        json.loads(f.readline())
        for cmd in ['qmp_capabilities', command]:
            f.write(json.dumps({'execute': cmd}).encode() + b'\n')
            while True:
                answer = json.loads(f.readline())
                if 'return' in answer:
                    break
                if 'error' in answer:
                    raise RuntimeError(f'QMP {cmd}: {answer}')


def gpu_nodes():
    nodes = list(Path('/dev').glob('nvidia*'))
    for entry in Path('/sys/class/drm').glob('*'):
        if (entry / 'device').resolve() == (PCI / GPU).resolve():
            node = Path('/dev/dri') / entry.name
            if node.exists():
                nodes.append(node)
    return [str(p) for p in nodes if not p.is_dir()]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--owner-command', type=Path, required=True)
    p.add_argument('--desktop-takeover', action='store_true', help='owner-authorized desktop cleanup')
    p.add_argument('--restore-at', required=True, help='ISO-8601 deadline with timezone')
    a = p.parse_args()
    root = a.root.resolve()
    if root != Path('/data/codex-VF1'):
        p.error('this lane writes only under /data/codex-VF1')
    deadline = dt.datetime.fromisoformat(a.restore_at).timestamp()
    if os.geteuid() != 0 or deadline <= time.time():
        p.error('root and a future restoration deadline required')
    if run('flock', '-n', '/tmp/kayfabe-fastguest.lock', 'true', check=False).returncode == 0:
        p.error('caller must hold flock -o /tmp/kayfabe-fastguest.lock')
    # Reject every live QEMU, including one unrelated to this lane; ignore zombies.
    for entry in Path('/proc').glob('[0-9]*/stat'):
        try:
            fields = entry.read_text().split()
            if 'qemu-system' in fields[1] and fields[2] != 'Z':
                p.error('another live QEMU exists')
        except FileNotFoundError:
            pass
    owner = json.loads(a.owner_command.read_text())
    argv = make_argv(owner['argv'], root)
    for needed in ['vfio-overlay.qcow2', 'OVMF_VARS.fd', 'progress.ps1']:
        if not (root / needed).is_file():
            p.error('missing precondition: ' + needed)
    if driver(GPU) != 'nvidia' or driver(AUDIO) != 'snd_hda_intel':
        p.error('unexpected initial GPU/audio binding')
    baseline = xid_count()
    if baseline != 26:
        p.error('host Xid count changed from lane baseline 26; abort before detach')
    write(root / 'supervisor.pid', str(os.getpid()))
    (root / 'command.json').write_text(json.dumps({'source_rev': owner.get('kf3_rev'),
        'argv': argv, 'xid_before': baseline, 'restore_at': a.restore_at}, indent=2)+'\n')
    persist_was_active = subprocess.run(['systemctl', 'is-active', '--quiet', 'nvidia-persistenced']).returncode == 0
    logind_was_active = subprocess.run(['systemctl', 'is-active', '--quiet', 'systemd-logind']).returncode == 0
    proc = None
    touched = False
    requested = False
    reason = 'PREFLIGHT_FAILED'
    restored = False

    def stop_signal(signum, frame):
        nonlocal requested
        requested = True
    signal.signal(signal.SIGTERM, stop_signal)
    signal.signal(signal.SIGINT, stop_signal)
    try:
        touched = True
        if a.desktop_takeover:
            close_desktop_users()
        else:
            run('systemctl', 'stop', 'display-manager')
            run('systemctl', 'stop', 'nvidia-persistenced', check=False)
            if open_holders(gpu_nodes()):
                raise RuntimeError('SAFETY_REFUSED: GPU nodes still open; desktop takeover not selected')
        for module in ['nvidia_drm', 'nvidia_modeset', 'nvidia_uvm']:
            run('modprobe', '-r', module)
        run('modprobe', 'vfio-pci')
        bind(AUDIO, 'vfio-pci')
        bind(GPU, 'vfio-pci')
        if xid_count() != baseline:
            raise RuntimeError('NEW_HOST_XID during detach')
        with (root / 'qemu.log').open('wb') as log:
            proc = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        write(root / 'qemu.pid', str(proc.pid))
        print('VFIO_QEMU_STARTED', proc.pid, dt.datetime.now(dt.timezone.utc).isoformat(), flush=True)
        progress_at = time.time()
        signature = None
        next_probe = 0
        while True:
            now = time.time()
            reason = abort_reason(xid_before=baseline, xid_now=xid_count(), now=now,
                deadline=deadline, progress_at=progress_at, alive=proc.poll() is None,
                stop=requested or (root / 'stop').exists())
            if reason:
                print('RESTORE_REASON', reason, flush=True)
                break
            if now >= next_probe:
                # Only benchmark log changes establish progress; successful agent replies alone do not.
                r = subprocess.run(['python3', '/data/kfagent_server.py', 'run',
                    (root / 'progress.ps1').read_text(), '25'], capture_output=True, text=True, timeout=35)
                (root / 'guest-progress.txt').write_text(r.stdout + r.stderr)
                try:
                    status = json.loads(r.stdout.strip())
                    current = status['signature']
                    if current and current != signature:
                        signature = current
                        progress_at = time.time()
                    (root / 'progress-status.json').write_text(json.dumps({
                        'observed': time.time(), 'progress_at': progress_at, 'guest': status})+'\n')
                except (ValueError, KeyError):
                    pass
                next_probe = time.time() + 15
            time.sleep(2)
    except Exception as error:
        reason = str(error)
        print('SESSION_ERROR', reason, flush=True)
    finally:
        if proc is not None and proc.poll() is None:
            try:
                qmp(root, 'quit' if reason == 'NO_PROGRESS_15_MIN' else 'system_powerdown')
            except Exception as error:
                print('ACPI_ERROR', error, flush=True)
            try:
                proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                # Lane authorizes QMP quit on hang; no SIGKILL is used.
                try:
                    qmp(root, 'quit')
                    proc.wait(timeout=30)
                    print('UNCLEAN_QMP_QUIT', flush=True)
                except Exception as error:
                    print('QMP_QUIT_ERROR', error, flush=True)
                    proc.terminate()  # Only the Popen child owned by this session.
                    try:
                        proc.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        print('RESTORE_BLOCKED: owned QEMU still alive; SSH remains available', flush=True)
        if touched and (proc is None or proc.poll() is not None):
            errors = []
            for bdf, target in [(GPU, 'nvidia'), (AUDIO, 'snd_hda_intel')]:
                try:
                    run('modprobe', target)
                    if driver(bdf) != target:
                        bind(bdf, target)
                    write(PCI / bdf / 'driver_override', '')
                except Exception as error:
                    errors.append(str(error))
            restore_commands = [('modprobe', 'nvidia_uvm'), ('modprobe', 'nvidia_modeset'),
                         ('modprobe', 'nvidia_drm', 'modeset=1', 'fbdev=1'),
                         ]
            if logind_was_active:
                restore_commands.append(('systemctl', 'start', 'systemd-logind'))
            if persist_was_active:
                restore_commands.append(('systemctl', 'start', 'nvidia-persistenced'))
            restore_commands.append(('systemctl', 'start', 'gdm'))
            for args in restore_commands:
                try:
                    run(*args)
                except Exception as error:
                    errors.append(str(error))
            smi = run('nvidia-smi', '--query-gpu=name,driver_version', '--format=csv,noheader', check=False)
            if smi.returncode:
                errors.append('nvidia-smi failed after restoration')
            run('loginctl', 'list-sessions', '--no-legend', check=False)
            restored = not errors and driver(GPU) == 'nvidia' and driver(AUDIO) == 'snd_hda_intel'
            print('RESTORED', restored, 'ERRORS', errors, flush=True)
        summary = {'reason': reason, 'restored': restored, 'xid_before': baseline,
                   'xid_after': xid_count(), 'qemu_exit': None if proc is None else proc.poll(),
                   'ended_utc': dt.datetime.now(dt.timezone.utc).isoformat()}
        (root / 'session-result.json').write_text(json.dumps(summary, indent=2)+'\n')
    return 0 if restored else 1


if __name__ == '__main__':
    raise SystemExit(main())
