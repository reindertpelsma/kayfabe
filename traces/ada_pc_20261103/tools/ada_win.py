#!/usr/bin/env python3
"""Ada PC Windows regression controller (trusted host), 2026-11-03.

Modelled on /data/hp1-20261025/controller.py (the recorded owner-host Win11 recipe): a fresh CoW
overlay on the read-only /data/hp1-20261025/baseline3/windows.qcow2 (owner's Win11 Pro 26200 apps
image chain; the owner overlay is never opened for writing), the owner's production argv/flags of
run 4501 with only the binary, the run paths and the broker socket changed, a Wayland broker window,
the loopback PowerShell agent (guest task polls 10.0.2.2:8765) and a loopback file server (8766).

usage: ada_win.py <arm> <rev8> <deadline_s>
Stops on: new host Xid, OOM, QEMU exit, deadline, or <A>/finish.flag. Own PIDs only.
"""
import datetime as dt, json, os, shutil, signal, subprocess, sys, time
from pathlib import Path

D = Path('/data/claude-ada-20261103')
ARM, REV, LIMIT = sys.argv[1], sys.argv[2], int(sys.argv[3])
A = D / ('win-' + ARM)
W = Path('/var/lib/kf-windows-20261005')
EXE = W / 'kf3-bins' / REV / 'qemu-system-x86_64'
BASE = Path('/data/hp1-20261025/baseline3/windows.qcow2')
VARS = Path('/data/hp1-20261025/baseline3/OVMF_VARS.fd')
REF = Path('/var/lib/kf-windows-20261005/boundary-kayfabe-4501/command.json')
BROKER = Path('/data/claude-spec-20261024/lane/src/broker/nvkvm-display-broker')
QMP = W / 'boundary-tools' / 'qmp.py'


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def write(name, obj):
    (A / name).write_text(json.dumps(obj, indent=2) + '\n')


def xids():
    out = subprocess.run(['dmesg'], capture_output=True, text=True).stdout
    return [l for l in out.splitlines() if 'NVRM: Xid' in l or 'Out of memory' in l or 'oom-kill' in l
            or 'GPU has fallen' in l]


def qmp(cmd, args=None):
    a = ['python3', str(QMP), str(A / 'qmp.sock'), 'cmd', cmd]
    if args is not None:
        a.append(json.dumps(args))
    return subprocess.run(a, capture_output=True, text=True, timeout=20)


def threads(pid):
    rows = {}
    for t in Path(f'/proc/{pid}/task').iterdir():
        try:
            comm = (t / 'comm').read_text().strip()
            f = (t / 'stat').read_text().rsplit(')', 1)[1].split()
            rows[t.name] = (comm, int(f[11]) + int(f[12]))
        except OSError:
            pass
    return rows


assert not A.exists(), f'{A} exists (a run directory is never reused)'
assert EXE.is_file(), f'no binary {EXE}'
running = subprocess.run(['pgrep', '-x', 'qemu-system-x86'], capture_output=True, text=True).stdout.split()
assert not running, f'QEMU already running: {running} (never touch the owner VM)'
avail = int([l for l in open('/proc/meminfo') if l.startswith('MemAvailable')][0].split()[1]) // 1048576
assert avail >= 16, f'MemAvailable {avail} GiB < 16'
A.mkdir(mode=0o775)
os.chown(A, 0, 1000)
os.chmod(A, 0o775)
before = {'path': str(BASE), 'size': BASE.stat().st_size, 'mtime_ns': BASE.stat().st_mtime_ns}
write('backing-before.json', before)
subprocess.run(['qemu-img', 'create', '-q', '-f', 'qcow2', '-F', 'qcow2', '-b', str(BASE),
                str(A / 'windows.qcow2')], check=True)
shutil.copyfile(VARS, A / 'OVMF_VARS.fd')
ref = json.loads(REF.read_text())
old = '/var/lib/kf-windows-20261005/boundary-kayfabe-4501'
argv = [s.replace(old, str(A)).replace('boundary-kayfabe-4501', 'ada-' + ARM)
        .replace('/run/user/1000/nvkvm/windows-run4501.sock', str(A / 'broker.sock')) for s in ref['argv']]
argv[0] = str(EXE)
flags = dict(ref['flags'])
flags.update({k: v for k, v in (e.split('=', 1) for e in os.environ.get('ADA_EXTRA_FLAGS', '').split()) })
env = {k: v for k, v in os.environ.items() if not k.startswith('KF3_')}
env.update(flags)
xid0 = xids()
write('command.json', {'revision': REV, 'argv': argv, 'flags': flags, 'reference': str(REF),
                       'binary_sha256': subprocess.run(['sha256sum', str(EXE)], capture_output=True,
                                                       text=True).stdout.split()[0],
                       'broker_sha256': subprocess.run(['sha256sum', str(BROKER)], capture_output=True,
                                                       text=True).stdout.split()[0],
                       'host_driver': subprocess.run(['nvidia-smi', '--query-gpu=driver_version',
                                                      '--format=csv,noheader'], capture_output=True,
                                                     text=True).stdout.strip(),
                       'xid_lines_before': len(xid0), 'mem_available_gib': avail, 'time': now()})
