#!/usr/bin/env python3
"""Resume the preserved 2026-10-04 native RTX 4070 fixture in an independent clone.

Exact borrowed-host experiment, not an installer or a Kayfabe success test.
Uses the pinned public vast-windows VFIO helper and its recovery journal.
Only virtual OVMF variable files change; physical firmware is never written.
"""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workdir', type=Path, required=True)
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('Root is required for this exact-host VFIO experiment')
    os.umask(0o077)
    helper = Path('/root/vast-windows-test/prepare/vfio_4070.py')
    pins = {
        helper: '79262fcbfbc7bab06c0cdc027d6bccc89b22864331ce6075bb1e27c990d47c67',
        helper.with_name('prepare.py'): '5013d628ef4d63e07d7952666d4ce2c0723186c6d41559c203d488c29918cf73',
    }
    for path, expected in pins.items():
        if digest(path) != expected:
            raise RuntimeError(f'Pinned helper changed: {path}')
    spec = importlib.util.spec_from_file_location('vfio4070', helper)
    vfio = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(vfio)
    with open('/run/vast-windows-4070.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        work = args.workdir.resolve()
        if work.exists() or any(c in str(work) for c in ',\n') or len(os.fsencode(work/'qga.sock')) >= 104:
            raise RuntimeError('Require a fresh directory suitable for QEMU paths')
        source = Path('/var/lib/vast-windows-4070-session/staging.qcow2')
        old_vars = source.with_name('vfio-OVMF_VARS.fd')
        old_state = json.loads(source.with_name('vfio-state.json').read_text())
        pid = old_state.get('qemu_pid')
        if pid and vfio.process_token(pid) == old_state.get('qemu_start_token'):
            raise RuntimeError('Previous fixture QEMU still runs')
        if not source.is_file() or source.is_symlink() or not old_vars.is_file():
            raise RuntimeError('Preserved fixture is absent or unexpected')
        before = vfio.inventory()
        vfio.require_profile(before['devices'], before['amd'])
        if not os.access('/dev/kvm', os.R_OK | os.W_OK):
            raise RuntimeError('/dev/kvm unavailable')
        info = json.loads(vfio.command(['qemu-img', 'info', '--output=json', source]).stdout)
        if info['format'] != 'qcow2' or info['virtual-size'] != 150 * 1024**3 or info.get('backing-filename'):
            raise RuntimeError('Unexpected preserved disk geometry or backing dependency')
        vfio.command(['qemu-img', 'check', '-f', 'qcow2', source])
        if shutil.disk_usage(work.parent).free < max(30 * 1024**3, source.stat().st_size * 2):
            raise RuntimeError('Insufficient space for an independent clone')
        work.mkdir(mode=0o700)
        shutil.copyfile(old_vars, work/'vfio-OVMF_VARS.fd')
        vfio.command(['qemu-img', 'convert', '-f', 'qcow2', '-O', 'qcow2', source, work/'staging.qcow2'])
        vfio.command(['qemu-img', 'check', '-f', 'qcow2', work/'staging.qcow2'])
        vfio.command(['qemu-img', 'compare', '-f', 'qcow2', '-F', 'qcow2', source, work/'staging.qcow2'])
        cfg_path = Path('/var/lib/vast-windows-test/inventory.json')
        cfg = vfio.prep.validate_inventory(json.loads(cfg_path.read_text()))
        args.qemu = 'qemu-system-x86_64'
        args.memory_mib = 8192
        args.cpus = 8
        args.ssh_forward_port = 22225
        args.ovmf_code = Path('/usr/share/OVMF/OVMF_CODE_4M.fd')
        if not args.ovmf_code.is_file():
            raise RuntimeError('Virtual OVMF code file unavailable')
        command = vfio.qemu_command(cfg, work, args)
        # Use current state after copying, never the pre-reboot recovery journal.
        before = vfio.inventory()
        vfio.require_profile(before['devices'], before['amd'])
        journal = work/'vfio-state.json'
        state = {'schema_version': 1, 'before': before, 'source_image': str(source),
                 'source_sha256': digest(source), 'source_virtual_vars_sha256': digest(old_vars),
                 'recipe_sha256': digest(Path(__file__)), 'qemu_command': command}
        vfio.mark(journal, state, 'clone_verified')
        for sig in (signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
        child = None
        try:
            vfio.detach(journal, state)
            with (work/'qemu.log').open('ab', buffering=0) as log:
                child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log,
                                         stderr=subprocess.STDOUT, start_new_session=True)
            state['qemu_pid'] = child.pid
            state['qemu_start_token'] = vfio.process_token(child.pid)
            vfio.mark(journal, state, 'windows_running')
            vfio.wait_vm(child, work)
            if child.returncode:
                raise RuntimeError(f'QEMU exited {child.returncode}')
        finally:
            if child and child.poll() is None:
                try:
                    vfio.prep.rpc(work/'qmp.sock', 'system_powerdown', qmp=True)
                except (OSError, EOFError, RuntimeError):
                    pass
                if not vfio.wait_vm(child, work, timeout=120):
                    vfio.mark(journal, state, 'qemu_still_running_recovery_deferred')
                    raise RuntimeError('Windows did not stop; retain PCI ownership until QEMU exits')
            vfio.restore(journal, state)


if __name__ == '__main__':
    main()
