#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Owner-approved native negative-control proof and frozen later-Xid guard.

Bench only. No product policy or global Xid31 exemption. Marker addresses and
channels come from this run's positive-control logs, never a per-die table.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

MAX_BYTES = 16 * 1024 * 1024


class GuardRefused(ValueError):
    """The native proof or later kernel snapshot is not accepted."""


def xid_lines(journal):
    return tuple(line for line in journal.splitlines() if "NVRM: Xid" in line)


def bounded(path):
    with Path(path).open("rb") as f:
        data = f.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise GuardRefused("kernel/proof byte bound exceeded")
    return data.decode("utf-8", "strict")


def channel_before(text, end):
    hits = re.findall(r"DOORBELL-STORE #[0-9]+ host_token=0x[0-9a-f]+ runlist=[0-9]+ chid=([0-9]+)", text[:end])
    if not hits:
        raise GuardRefused("source marker has no channel submission")
    return int(hits[-1])


def expected_faults(defer_log, missing_log):
    for line in ("DEFER_PRIME_FAULTS=3/3", "DEFER_RIGCTL=PASS", "DEFER_NEGCTL=PASS", "RUNG_defer_liveness=PASS"):
        if line not in defer_log.splitlines():
            raise GuardRefused("native defer positive-control proof missing")
    for line in ("RUNGCTL_missing_page=PASS", "RUNG_missing_page=PASS"):
        if line not in missing_log.splitlines():
            raise GuardRefused("native missing-page positive-control proof missing")
    result = []
    markers = list(re.finditer(r"^FAULT_MARK=defer_liveness_prime arm=(\S+) va=(0x[0-9a-f]+)", defer_log, re.M))
    if len(markers) != 3:
        raise GuardRefused("expected three source prime markers")
    for marker in markers:
        tail = defer_log[marker.end():].split("═══ DL ARM", 1)[0]
        prime = re.search(r"info\s+DL prime copy\s*= Dead", tail)
        test = re.search(r"info\s+DL test copy\s*= (Live|Dead)", tail)
        if prime is None or test is None:
            raise GuardRefused("prime/test outcome absent")
        va = int(marker[2], 16)
        # Exact approved native fault shapes: prime/test CE reads and the GR
        # missing-page release below. These are test operations, not die facts.
        result.append((31, channel_before(tail, prime.start()), va, "FAULT_PTE", "ACCESS_TYPE_VIRT_READ", "CE0", "HUBCLIENT_CE1", "kayfabe-rm-ladd"))
        if test[1] == "Dead":
            if marker[1] != "DEFER(the-question)" or "DL rescue copy     = Live" not in tail:
                raise GuardRefused("unrecovered/unexpected test-channel fault")
            result.append((31, channel_before(tail, test.start()), va, "FAULT_PTE", "ACCESS_TYPE_VIRT_READ", "CE0", "HUBCLIENT_CE1", "kayfabe-rm-ladd"))
    markers = list(re.finditer(r"^FAULT_MARK=victim_release va=(0x[0-9a-f]+)", missing_log, re.M))
    if len(markers) != 1 or "bystander before = Landed" not in missing_log or "bystander after  = Landed" not in missing_log:
        raise GuardRefused("missing-page marker/bystander proof absent")
    tail = missing_log[markers[0].end():]
    victim = re.search(r"info\s+R3 victim\s*= Lost", tail)
    if victim is None:
        raise GuardRefused("missing-page victim did not fault")
    result.append((31, channel_before(tail, victim.start()), int(markers[0][1], 16), "FAULT_PDE", "ACCESS_TYPE_VIRT_WRITE", "GR0_PBDMA0", "HUBCLIENT_ESC", "kayfabe-rm-ladd"))
    return result


def approve_native(journal, defer_log, missing_log):
    expected = expected_faults(defer_log, missing_log)
    if len(expected) != 5:
        raise GuardRefused('approved native baseline requires exactly five source faults')
    actual = []
    lines = xid_lines(journal)
    pattern = re.compile(r"NVRM: Xid \(PCI:[^)]+\): (?P<code>[0-9]+), pid=[0-9]+, name=(?P<name>[^,]+), channel (?P<channel>0x[0-9a-f]+),.*?MMU Fault: ENGINE (?P<engine>\S+) (?P<client>\S+) faulted @ (?P<va>0x[0-9a-f_]+)\. Fault is of type (?P<fault>\S+) (?P<access>\S+)$")
    for line in lines:
        m = pattern.search(line)
        if m is None:
            raise GuardRefused("unmatched Xid record")
        actual.append((int(m['code']), int(m['channel'], 16), int(m['va'].replace('_', ''), 16), m['fault'], m['access'], m['engine'], m['client'], m['name']))
    if Counter(actual) != Counter(expected):
        raise GuardRefused("Xid type/channel/address/source multiset differs from fault controls")
    return lines


