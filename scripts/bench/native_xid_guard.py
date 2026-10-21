#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Owner-approved native negative-control proof and frozen later-Xid guard.

Bench only. No product policy or global Xid31 exemption. Marker addresses and
channels come from this run's positive-control logs, never a per-die table.
"""
import argparse
from collections import Counter
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
    a = p.parse_args()
    try:
        if a.mode == 'native':
            lines = approve_native(bounded(a.journal), bounded(a.defer), bounded(a.missing))
            print(json.dumps({'approved_native_xid_lines': lines, 'count': len(lines), 'rule': 'owner decision1: source-matched controls only; all later changes fatal'}))
        elif a.mode == 'check':
            check_unchanged(bounded(a.journal), json.loads(bounded(a.receipt))['approved_native_xid_lines'])
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
