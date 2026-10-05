#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Matched fresh-overlay Windows boundary runs on the borrowed RTX4070 PC.

Research only. Both arms use the same baseline and immutable QEMU. VFIO uses
the previously audited exact-host detach/restore helper; no physical firmware
changes. MMIO logging is diagnostic, not a complete GSP/DMA capture.
"""
import argparse
import datetime
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time

BASE = Path('/var/lib/kf-windows-20261005')
REVISION = 'b431aeaf9fca5d78b451754c0de7b8dbbe9d7652'
FLAGS = ('KF3_GFX_POOL_PROBE', 'KF3_TIMER_MAP', 'KF3_TSPACE',
         'KF3_SW_RUNLIST_PROBE', 'KF3_MEMORY_LIST_PROBE',
         'KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE')
ILUT_FLAG = 'KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE'


def digest(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def stop_child(child):
    """Do not permit PCI restoration until QEMU is positively reaped."""
    if child is None or child.poll() is not None:
        return
    child.terminate()
    try:
        child.wait(timeout=90)
    except subprocess.TimeoutExpired:
        child.kill()
        # Failure here deliberately prevents restore; never rebind a live VM.
        child.wait(timeout=30)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--arm', choices=('vfio', 'kayfabe'), required=True)
    p.add_argument('--run', type=int, choices=range(1, 100), required=True)
    p.add_argument('--max-seconds', type=int, default=600)
    p.add_argument('--no-mmio-trace', action='store_true')
    p.add_argument('--product-revision', default=REVISION)
    p.add_argument('--qemu-revision', help='Immutable artifact revision; defaults to product revision')
    p.add_argument('--gsp-observer', action='store_true')
    p.add_argument('--ilut-probe', action='store_true')
    a = p.parse_args()
    if os.geteuid() or not 60 <= a.max_seconds <= 1800:
        p.error('Require root and a 60..1800 second runtime bound')
    artifact = a.qemu_revision or a.product_revision
    if not all(re.fullmatch('[0-9a-f]{40}', x) for x in (artifact, a.product_revision)):
        p.error('Require full source revisions for product and QEMU artifact')
    if a.gsp_observer and (a.arm != 'vfio' or not a.no_mmio_trace):
        p.error('Narrow GSP observer requires VFIO without generic MMIO tracing')
    if a.ilut_probe and a.arm != 'kayfabe':
        p.error('ILUT construction probe is only a Kayfabe arm')
    os.umask(0o077)
    name = f'boundary-{a.arm}-{a.run}'
    work = BASE/name
    qemu = BASE/'kf3-bins'/artifact[:8]/'qemu-system-x86_64'
    baseline = BASE/'baseline/windows.qcow2'
    template = json.loads((BASE/'probe-l/command.json').read_text())
    if template['revision'] != REVISION:
        raise RuntimeError('Pinned command template changed')
    with open('/tmp/kayfabe-fastguest.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if work.exists() or not qemu.is_file() or not baseline.is_file():
            raise RuntimeError('Fresh run directory and pinned fixtures required')
        work.mkdir(mode=0o700)
        subprocess.run(['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2',
                        '-b', str(baseline), str(work/'windows.qcow2')], check=True)
        shutil.copyfile(BASE/'baseline/OVMF_VARS.fd', work/'OVMF_VARS.fd')
        old = str(BASE/'probe-l')
        cmd = [s.replace(old, str(work)).replace('kayfabe-windows-probe-l', name)
               for s in template['argv']]
        if cmd[0] != str(BASE/'kf3-bins'/REVISION[:8]/'qemu-system-x86_64') or not cmd[-1].startswith('kf3-gpu,'):
            raise RuntimeError('Unexpected pinned QEMU command')
        cmd[0] = str(qemu)
        vfio = None
        state = None
        journal = work/'vfio-state.json'
        if a.arm == 'vfio':
            helper = Path('/root/vast-windows-test/prepare/vfio_4070.py')
            pins = {helper: '79262fcbfbc7bab06c0cdc027d6bccc89b22864331ce6075bb1e27c990d47c67',
                    helper.with_name('prepare.py'): '5013d628ef4d63e07d7952666d4ce2c0723186c6d41559c203d488c29918cf73'}
            for path, expected in pins.items():
                if digest(path) != expected:
                    raise RuntimeError(f'Pinned helper changed: {path}')
            spec = importlib.util.spec_from_file_location('vfio4070', helper)
            vfio = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(vfio)
            write_sysfs = vfio.write_sysfs
            def console_aware_write(path, value):
                try:
                    write_sysfs(path, value)
                except FileNotFoundError:
                    node = Path(path)
                    # DRM may unregister this console while display services
                    # stop. Its absence needs no unbind/rebind. All PCI writes
                    # and any other error retain the helper's refusal behavior.
                    if node.name != 'bind' or node.parent.parent != Path('/sys/class/vtconsole') or node.exists():
                        raise
                    print('VANISHED_VTCONSOLE', path, flush=True)
            vfio.write_sysfs = console_aware_write
            before = vfio.inventory()
            vfio.require_profile(before['devices'], before['amd'])
            suffix = '' if a.no_mmio_trace else ',x-no-mmap=on'
            if a.gsp_observer:
                suffix = (f',x-gsp-observer={work}/gsp.jsonl,x-no-kvm-intx=on,'
                          'x-no-kvm-msi=on,x-no-kvm-msix=on,x-no-kvm-ioeventfd=on,'
                          'x-no-vfio-ioeventfd=on,enable-migration=off')
            cmd[-1] = 'vfio-pci,host=0000:01:00.0,bus=pci.0,addr=0x6.0,multifunction=on'+suffix
            cmd += ['-device', 'vfio-pci,host=0000:01:00.1,bus=pci.0,addr=0x6.1']
            if not a.no_mmio_trace:
                # Polling reads exhausted the first pilot's 256MiB bound before
                # collection. Capture writes/config plus a separate caps snapshot.
                (work/'trace-events').write_text('vfio_region_write\nvfio_pci_read_config\nvfio_pci_write_config\n')
                cmd += ['-trace', f'events={work}/trace-events,file={work}/mmio.log']
            state = {'schema_version': 1, 'before': before, 'qemu_command': cmd}
        env = dict(os.environ)
        for flag in (*FLAGS, ILUT_FLAG):
            env.pop(flag, None)
        env['KF3_RPC_TRACE'] = '1'
        if a.arm == 'kayfabe':
            env.update({flag: '1' for flag in FLAGS})
            if a.ilut_probe:
                env[ILUT_FLAG] = '1'
        meta = dict(schema=1, arm=a.arm, run=a.run, revision=a.product_revision,
                    qemu_artifact_revision=artifact, runner_sha256=digest(Path(__file__)),
                    gsp_observer=a.gsp_observer,
                    argv=cmd, baseline=str(baseline), baseline_sha256=digest(baseline),
                    qemu_sha256=digest(qemu), firmware_sha256=digest(work/'OVMF_VARS.fd'),
                    flags={flag: env.get(flag) for flag in (*FLAGS, ILUT_FLAG)},
                    mmio_trace=a.arm == 'vfio' and not a.no_mmio_trace,
                    mmio_read_coverage='not traced; capability page snapshot only',
                    time_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
        (work/'command.json').write_text(json.dumps(meta, indent=2)+'\n')
        child = None
        def interrupted(*_):
            raise KeyboardInterrupt()
        for sig in (signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, interrupted)
        print('BOUNDARY_START', name, flush=True)
        try:
            if vfio:
                vfio.detach(journal, state)
            with (work/'qemu.log').open('wb', buffering=0) as out:
                child = subprocess.Popen(cmd, env=env, stdin=subprocess.DEVNULL,
                                         stdout=out, stderr=subprocess.STDOUT)
            if vfio:
                state['qemu_pid'] = child.pid
                state['qemu_start_token'] = vfio.process_token(child.pid)
                vfio.mark(journal, state, 'windows_running')
            deadline = time.monotonic()+a.max_seconds
            while child.poll() is None:
                oversized = any(path.exists() and path.stat().st_size > 256*1024**2
                                for path in (work/'mmio.log', work/'qemu.log', work/'serial.log', work/'gsp.jsonl'))
                if time.monotonic() >= deadline or oversized:
                    raise RuntimeError('Diagnostic runtime or trace-size bound reached')
                time.sleep(1)
            if child.returncode:
                raise RuntimeError(f'QEMU exited {child.returncode}')
        finally:
            # A controller stop can arrive while a size-limit failure already
            # unwinds. A second signal must not interrupt ownership restoration.
            for sig in (signal.SIGTERM, signal.SIGHUP):
                signal.signal(sig, signal.SIG_IGN)
            try:
                stop_child(child)
            except Exception as exc:
                (work/'manual-recovery-required.txt').write_text(
                    'QEMU exit unconfirmed; PCI restoration intentionally refused.\n'+repr(exc)+'\n')
                print('BOUNDARY_EXIT_UNCONFIRMED', name, flush=True)
                raise
            if vfio:
                vfio.restore(journal, state)
            print('BOUNDARY_EXIT', name, None if child is None else child.returncode, flush=True)


if __name__ == '__main__':
    main()