def check_unchanged(journal, baseline):
    if xid_lines(journal) != tuple(baseline):
        raise GuardRefused("new/replaced/lost Xid after frozen native proof")


SUITE_FILES = (
    'crates/kayfabe-rm-ladder/src/main.rs',
    'scripts/fastguest/fast_suite.sh',
    'scripts/fastguest/run_fast_guest.sh',
    'scripts/fastguest/build_fast_guest.sh',
)


def pin_suite(root, revision):
    """Pin the actual source/runner files, refusing a stale or modified suite."""
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise GuardRefused('missing/invalid suite revision')
    root = Path(root)
    actual = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    if actual != revision:
        raise GuardRefused('suite revision differs from pinned revision')
    result = {}
    for name in SUITE_FILES:
        committed = subprocess.check_output(['git', 'show', revision + ':' + name], cwd=root)
        local = (root / name).read_bytes()
        if local != committed or len(local) > MAX_BYTES:
            raise GuardRefused('modified/oversized suite source: ' + name)
        result[name] = hashlib.sha256(local).hexdigest()
    return result


def capture_guest(pid, binary, revision, arm, root):
    """Only the supervisor's live QEMU child supplies process identity."""
    identity = process_identity(pid)
    if pid <= 1 or identity is None or identity[0] != os.getpid():
        raise GuardRefused('QEMU is not the live owned child')
    executable = str(Path('/proc', str(pid), 'exe').resolve(strict=True))
    if executable != str(Path(binary).resolve(strict=True)):
        raise GuardRefused('QEMU executable identity differs')
    return {'pid': pid, 'starttime': identity[1], 'exe': executable,
            'arm': arm, 'revision': revision, 'source_pin': pin_suite(root, revision)}


def verify_guest_identity(context):
    identity = process_identity(context['pid'])
    if identity is None or identity[1] != context['starttime']:
        raise GuardRefused('owned QEMU starttime/executable changed or disappeared')
    process = Path('/proc', str(context['pid']))
    try:
        executable = str((process / 'exe').resolve(strict=True))
    except FileNotFoundError:
        # A thread-group leader can exit before QEMU's remaining threads. Its
        # exe link then disappears although the owned Popen child is still live.
        # Verify every retained thread image; only kernel-confirmed exiting
        # tasks may lack one. PF_EXITING is Linux include/linux/sched.h's 0x4.
        if not task_exiting(process):
            raise GuardRefused('live owned QEMU executable disappeared')
        for task in (process / 'task').iterdir():
            try:
                retained = str((task / 'exe').resolve(strict=True))
            except FileNotFoundError:
                if not task_exiting(task):
                    raise GuardRefused('live owned worker executable disappeared')
                continue
            if retained != context['exe']:
                raise GuardRefused('owned QEMU worker executable changed')
        final_identity = process_identity(context['pid'])
        if final_identity is None or final_identity[1] != context['starttime']:
            raise GuardRefused('owned QEMU identity disappeared during thread verification')
        return
    if executable != context['exe']:
        raise GuardRefused('owned QEMU starttime/executable changed or disappeared')


def task_exiting(task):
    try:
        fields = bounded(task / 'stat').rpartition(')')[2].split()
    except FileNotFoundError:
        return True  # Reaped tasks cannot execute userspace again.
    if len(fields) < 20:
        raise GuardRefused('owned task status unreadable')
    return bool(int(fields[6]) & 0x4)


def verify_binary_revision(binary, revision):
    if (not re.fullmatch(r'[0-9a-f]{40}', revision) or
            Path(binary).parent.name != revision[:8] or Path(binary).name != 'qemu-system-x86_64'):
        raise GuardRefused('binary is not in its pinned revision directory')


