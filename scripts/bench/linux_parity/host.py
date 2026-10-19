#!/usr/bin/env python3
"""LX1 host-only launch/restore/sampling. Does not change kayfabe product code.

Invoke hardware commands only under the lease documented in RUNBOOK.md.
Configuration and logs stay in the lane directory; plan is GPU-free.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path('/data/codex-LX1')
LOCK = Path('/tmp/kayfabe-fastguest.lock')


def starttime(stat):
    return stat.rsplit(')', 1)[1].split()[19]


def proc_start(pid):
    return starttime(Path(f'/proc/{pid}/stat').read_text())


def idle_guard(qemus, brokers, clients):
    if qemus or brokers or clients:
        raise RuntimeError(f'not idle: qemu={qemus} broker={brokers} clients={clients}')


def group_guard(targets, actual, amd_desktop):
    if set(targets) != set(actual) or not amd_desktop:
        raise RuntimeError('IOMMU group mismatch or AMD desktop not verified')


def restore_guard(live_vm, original):
    if live_vm or not original or any(x not in {'nvidia', 'snd_hda_intel'} for x in original.values()):
        raise RuntimeError('restore requires exited QEMU and recorded original drivers')


def lease_guard(record, pid, current_start, holders):
    if record['pid'] != pid or record['start'] != current_start or pid not in holders:
        raise RuntimeError('missing/stale global flock lease')


def require_lease():
    record = json.loads((ROOT / 'lease.json').read_text())
    # Match lock inode, not merely a process named flock.
    ino = str(LOCK.stat().st_ino)
    holders = set()
    for line in Path('/proc/locks').read_text().splitlines():
        fields = line.split()
        if len(fields) >= 6 and fields[1] == 'FLOCK' and fields[3] == 'WRITE' and fields[5].rsplit(':', 1)[-1] == ino:
            holders.add(int(fields[4]))
    lease_guard(record, record['pid'], proc_start(record['pid']), holders)


def processes():
    qemus, brokers = [], []
    for p in Path('/proc').glob('[0-9]*'):
        try:
            comm = (p / 'comm').read_text().strip()
            if comm.startswith('qemu-system'):
                qemus.append(int(p.name))
            if comm.startswith('nvkvm-display-b'):
                brokers.append(int(p.name))
        except (OSError, ProcessLookupError):
            pass
    return qemus, brokers


def gpu_clients():
    nodes = [str(p) for pattern in ('/dev/nvidia[0-9]*', '/dev/nvidiactl', '/dev/nvidia-uvm') for p in Path('/').glob(pattern.lstrip('/'))]
    # fuser return 1 is the documented no-users result; other failures remain errors.
    if not nodes:
        return []
    r = subprocess.run(['fuser', *nodes], capture_output=True, text=True, timeout=10)
    if r.returncode not in (0, 1):
        raise RuntimeError('cannot inventory NVIDIA device users: ' + r.stderr)
    return r.stdout.split()


def plan(c, arm, inherited):
    root = Path(c['root'])
    disk = Path(c['disk'])
    if root != ROOT or not disk.is_relative_to(root) or '..' in disk.parts or not re.fullmatch(r'[A-Za-z0-9_-]+', c['run']):
        raise ValueError('configuration must use lane-local disk and run directory')
    if arm not in 'ABCDP' or len(arm) != 1:
        raise ValueError('unknown arm')
    if not re.fullmatch(r'\d+\.\d+\.\d+', c['driver']):
        raise ValueError('explicit exact driver required')
    env = {k: v for k, v in inherited.items() if not k.startswith('KF3_')}
    if arm in ('B', 'D'):
        env['KF3_TSG_MIRROR'] = '1' if arm == 'B' else '0'
    rev = '0d7ba743' if arm in ('B', 'D') else 'f8f9d1a5'
    binary = '/usr/bin/qemu-system-x86_64' if arm == 'P' else str(Path(c['bins']) / rev / 'qemu-system-x86_64')
    run = root / c['run']
    args = [binary, '-name', 'lx1-' + c['run'], '-nodefaults', '-no-user-config',
            '-object', 'memory-backend-memfd,id=ram0,size=12288M,share=on',
            '-machine', 'q35,accel=kvm,memory-backend=ram0', '-m', '12288', '-smp', '8',
            '-cpu', 'host,-vmx,migratable=off,+invtsc', '-uuid', '77cfb64d-651c-4f8a-b6ec-7d257a81fddc',
            '-drive', 'file=' + str(disk) + ',if=none,id=disk0,format=qcow2,cache=none,aio=threads',
            '-object', 'iothread,id=iod0', '-device', 'virtio-blk-pci,drive=disk0,iothread=iod0',
            '-netdev', 'user,id=n0,hostfwd=tcp:127.0.0.1:2229-:22',
            '-device', 'virtio-net-pci,netdev=n0,mac=52:54:00:19:10:19',
            '-display', 'none', '-serial', 'file:' + str(run / 'serial.log'),
            '-qmp', 'unix:' + str(run / 'qmp.sock') + ',server=on,wait=off',
            '-monitor', 'none', '-no-reboot', '-msg', 'timestamp=on']
    if c.get('seed'):
        seed = Path(c['seed'])
        if not seed.is_relative_to(root) or '..' in seed.parts:
            raise ValueError('cloud seed must be lane local')
        args += ['-drive', 'file=' + str(seed) + ',if=virtio,format=raw,readonly=on']
    if arm == 'C':
        for dev in c['pci']:
            if not re.fullmatch(r'0000:[0-9a-f]{2}:[0-9a-f]{2}\.[0-7]', dev):
                raise ValueError('invalid PCI address')
            args += ['-device', 'vfio-pci,host=' + dev]
    elif arm != 'P':
        args += ['-device', 'kf3-gpu,fb-mb=8192,bar1-size=8589934592,bar2-size=33554432,display=on,guest-driver=' + c['driver'] + ',id=kf0']
    return args, env


def driver(dev):
    p = Path('/sys/bus/pci/devices') / dev / 'driver'
    return p.resolve().name if p.exists() else None


def bind(dev, target):
    base = Path('/sys/bus/pci/devices') / dev
    current = driver(dev)
    if current == target:
        (base / 'driver_override').write_text('\n')
        return
    (base / 'driver_override').write_text(target)
    if current:
        (base / 'driver' / 'unbind').write_text(dev)
    Path('/sys/bus/pci/drivers_probe').write_text(dev)
    if driver(dev) != target:
        raise RuntimeError(f'binding failed: {dev} -> {target}')
    (base / 'driver_override').write_text('\n')


def restore(original):
    require_lease()
    qemus, _ = processes()
    restore_guard(bool(qemus), original)
    for dev, target in original.items():
        bind(dev, target)


def launch(c, arm):
    require_lease()
    qemus, brokers = processes()
    idle_guard(qemus, brokers, gpu_clients() if arm == 'C' else [])
    args, env = plan(c, arm, dict(os.environ))
    if not Path(args[0]).is_file() or not Path(c['disk']).is_file():
        raise RuntimeError('missing binary or disk')
    run = ROOT / c['run']
    run.mkdir(mode=0o700, exist_ok=False)
    original = {}
    if arm == 'C':
        actual = set()
        for dev in c['pci']:
            actual.update(x.name for x in (Path('/sys/bus/pci/devices') / dev / 'iommu_group/devices').iterdir())
        group_guard(c['pci'], actual, c.get('amd_desktop_verified') is True)
        original = {dev: driver(dev) for dev in c['pci']}
        restore_guard(False, original)
        (run / 'bindings.json').write_text(json.dumps(original))
        subprocess.run(['modprobe', 'vfio-pci'], check=True, timeout=15)
    (run / 'command.json').write_text(json.dumps(dict(argv=args, arm=arm, flags={k: v for k, v in env.items() if k.startswith('KF3_')}, started_ns=time.time_ns()), indent=2))
    try:
        for dev in original:
            bind(dev, 'vfio-pci')
        with (run / 'qemu.log').open('x') as log:
            p = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
            (run / 'pid.json').write_text(json.dumps(dict(pid=p.pid, start=proc_start(p.pid))))
            rc = p.wait()
        (run / 'exit.json').write_text(json.dumps(dict(rc=rc, end_ns=time.time_ns())))
    finally:
        if original:
            restore(original)
    return rc


def snapshot(pid=None):
    row = dict(realtime_ns=time.time_ns(), monotonic_ns=time.monotonic_ns(), pressure={}, vcpu=[])
    for kind in ('cpu', 'memory', 'io'):
        p = Path('/proc/pressure') / kind
        row['pressure'][kind] = p.read_text() if p.exists() else None
    fields = 'timestamp,clocks.sm,clocks.mem,pstate,power.draw,utilization.gpu,utilization.memory,pcie.link.gen.current,pcie.link.width.current,clocks_event_reasons.active'
    r = subprocess.run(['nvidia-smi', '--query-gpu=' + fields, '--format=csv,noheader,nounits'], capture_output=True, text=True, timeout=5)
    row['nvml'] = dict(rc=r.returncode, csv=r.stdout.strip(), error=r.stderr.strip())
    # nvidia-smi -q exposes PCIe throughput and individual throttle reason names.
    q = subprocess.run(['nvidia-smi', '-q'], capture_output=True, text=True, timeout=5)
    row['nvml_detail'] = dict(rc=q.returncode, text=q.stdout, error=q.stderr)
    if pid:
        row['qemu'] = dict(pid=pid, start=proc_start(pid))
        for task in sorted(Path(f'/proc/{pid}/task').iterdir()):
            try:
                name = (task / 'comm').read_text().strip()
                if '/KVM' in name or name.startswith('CPU '):
                    row['vcpu'].append(dict(tid=int(task.name), name=name, start=starttime((task / 'stat').read_text()), schedstat=(task / 'schedstat').read_text().strip()))
            except OSError:
                pass
    row['sample_end_ns'] = time.time_ns()
    return row


def lease():
    ROOT.mkdir(mode=0o700, exist_ok=True)
    parent = os.getppid()
    record = dict(pid=parent, start=proc_start(parent), guardian=os.getpid(), utc_ns=time.time_ns())
    path = ROOT / 'lease.json'
    with path.open('x') as f:
        json.dump(record, f)
    require_lease()
    print('LX1_LEASE_HELD', json.dumps(record), flush=True)
    while not (ROOT / 'RELEASE').exists():
        time.sleep(1)
    qemus, brokers = processes()
    idle_guard(qemus, brokers, [])
    path.unlink()
    (ROOT / 'RELEASE').unlink()
    print('LX1_LEASE_RELEASED', flush=True)


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('command', choices=['plan', 'run', 'restore', 'sample', 'lease'])
    p.add_argument('--config')
    p.add_argument('--arm', choices=list('ABCDP'))
    p.add_argument('--pid', type=int)
    p.add_argument('--seconds', type=int, default=600)
    a = p.parse_args()
    if a.command == 'lease':
        lease()
        return 0
    if a.command == 'restore':
        restore(json.loads(Path(a.config).read_text()))
        return 0
    if a.command == 'sample':
        require_lease()
        expected = proc_start(a.pid) if a.pid else None
        for _ in range(a.seconds):
            t = time.monotonic()
            if a.pid and proc_start(a.pid) != expected:
                raise RuntimeError('QEMU identity changed during sampler')
            print(json.dumps(snapshot(a.pid)), flush=True)
            time.sleep(max(0, 1 - (time.monotonic() - t)))
        return 0
    c = json.loads(Path(a.config).read_text())
    if a.command == 'plan':
        args, env = plan(c, a.arm, {})
        print(json.dumps(dict(argv=args, env=env), indent=2))
        return 0
    return launch(c, a.arm)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception as e:
        print('LX1_REFUSED:', str(e), file=sys.stderr, flush=True)
        sys.exit(2)