procs = {}
qemu = None
reason = None
try:
    procs['broker'] = subprocess.Popen(
        ['runuser', '-u', 'ubuntu', '--', 'env', 'XDG_RUNTIME_DIR=/run/user/1000', 'WAYLAND_DISPLAY=wayland-0',
         'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus', str(BROKER), '--socket', str(A / 'broker.sock'),
         '--backend', 'wayland', '--persist', '--stats-line', '--title', 'ADA ' + ARM],
        stdout=(A / 'broker.log').open('w'), stderr=subprocess.STDOUT, start_new_session=True)
    for _ in range(50):
        if (A / 'broker.sock').exists():
            break
        assert procs['broker'].poll() is None, 'BROKER_EXIT'
        time.sleep(.2)
    assert (A / 'broker.sock').exists(), 'BROKER_SOCKET_MISSING'
    procs['agent'] = subprocess.Popen(['python3', '-u', str(D / 'agent.py'), str(A / 'agent')],
                                      stdout=(A / 'agent.log').open('w'), stderr=subprocess.STDOUT,
                                      start_new_session=True)
    procs['files'] = subprocess.Popen(['python3', '-m', 'http.server', '8766', '--bind', '127.0.0.1',
                                       '--directory', str(D / 'files')],
                                      stdout=(A / 'files.log').open('w'), stderr=subprocess.STDOUT,
                                      start_new_session=True)
    time.sleep(.5)
    assert procs['agent'].poll() is None, 'AGENT_PORT_IN_USE'
    assert procs['files'].poll() is None, 'FILES_PORT_IN_USE'
    qemu = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL, stdout=(A / 'qemu.log').open('w'),
                            stderr=subprocess.STDOUT, start_new_session=True)
    write('owned.json', {'controller': os.getpid(), 'qemu': qemu.pid,
                         **{k: p.pid for k, p in procs.items()}, 'time': now()})
    print('ADA_LAUNCHED', ARM, qemu.pid, now(), flush=True)
    t0 = time.time()
    last_cpu = 0.0
    cpu = (A / 'cpu.jsonl').open('w')
    while reason is None:
        x = xids()
        if len(x) > len(xid0):
            write('new-xids.json', x[len(xid0):])
            reason = 'NEW_HOST_XID'
        elif qemu.poll() is not None:
            reason = f'QEMU_EXIT rc={qemu.returncode}'
        elif time.time() - t0 > LIMIT:
            reason = 'DEADLINE'
        elif (A / 'finish.flag').exists():
            reason = 'FINISH ' + (A / 'finish.flag').read_text().strip()
        if reason is None and time.time() - last_cpu >= 10:
            last_cpu = time.time()
            try:
                cpu.write(json.dumps({'t': round(time.time() - t0, 1), 'th': threads(qemu.pid)}) + '\n')
                cpu.flush()
            except OSError:
                pass
        write('monitor.json', {'time': now(), 'reason': reason, 'qemu_pid': qemu.pid,
                               'elapsed_s': int(time.time() - t0)})
        time.sleep(1)
finally:
    write('stop.json', {'reason': reason, 'time': now()})
    print('ADA_STOP', reason, now(), flush=True)
    if qemu is not None and qemu.poll() is None:
        how = None
        # 1. guest shutdown through the agent (QMP powerdown alone has not stopped this image before)
        if procs.get('agent') and procs['agent'].poll() is None:
            subprocess.run(['python3', str(D / 'agent.py'), str(A / 'agent'), 'run',
                            'Stop-Computer -Force', '20'], capture_output=True, timeout=40)
            try:
                qemu.wait(timeout=240); how = 'agent-shutdown'
            except subprocess.TimeoutExpired:
                pass
        if how is None:
            qmp('system_powerdown')
            try:
                qemu.wait(timeout=180); how = 'qmp-powerdown'
            except subprocess.TimeoutExpired:
                pass
        if how is None:
            qemu.send_signal(signal.SIGTERM)  # own PID only, recorded in owned.json
            try:
                qemu.wait(timeout=60); how = 'SIGTERM-own-pid'
            except subprocess.TimeoutExpired:
                how = 'STILL-RUNNING'
        write('powerdown.json', {'how': how, 'rc': qemu.returncode, 'time': now()})
    for p in procs.values():
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGTERM)
            try:
                p.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
    write('backing-after.json', {'unchanged': BASE.stat().st_size == before['size']
                                 and BASE.stat().st_mtime_ns == before['mtime_ns']})
    write('xid-after.json', {'xid_lines_before': len(xid0), 'xid_lines_after': len(xids()), 'time': now()})
    print('ADA_EXIT', ARM, now(), flush=True)