def guest_launch_arm(argv):
    if argv.count('-append') != 1:
        raise GuardRefused('fast guest needs one pinned kernel command line')
    append = argv[argv.index('-append') + 1].split()
    selections = [word.removeprefix('KF_ARMS=') for word in append if word.startswith('KF_ARMS=')]
    if len(selections) != 1:
        raise GuardRefused('missing/duplicate arm selection')
    tokens = selections[0].split(',')
    if len(tokens) != 2 or tokens[1] != '--probe-launch-dma' or not re.fullmatch(r'--[a-z0-9-]+', tokens[0]):
        raise GuardRefused('one pinned arm and launch-dma primitive required')
    return tokens[0]


def _context(context):
    if (type(context.get('pid')) is not int or context['pid'] <= 1 or
            type(context.get('starttime')) is not int or context['starttime'] <= 0 or
            not isinstance(context.get('exe'), str) or not context['exe'].startswith('/') or
            not re.fullmatch(r'[0-9a-f]{40}', context.get('revision', '')) or
            not context.get('source_pin') or
            any(not re.fullmatch(r'[0-9a-f]{64}', value) for value in context['source_pin'].values())):
        raise GuardRefused('missing actual QEMU identity or suite source pin')
    if context.get('arm') not in ('--defer-liveness', '--missing-page-fault'):
        raise GuardRefused('guest exception is only for the two pinned negative controls')


def _guest_record(line):
    if line.count('NVRM: Xid') != 1:
        raise GuardRefused('multiple/hidden Xid records')
    match = re.fullmatch(
        r'[^\n]*? kernel: NVRM: Xid \(PCI:(?P<pci>[0-9a-f:]+)\): '
        r'(?P<code>[0-9]+), pid=(?P<pid>[0-9]+), name=(?P<name>[^,]+), '
        r'channel (?P<channel>0x[0-9a-f]+), intr (?P<intr>[0-9a-f]{8})\. '
        r'MMU Fault: ENGINE (?P<engine>\S+) (?P<client>\S+) faulted @ '
        r'(?P<va>0x[0-9a-f_]+)\. Fault is of type (?P<fault>\S+) (?P<access>\S+)', line)
    if match is None:
        raise GuardRefused('unparsed guest Xid record')
    return match.groupdict()


def _doorbells(text):
    return [(m.start(), int(m[1])) for m in re.finditer(
        r'DOORBELL-STORE #[0-9]+ host_token=0x[0-9a-f]+ runlist=[0-9]+ chid=([0-9]+)', text)]


