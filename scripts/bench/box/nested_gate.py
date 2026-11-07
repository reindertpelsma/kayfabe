#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Nested feasibility stage, never a compatibility claim. Stdlib, no guest builds.

Host mode streams a prepared artifact set over SSH into a private temporary
outer-guest directory. Inner mode probes and boots one thin guest with one
raw-client --ce-client selection (three internal controls; no fault injection).
All raw output stays in logs; only this harness authors NESTED_ROW/verdicts.
"""
import hashlib
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid

CHECK_PASS = 'done — raw CE client only (ALL ARMS MET)'
CHECK_FAIL = 'done — raw CE client only (WITH FAILED EVIDENCE)'
MAX_LOG = 16 * 1024 * 1024


class Retry(Exception):
    pass


def emit(name, result, start):
    print(f'NESTED_ROW name={name} result={result} secs={time.monotonic()-start:.3f}', flush=True)


def finish(kind, cls=None):
    rc = {'PASS': 0, 'FAIL': 1, 'RETRY': 3}[kind]
    print('NESTED_' + kind + (' class=' + cls if cls else ''), flush=True)
    print(f'NESTED_EXIT rc={rc}', flush=True)
    return rc


def choose_accel(probe):
    if probe.get('kvm_start') == 'yes':
        return 'kvm'
    if probe.get('tcg') == 'yes':
        return 'tcg'
    raise Retry('ACCEL')


def check_serial(text, rc):
    ready = re.findall(r'^FASTGUEST: ready [0-9.]+s$', text, re.M)
    clients = re.findall(r'^FASTGUEST: client rc=([0-9]+) at [0-9.]+s$', text, re.M)
    done = re.findall(r'^FASTGUEST: DONE$', text, re.M)
    checks = [line for line in text.splitlines() if line in (CHECK_PASS, CHECK_FAIL)]
    if rc != 0 or len(ready) != 1 or len(clients) != 1 or len(done) != 1 or len(checks) != 1:
        return 'RETRY'
    positions = [text.index(ready[0]), text.index(checks[0]), text.index('FASTGUEST: client rc='), text.index('FASTGUEST: DONE')]
    if positions != sorted(positions):
        return 'RETRY'
    if clients != ['0'] or checks != [CHECK_PASS]:
        return 'FAIL'
    return 'PASS'


def read_log(path):
    with path.open('rb') as f:
        data = f.read(MAX_LOG + 1)
    if len(data) > MAX_LOG:
        raise Retry('LOG')
    return data.decode('utf-8', 'replace').replace('\r', '')


def bounded(command, seconds, progress=None, log_paths=(), **kwargs):
    """Own the process only; TERM and reap before grading. Never SIGKILL a GPU VM."""
    if seconds <= 0:
        raise Retry('TIME')
    def check_logs():
        if any(p.exists() and p.stat().st_size > MAX_LOG for p in log_paths):
            raise Retry('LOG')
    with subprocess.Popen(command, start_new_session=True, **kwargs) as child:
        deadline = time.monotonic()+seconds
        try:
            while True:
                check_logs()
                if progress:
                    progress()
                try:
                    rc = child.wait(timeout=min(0.2, max(0, deadline-time.monotonic())))
                    check_logs()
                    return rc
                except subprocess.TimeoutExpired:
                    if time.monotonic() >= deadline:
                        raise
        except (subprocess.TimeoutExpired, KeyboardInterrupt, Retry) as fault:
            # A second termination request cannot skip ownership/reaping.
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            try:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            # Cleanup is outside the execution budget, as in the fastguest lane.
            # Keep waiting for our VM: never release ownership over a live GPU VM.
            reason = str(fault) if isinstance(fault, Retry) else 'TIME'
            print('NESTED_CLEANUP reason='+reason+' waiting=1', flush=True)
            child.wait()
            raise Retry(reason) from None
        finally:
            if progress:
                progress()


def inner(root, rev, kvm_budget, tcg_budget, deadline):
    started = time.monotonic()
    qemu = root/'kf3-bins'/rev/'qemu-system-x86_64'
    fg = root/'fastguest'
    for f in (qemu, fg/'vmlinuz', fg/'initrd.cpio.gz', fg/'driver.STAGED'):
        if not f.is_file():
            raise Retry('NOT_BUILT')
    versions = re.findall(r'^version=([0-9]+\.[0-9]+(?:\.[0-9]+)?)$', (fg/'driver.STAGED').read_text(), re.M)
    if len(versions) != 1:
        raise Retry('NOT_BUILT')
    qemu.chmod(qemu.stat().st_mode | 0o100)
    digest = hashlib.sha256()
    with qemu.open('rb') as binary:
        for block in iter(lambda: binary.read(1 << 20), b''):
            digest.update(block)
    print(f'NESTED_BINS rev={rev} qemu_sha256={digest.hexdigest()}', flush=True)
    emit('install', 'PASS', started)
    probe_start = time.monotonic()
    with (root/'probe.log').open('w') as out:
        rc = bounded(['bash', str(root/'nested_probe.sh')], min(15, deadline-time.monotonic()),
                     stdout=out, stderr=subprocess.STDOUT, env=dict(os.environ, QEMU_BIN=str(qemu)))
    text = read_log(root/'probe.log')
    print(text, end='', flush=True)
    pairs = re.findall(r'^NESTED_PROBE ([a-z_]+)=(yes|no|unknown)$', text, re.M)
    expected = {'kvm_device', 'kvm_intel_nested', 'kvm_amd_nested', 'cpu_vmx', 'cpu_svm', 'kvm_start', 'tcg'}
    if rc or len(pairs) != len(expected) or {k for k, _ in pairs} != expected:
        raise Retry('PROBE')
    accel = choose_accel(dict(pairs))
    emit('probe', 'PASS', probe_start)
    budget = kvm_budget if accel == 'kvm' else tcg_budget
    if deadline-time.monotonic() < budget:
        raise Retry('TIME')
    print(f'NESTED_START accel={accel} budget_s={budget} arm=ce_client', flush=True)
    serial = root/'serial.log'
    command = [str(qemu), '-name', 'kf-nested', '-uuid', str(uuid.uuid4()),
               '-object', 'memory-backend-memfd,id=ram0,size=1024M,share=on',
               '-machine', f'q35,accel={accel},memory-backend=ram0', '-m', '1024',
               '-cpu', 'host' if accel == 'kvm' else 'max', '-smp', '1',
               '-kernel', str(fg/'vmlinuz'), '-initrd', str(fg/'initrd.cpio.gz'),
               '-append', f'console=ttyS0 panic=1 loglevel=6 KF_ARMS=--ce-client KF_IOCTL_TRACE=ring KF_BUDGET_S={budget}',
               '-device', 'kf3-gpu,fb-mb=1024,bar1-size=134217728,bar2-size=33554432,id=kf0,guest-driver='+versions[0],
               '-serial', 'file:'+str(serial), '-display', 'none', '-monitor', 'none', '-no-reboot']
    boot_start = time.monotonic()
    boot_reported = False
    raw_start = boot_start
    serial_position, pending = 0, ''
    def boot_progress():
        nonlocal boot_reported, serial_position, pending, raw_start
        if boot_reported or not serial.exists():
            return
        with serial.open('rb') as log:
            log.seek(serial_position)
            chunk = log.read(65536)
            serial_position += len(chunk)
        pending = (pending+chunk.decode('utf-8', 'replace')).replace('\r', '')
        if re.search(r'^FASTGUEST: ready [0-9.]+s$', pending, re.M):
            raw_start = time.monotonic()
            emit('boot', 'PASS', boot_start)
            boot_reported = True
        pending = pending[-8192:]
    with (root/'qemu.log').open('w') as out:
        rc = bounded(command, budget, stdout=out, stderr=subprocess.STDOUT,
                     progress=boot_progress, log_paths=(root/'qemu.log', serial))
    text = read_log(serial) if serial.exists() else ''
    booted = len(re.findall(r'^FASTGUEST: ready [0-9.]+s$', text, re.M)) == 1
    if not boot_reported:
        emit('boot', 'PASS' if booted else 'NOT_RUN', boot_start)
    verdict = check_serial(text, rc)
    emit('raw_ce', verdict if verdict != 'RETRY' else 'NOT_RUN', raw_start)
    emit('cleanup', 'PASS', time.monotonic())  # owned QEMU reaped before this row
    if verdict == 'RETRY':
        raise Retry('BOX')
    return verdict


def host(tag, prov, rev):
    if not re.fullmatch(r'[A-Za-z0-9_]{1,48}', tag) or not re.fullmatch(r'[0-9a-f]{40}', rev):
        raise Retry('USAGE')
    bench = Path(os.environ.get('BENCH_DIR', '/workspace/bench'))
    bins = Path(os.environ.get('KF_NESTED_BINS', str(bench/'kf3-bins'/rev[:8])))
    fg = Path(os.environ.get('KF_FASTGUEST_DIR', str(bench/'fastguest')))
    files = [(bins/'qemu-system-x86_64', 'kf3-bins/'+rev+'/qemu-system-x86_64'),
             *((fg/name, 'fastguest/'+name) for name in ('vmlinuz', 'initrd.cpio.gz', 'driver.STAGED')),
             (Path(__file__), 'nested_gate.py'), (Path(__file__).with_name('nested_probe.sh'), 'nested_probe.sh')]
    # No directory copying, no source builds, no symlink traversal, no SSH agent forwarding.
    if any(not p.is_file() or p.is_symlink() for p, _ in files):
        raise Retry('NOT_BUILT')
    started = time.monotonic()
    budget = int(os.environ.get('KF_NESTED_BUDGET_S', '2400'))
    kvm = int(os.environ.get('KF_NESTED_KVM_S', '600'))
    tcg = int(os.environ.get('KF_NESTED_TCG_S', '1800'))
    if not 1 <= kvm < tcg < budget <= 7200:
        raise Retry('USAGE')
    target = os.environ.get('KF_NESTED_SSH_HOST', 'ubuntu@127.0.0.1')
    port = os.environ.get('KF_NESTED_SSH_PORT', '2222')
    if not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.@-]{0,127}', target) or not port.isdecimal() or not 1 <= int(port) <= 65535:
        raise Retry('USAGE')
    key = os.environ.get('KF_NESTED_SSH_KEY', str(bench/'guest_key'))
    ssh = ['ssh', '-a', '-x', '-o', 'ForwardAgent=no', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10',
           '-o', 'ServerAliveInterval=15', '-o', 'ServerAliveCountMax=2', '-o', 'StrictHostKeyChecking=no',
           '-o', 'UserKnownHostsFile=/dev/null', '-i', key, '-p', port, target]
    # All substituted tokens below are validated hex/integers; the outer endpoint is prepared,
    # isolated bench infrastructure. No remote root install is needed for the private payload.
    remote = ('set -eu; d=$(mktemp -d /tmp/kf-nested.XXXXXXXX); '
              'trap \'rm -rf -- "$d"\' EXIT; tar -xf - -C "$d"; '
              f'python3 "$d/nested_gate.py" --inner "$d" {rev} {kvm} {tcg} {int(time.time())+budget}')
    prov.mkdir(parents=True, exist_ok=True)
    path = prov/(tag+'_nested.log')
    def forward(stream):
        # Follow even partial writes without buffering an unbounded guest line.
        pending, discard = '', False
        def poll():
            nonlocal pending, discard
            chunk = stream.read(65536)
            for piece in chunk.splitlines(keepends=True):
                pending += piece
                if len(pending) > 8192:
                    pending, discard = '', True
                if piece.endswith('\n'):
                    if not discard and pending.startswith(('NESTED_ROW ', 'NESTED_PROBE ', 'NESTED_START ', 'NESTED_BINS ')):
                        print(pending, end='', flush=True)
                    pending, discard = '', False
        return poll

    with tempfile.TemporaryFile() as package:
        with tarfile.open(fileobj=package, mode='w') as tar:
            for p, name in files:
                tar.add(p, arcname=name, recursive=False)
        package.seek(0)
        with path.open('w') as out, path.open(errors='replace') as stream:
            rc = bounded(ssh+['bash -c '+shlex.quote(remote)], budget+60-(time.monotonic()-started),
                         stdin=package, stdout=out, stderr=subprocess.STDOUT, progress=forward(stream), log_paths=(path,))
    # The payload grader independently verifies rows/verdict. Keep untrusted remote
    # output intact for evidence, but never turn a failed SSH exit into PASS.
    text = read_log(path)
    if rc not in (0, 1, 3):
        raise Retry('SSH')
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'ci'))
    from vast_payload import nested_rows, EvidenceError
    try:
        nested_rows(text, rc)
    except EvidenceError as e:
        if e.name == 'nested':
            kind, cls = 'FAIL', 'CHECK'
        else:
            kind, cls = 'RETRY', e.name.removeprefix('nested_retry_').upper()
    else:
        kind, cls = 'PASS', None
    return kind, cls


def main(args):
    def stop(*_):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, stop)
    try:
        if len(args) == 6 and args[0] == '--inner':
            _, directory, rev, kvm, tcg, budget = args
            root = Path(directory)
            try:
                kind = inner(root, rev, int(kvm), int(tcg), time.monotonic()+int(budget)-time.time())
            finally:
                # Bounded diagnostic tails survive deletion of the private installation.
                # Prefix every guest line so it cannot author result rows or verdicts.
                for name in ('probe.log', 'qemu.log', 'serial.log'):
                    path = root/name
                    if path.is_file():
                        with path.open('rb') as log:
                            log.seek(max(0, path.stat().st_size-65536))
                            tail = log.read(65536).decode('utf-8', 'replace')
                        for line in tail.splitlines()[-100:]:
                            print('NESTED_DIAG '+name+' '+ascii(line[:1000]), flush=True)
            return finish(kind, 'CHECK' if kind == 'FAIL' else None)
        if len(args) != 3:
            raise Retry('USAGE')
        kind, cls = host(args[0], Path(args[1]), args[2])
        return finish(kind, cls)
    except KeyboardInterrupt:
        return finish('RETRY', 'TIME')
    except Retry as error:
        return finish('RETRY', str(error))
    except (OSError, ValueError, subprocess.SubprocessError):
        return finish('RETRY', 'OPERATOR')


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