def _guest_operations(serial, arm, final):
    """Source-shaped operations; partial observations never approve a baseline."""
    lines = serial.splitlines()
    if final:
        if not re.search(r'^FASTGUEST: arms ' + re.escape(arm) + r' --probe-launch-dma  deadline ', serial, re.M):
            raise GuardRefused('wrong arm/primitive in pinned suite')
        if 'FASTGUEST: DONE' not in lines or not re.search(r'^FASTGUEST: client rc=0 at ', serial, re.M):
            raise GuardRefused('guest did not complete successfully')
    result = []
    if arm == '--defer-liveness':
        markers = list(re.finditer(r'^FAULT_MARK=defer_liveness_prime arm=(\S+) va=(0x[0-9a-f]+)(?: .*)?$', serial, re.M))
        names = ['PLAIN(rig-control)', 'DEFER(the-question)', 'DEFER+INVALIDATE(known-positive)']
        if len(markers) > 3 or [m[1] for m in markers] != names[:len(markers)]:
            raise GuardRefused('unexpected source defer markers/order')
        if final and len(markers) != 3:
            raise GuardRefused('missing source defer markers')
        for i, marker in enumerate(markers):
            tail = serial[marker.end():markers[i + 1].start() if i + 1 < len(markers) else len(serial)]
            doors = _doorbells(tail)
            va = int(marker[2], 16)
            if doors:
                result.append((doors[0][1], va, 'FAULT_PTE', 'ACCESS_TYPE_VIRT_READ'))
            # Source: prime, surviving test control, rescue control, then test.
            if i == 1 and len(doors) >= 4:
                result.append((doors[3][1], va, 'FAULT_PTE', 'ACCESS_TYPE_VIRT_READ'))
            if final:
                prime = re.search(r'info\s+DL prime copy\s*= Dead', tail)
                test = re.search(r'info\s+DL test copy\s*= (Live|Dead)', tail)
                if (prime is None or test is None or not doors or
                        channel_before(tail, prime.start()) != doors[0][1] or
                        not re.search(r'DL prime notifier\s*= fired=true status=0xffff except_type=0x1f\b', tail)):
                    raise GuardRefused('defer prime source/notifier proof absent')
                wanted = 'Dead' if i == 1 else 'Live'
                if test[1] != wanted:
                    raise GuardRefused('defer control outcomes differ from approved four-fault shape')
                if i == 1 and (len(doors) < 4 or channel_before(tail, test.start()) != doors[3][1] or
                               'DL rescue copy     = Live' not in tail or
                               not re.search(r'DL test notifier\s*= fired=true status=0xffff except_type=0x1f\b', tail)):
                    raise GuardRefused('defer rescue/channel/notifier proof absent')
        if final and any(x not in lines for x in ('DEFER_PRIME_FAULTS=3/3', 'DEFER_RIGCTL=PASS',
                                                  'DEFER_NEGCTL=PASS', 'RUNG_defer_liveness=PASS')):
            raise GuardRefused('defer positive recovery verdict absent')
    else:
        markers = list(re.finditer(r'^FAULT_MARK=victim_release va=(0x[0-9a-f]+)$', serial, re.M))
        if len(markers) > 1 or (final and len(markers) != 1):
            raise GuardRefused('missing/duplicate victim source marker')
        if markers:
            tail = serial[markers[0].end():]
            doors = _doorbells(tail)
            if doors:
                result.append((doors[0][1], int(markers[0][1], 16), 'FAULT_PDE', 'ACCESS_TYPE_VIRT_WRITE'))
            if final and (not doors or not re.search(r'R3 victim\s*= Lost', tail) or
                          not re.search(r'R3 notifier\s*= fired=true status=0xffff except_type=0x1f\b', tail) or
                          'bystander before = Landed' not in serial or 'bystander after  = Landed' not in tail or
                          any(x not in lines for x in ('RUNGCTL_missing_page=PASS', 'RUNG_missing_page=PASS'))):
                raise GuardRefused('missing-page source/notifier/bystander recovery proof absent')
    return result


def approve_guest(journal, baseline, serial, qlog, context, *, final=True):
    """Return extended exact history ONLY after complete proof; None is pending.

    During a pinned negative arm, reject any irreconcilable fault immediately.
    Matching incomplete observations are pending, never a phase acceptance.
    No next QEMU/arm may run until final=True succeeds with recovery and twins.
    """
    _context(context)
    lines = xid_lines(journal)
    if len(baseline) < 5 or lines[:len(baseline)] != tuple(baseline):
        raise GuardRefused('native/accepted Xid history changed or lost')
    pci = re.search(r'Xid \(PCI:([0-9a-f:]+)\)', baseline[0])
    if pci is None:
        raise GuardRefused('native PCI identity absent')
    operations = _guest_operations(serial, context['arm'], final)
    added = lines[len(baseline):]
    count = 4 if context['arm'] == '--defer-liveness' else 1
    if len(added) > count or len(added) > len(operations) or (final and len(added) != count):
        raise GuardRefused('guest faults differ from exact source operation count')
    twins = []
    for line in qlog.splitlines():
        if 'kf3: RC host twin' not in line:
            continue
        m = re.search(r'kf3: RC host twin (0x[0-9a-f]+) \(guest chid (0x[0-9a-f]+), engine (0x[0-9a-f]+)\) wrote its notifier: except_type=0x1f \(Xid 31\) — forwarding RC_TRIGGERED$', line)
        if m is None:
            raise GuardRefused('unparsed/unrelated host-twin fault')
        twins.append((int(m[1], 16), int(m[2], 16)))
    if len(twins) > count or (final and len(twins) != count):
        raise GuardRefused('missing/extra physical-to-guest channel correlation')
    for i, line in enumerate(added):
        record = _guest_record(line)
        guest_chid, va, fault, access = operations[i]
        if (int(record['code']) != 31 or int(record['pid']) != context['pid'] or
                record['name'] != 'qemu-system-x86' or record['pci'] != pci[1] or
                record['intr'] != '00000000' or record['engine'] != 'CE0' or record['client'] != 'HUBCLIENT_CE1' or
                int(record['va'].replace('_', ''), 16) != va or record['fault'] != fault or record['access'] != access):
            raise GuardRefused('guest Xid source/PID/address/type/route differs')
        if i < len(twins) and twins[i] != (int(record['channel'], 16), guest_chid):
            raise GuardRefused('host-twin/guest channel differs from source submission')
    return lines if final else None


def supervise_guest(binary, argv, receipt, revision, root, serial, qlog):
    """Own one QEMU; no receipt advances until its full proof and clean exit.

    The caller runs one suite arm per invocation and stops on any nonzero exit.
    Kernel history is also checked before launch, covering inter-arm gaps.
    """
    verify_binary_revision(binary, revision)
    pin_suite(root, revision)
    arm = guest_launch_arm(argv)
    state = json.loads(bounded(receipt))
    baseline = state.get('accepted_xid_lines', state['approved_native_xid_lines'])
    check_unchanged(kernel_snapshot(), baseline)
    child = subprocess.Popen([str(binary), *argv])
    try:
        context = capture_guest(child.pid, binary, revision, arm, root)
        negative = arm in ('--defer-liveness', '--missing-page-fault')
        while True:
            # Read the kernel first: source console output predates submission,
            # and the later file reads cannot predate an observed kernel fault.
            journal = kernel_snapshot()
            running = child.poll() is None
            if running:
                try:
                    verify_guest_identity(context)
                except (GuardRefused, FileNotFoundError):
                    if child.poll() is None:
                        raise
                    running = False
            if negative:
                def read_log(path):
                    try:
                        return bounded(path)
                    except FileNotFoundError:
                        if not running:
                            raise GuardRefused('final guest proof log missing')
                        return ''
                serial_text, qlog_text = read_log(serial), read_log(qlog)
                accepted = approve_guest(journal, baseline, serial_text, qlog_text, context, final=not running)
            else:
                check_unchanged(journal, baseline)
                accepted = tuple(baseline)
            if not running:
                break
            time.sleep(1)  # Existing bench observation cadence; no product timer.
        if child.wait() != 0:
            raise GuardRefused('owned QEMU did not exit successfully')
        if negative:
            state['accepted_xid_lines'] = accepted
            state.setdefault('guest_proofs', []).append({
                **context, 'serial_sha256': hashlib.sha256(serial_text.encode()).hexdigest(),
                'qlog_sha256': hashlib.sha256(qlog_text.encode()).hexdigest(),
                'accepted_faults': len(accepted) - len(baseline),
            })
        state.setdefault('completed_arms', []).append(context)
        temporary = Path(str(receipt) + '.next')
        temporary.write_text(json.dumps(state, indent=2) + '\n')
        temporary.replace(receipt)
        return 0
    finally:
        if child.poll() is None:
            child.terminate()  # Popen-owned child only; never a process pattern.
            child.wait(timeout=30)


def kernel_snapshot():
    p = subprocess.Popen(['timeout', '10', 'journalctl', '-k', '-b', '-o', 'short-iso-precise', '--no-pager'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        data = p.stdout.read(MAX_BYTES + 1)
        if len(data) > MAX_BYTES:
            raise GuardRefused("kernel journal byte bound exceeded")
        if p.wait(timeout=10) != 0 or len(data.splitlines()) < 2:
            raise GuardRefused("kernel journal unreadable/empty")
        return data.decode('utf-8', 'strict')
    finally:
        if p.poll() is None:
            p.terminate()
            p.wait(timeout=10)


def process_identity(pid):
    try:
        with Path('/proc', str(pid), 'stat').open() as f:
            text = f.read(4097)
        if len(text) > 4096:
            raise GuardRefused('process identity byte bound exceeded')
        fields = text.rpartition(')')[2].split()
        return int(fields[1]), int(fields[19])  # PPid and starttime, after comm.
    except FileNotFoundError:
        return None


def terminate_owned_tree(root):
    """Freeze and terminate the recorded controller's descendants, across PGIDs.

    timeout/setsid can create new groups. A group signal alone leaves those GPU
    jobs alive while a shell waits to run its TERM trap. Starttimes prevent PID
    reuse from extending ownership to another process.
    """
    current = os.getpid()
    if root <= 1 or os.getpgrp() != root or os.getpgid(root) != root:
        raise GuardRefused('abort target is not the own controller group')
    frozen = {}

    def same(pid, identity):
        now = process_identity(pid)
        # Parentage proves the original tree, but changes when a parent exits.
        # The immutable starttime, rather than PPid, identifies the same task.
        return now is not None and now[1] == identity[1]

    try:
        identity = process_identity(root)
        if root != current and identity is not None:
            os.kill(root, signal.SIGSTOP)
            frozen[root] = identity
        for _ in range(3):
            records = {}
            for p in Path('/proc').iterdir():
                if not p.name.isdigit():
                    continue
                if len(records) >= 16384:
                    raise GuardRefused('owned-tree process bound exceeded')
                identity = process_identity(int(p.name))
                if identity is not None:
                    records[int(p.name)] = identity
            owned = {root}
            while True:
                found = {pid for pid, (parent, _) in records.items() if parent in owned}
                if found <= owned:
                    break
                owned |= found
            for pid in owned - {current}:
                identity = records.get(pid)
                if identity is not None and same(pid, identity):
                    try:
                        os.kill(pid, signal.SIGSTOP)
                        frozen[pid] = identity
                    except ProcessLookupError:
                        pass
    finally:
        # Queue every TERM while the tree is frozen; resume children before the
        # controller so its EXIT trap cannot kill this watcher prematurely.
        for pid, identity in frozen.items():
            if same(pid, identity):
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        order = [pid for pid in frozen if pid != root] + ([root] if root in frozen else [])
        for pid in order:
            if same(pid, frozen[pid]):
                try:
                    os.kill(pid, signal.SIGCONT)
                except ProcessLookupError:
                    pass
        os.killpg(root, signal.SIGTERM)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='mode', required=True)
    n = sub.add_parser('native')
    n.add_argument('journal'); n.add_argument('defer'); n.add_argument('missing')
    c = sub.add_parser('check'); c.add_argument('journal'); c.add_argument('receipt')
    w = sub.add_parser('watch'); w.add_argument('receipt'); w.add_argument('owned_pgid', type=int); w.add_argument('fatal_receipt')
    g = sub.add_parser('guest-exec')
    g.add_argument('receipt'); g.add_argument('revision'); g.add_argument('binary')
    g.add_argument('arguments', nargs=argparse.REMAINDER)
    a = p.parse_args()
    try:
        if a.mode == 'native':
            lines = approve_native(bounded(a.journal), bounded(a.defer), bounded(a.missing))
            print(json.dumps({'approved_native_xid_lines': lines, 'count': len(lines), 'rule': 'owner decision1: source-matched controls only; all later changes fatal'}))
        elif a.mode == 'check':
            check_unchanged(bounded(a.journal), json.loads(bounded(a.receipt))['approved_native_xid_lines'])
        elif a.mode == 'guest-exec':
            argv = a.arguments[1:] if a.arguments[:1] == ['--'] else a.arguments
            serials = []
            for i, argument in enumerate(argv[:-1]):
                if argument == '-chardev' and argv[i + 1].startswith('file,id=kfcon0,path='):
                    serials.append(argv[i + 1].split('path=', 1)[1])
            if len(serials) != 1:
                raise GuardRefused('one hvc serial proof file required')
            qlog = Path('/proc/self/fd/1').resolve(strict=True)
            if not qlog.is_file():
                raise GuardRefused('QEMU log must be a retained regular file')
            return supervise_guest(a.binary, argv, a.receipt, a.revision,
                                   Path(__file__).resolve().parents[2], serials[0], qlog)
        else:
            if a.owned_pgid <= 1 or os.getpgid(a.owned_pgid) != a.owned_pgid or os.getpgrp() != a.owned_pgid:
                raise GuardRefused('watcher must share its own recorded controller process group')
            baseline = json.loads(bounded(a.receipt))['approved_native_xid_lines']
            while True:
                try:
                    check_unchanged(kernel_snapshot(), baseline)
                except (GuardRefused, OSError, ValueError, subprocess.SubprocessError) as error:
                    try:
                        Path(a.fatal_receipt).write_text(json.dumps({'STOP': str(error), 'owned_pgid': a.owned_pgid}) + '\n')
                    finally:
                        terminate_owned_tree(a.owned_pgid)
                    raise
                time.sleep(1)  # Bench observation only; no product timer is changed.
    except (GuardRefused, OSError, ValueError, subprocess.SubprocessError) as error:
        print('NATIVE_XID_GUARD_STOP: ' + str(error), file=sys.stderr)
        return 7
    return 0


if __name__ == '__main__':
    sys.exit(main())
