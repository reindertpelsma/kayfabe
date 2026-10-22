#!/usr/bin/env python3
"""Bench-only campaign guard. Native source controls, then immutable per-arm fault proof.

Native permission is exactly code/channel/VA/fault/access/source plus recovery.
Engine/client labels are recorded metadata: they vary with the actual host route.
No native32 is accepted. Every later31 is fatal. QEMU32 needs this arm's PID,
exact bounded shape, and initialization already observed before its timestamp.
"""
import argparse
from collections import Counter
from datetime import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import subprocess
import signal
import time

import xid_policy

SPEC = importlib.util.spec_from_file_location('native_xid_guard', Path(__file__).resolve().parents[1] / 'native_xid_guard.py')
N = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(N)
MMU = re.compile(r'NVRM: Xid \(PCI:[^)]+\): ([0-9]+), pid=[0-9]+, name=([^,]+), channel (0x[0-9a-f]+),.*?MMU Fault: ENGINE (\S+) (\S+) faulted @ (0x[0-9a-f_]+)\. Fault is of type (\S+) (\S+)$')


def source_faults(defer, missing):
    """Source-marked control operations; no per-die engine/client constants."""
    result = []
    if defer:
        for line in ('DEFER_PRIME_FAULTS=3/3','DEFER_RIGCTL=PASS','DEFER_NEGCTL=PASS','RUNG_defer_liveness=PASS'):
            if line not in defer.splitlines():
                raise N.GuardRefused('defer recovery proof absent')
        marks = list(re.finditer(r'^FAULT_MARK=defer_liveness_prime arm=(\S+) va=(0x[0-9a-f]+)', defer, re.M))
        if len(marks) != 3:
            raise N.GuardRefused('three prime markers required')
        for mark in marks:
            tail = defer[mark.end():].split('═══ DL ARM',1)[0]
            prime = re.search(r'info\s+DL prime copy\s*= Dead',tail)
            test = re.search(r'info\s+DL test copy\s*= (Live|Dead)',tail)
            if prime is None or test is None:
                raise N.GuardRefused('prime/test result absent')
            result.append((31,N.channel_before(tail,prime.start()),int(mark[2],16),'FAULT_PTE','ACCESS_TYPE_VIRT_READ','kayfabe-rm-ladd','defer-liveness'))
            if test[1]=='Dead':
                if mark[1]!='DEFER(the-question)' or 'DL rescue copy     = Live' not in tail:
                    raise N.GuardRefused('unexpected/unrecovered test fault')
                result.append((31,N.channel_before(tail,test.start()),int(mark[2],16),'FAULT_PTE','ACCESS_TYPE_VIRT_READ','kayfabe-rm-ladd','defer-liveness'))
    if missing:
        for line in ('RUNGCTL_missing_page=PASS','RUNG_missing_page=PASS'):
            if line not in missing.splitlines():
                raise N.GuardRefused('missing-page recovery absent')
        marks = list(re.finditer(r'^FAULT_MARK=victim_release va=(0x[0-9a-f]+)',missing,re.M))
        if len(marks)!=1 or 'bystander before = Landed' not in missing or 'bystander after  = Landed' not in missing:
            raise N.GuardRefused('missing-page marker/bystanders absent')
        tail=missing[marks[0].end():]; victim=re.search(r'info\s+R3 victim\s*= Lost',tail)
        if victim is None:
            raise N.GuardRefused('victim did not fault')
        result.append((31,N.channel_before(tail,victim.start()),int(marks[0][1],16),'FAULT_PDE','ACCESS_TYPE_VIRT_WRITE','kayfabe-rm-ladd','missing-page-fault'))
    return result


def parse_native(line):
    if line.count('NVRM: Xid')!=1:
        raise N.GuardRefused('multiple/unrecognized native Xid markers')
    m=MMU.search(line)
    if m is None:
        raise N.GuardRefused('native Xid lacks source-controlled MMU shape')
    record=(int(m[1]),int(m[3],16),int(m[6].replace('_',''),16),m[7],m[8],m[2])
    return record,m


def native_baseline(journal,defer,missing):
    expected=[x[:6] for x in source_faults(defer,missing)]
    lines=N.xid_lines(journal)
    actual=[parse_native(line)[0] for line in lines]
    if Counter(actual)!=Counter(expected):
        raise N.GuardRefused('native source/channel/address/type/recovery differs')
    return lines


def utc(value):
    at = datetime.fromisoformat(value)
    if at.tzinfo is None:
        raise ValueError('timestamp lacks timezone')
    return at


def evaluate_snapshot(journal, accepted, phase_lines, arm, pid, init_utc):
    lines = N.xid_lines(journal)
    if lines[:len(accepted)] != tuple(accepted):
        raise N.GuardRefused('frozen kernel history lost or replaced')
    new = lines[len(accepted):]
    if not new:
        return lines
    if arm is None or pid is None or init_utc is None:
        raise N.GuardRefused('new fault without current arm/init proof')
    current = tuple(phase_lines) + new
    verdict = xid_policy.judge(current, arm, int(pid), True)
    if verdict['verdict'] != 'EXPECTED':
        raise N.GuardRefused('fault outside exact owned-QEMU exception')
    try:
        init = utc(init_utc)
        if any(utc(line.split()[0]) < init for line in current):
            raise N.GuardRefused('fault predates observed full-init prefix')
    except (ValueError, IndexError) as error:
        raise N.GuardRefused('unprovable fault/init timestamp') from error
    return lines


def init_time(path):
    """A complete consecutive1000-row prefix proves RPC progress; Code0 is separate."""
    if not path or not Path(path).exists():
        return None
    previous = None
    try:
        with Path(path).open() as f:
            for ordinal in range(1000):
                line = f.readline(4097)
                if not line.endswith('\n') or len(line) > 4096:
                    return None
                row = json.loads(line)
                if row['ordinal'] != ordinal:
                    return None
                at = utc(row['observed_utc'])
                if previous is not None and at < previous:
                    return None
                previous = at
        return previous.isoformat()
    except (OSError, ValueError, KeyError, TypeError):
        return None


def phase_init_time(phase):
    """Progress plus a host-observed Code0 for this exact VM life, never eventual Code0."""
    progress = init_time(phase.get('rpc_path'))
    proof = phase.get('windows_init')
    if progress is None or not isinstance(proof, dict):
        return None
    try:
        if type(proof['code']) is not int or proof['code'] != 0 or proof['driver'] != '32.0.15.8088':
            return None
        if any(proof[key] != phase[key] for key in ('arm', 'pid', 'starttime')):
            return None
        return max(utc(progress), utc(proof['observed_utc'])).isoformat()
    except (KeyError, TypeError, ValueError):
        return None


def record_windows_init(phase_path, diag_path, arm):
    """Host wrapper records actual CIM output and the host observation time."""
    import hashlib
    phase = json.loads(N.bounded(phase_path))
    if phase['arm'] != arm:
        raise N.GuardRefused('init observation belongs to another arm')
    phase['expected_pci'] = phase['pci']
    validate_phase_owner(phase)
    with Path(diag_path).open() as f:
        text = f.read(65537)
    if len(text) > 65536:
        raise N.GuardRefused('Windows diagnostic byte bound exceeded')
    rows = [line[len('BW580_RESULT '):] for line in text.splitlines() if line.startswith('BW580_RESULT ')]
    if len(rows) != 1:
        raise N.GuardRefused('one actual Windows diagnostic result required')
    row = json.loads(rows[0])
    phase.pop('windows_init', None)
    if type(row.get('code')) is int and row['code'] == 0 and row.get('driver') == '32.0.15.8088' and row.get('tdr4101') == 0:
        phase['windows_init'] = dict(arm=arm, pid=phase['pid'], starttime=phase['starttime'],
                                    code=0, driver=row['driver'], observed_utc=datetime.now().astimezone().isoformat(),
                                    diagnostic_sha256=hashlib.sha256(text.encode()).hexdigest())
    atomic_json(phase_path, phase)


def atomic_json(path, data):
    p = Path(path)
    tmp = p.with_name(p.name + '.tmp')
    tmp.write_text(json.dumps(data) + '\n')
    tmp.replace(p)


def verify_native_run(journal,defer,missing,receipts,pci):
    lines=native_baseline(journal,defer,missing)
    pids={}
    for row in receipts:
        kind=row['kind']; pid=int(row['pid'])
        if pid<=1 or kind not in ('defer-liveness','missing-page-fault') or kind in pids:
            raise N.GuardRefused('invalid/duplicate native process receipt')
        pids[kind]=pid
    expected=[]
    for item in source_faults(defer,missing):
        if item[6] not in pids:
            raise N.GuardRefused('missing native source PID')
        expected.append(item[:6]+(pids[item[6]],pci))
    actual=[]
    for line in lines:
        record,m=parse_native(line)
        prefix=re.search(r'NVRM: Xid \(PCI:([^)]+)\): [0-9]+, pid=([0-9]+),',line)
        if prefix is None:
            raise N.GuardRefused('native process/device missing')
        # The operation selects COPY-read or GRAPHICS semaphore-write. Physical
        # engine/client numbers are decoded by this host, never borrowed by die.
        route=(m[4],m[5])
        if record[3:5]==('FAULT_PTE','ACCESS_TYPE_VIRT_READ'):
            good=re.fullmatch(r'CE[0-9]+',route[0]) and re.fullmatch(r'HUBCLIENT_CE[0-9]+',route[1])
        else:
            good=re.fullmatch(r'GR(?:_HOST[0-9]+|[0-9]+_PBDMA[0-9]+)',route[0]) and re.fullmatch(r'HUBCLIENT_ESC[0-9]*',route[1])
        if not good:
            raise N.GuardRefused('source operation and native route differ')
        actual.append(record+(int(prefix[2]),prefix[1]))
    if Counter(actual)!=Counter(expected):
        raise N.GuardRefused('native source PID/device multiset differs')
    return lines


def validate_phase_owner(phase,proc_root='/proc'):
    try:
        pid=int(phase['pid'])
        if pid<=1 or phase['pci']!=phase['expected_pci']:
            raise N.GuardRefused('invalid owned PID/device')
        proc=Path(proc_root)/str(pid)
        fields=(proc/'stat').read_text()[:4097].rpartition(')')[2].split()
        if int(fields[19])!=int(phase['starttime']) or (proc/'exe').resolve(strict=True)!=Path(phase['exe']).resolve(strict=True):
            raise N.GuardRefused('owned QEMU starttime/executable changed')
    except (OSError,KeyError,ValueError,IndexError) as error:
        raise N.GuardRefused('owned process proof missing/invalid') from error
    return True


def abort_owned_qemu(phase):
    """Stop only the previously verified VM, including after it daemonized."""
    if not phase or int(phase.get('pid',0))<=1:
        return False
    descriptor=None
    try:
        descriptor=os.pidfd_open(int(phase['pid']),0)
        validate_phase_owner(phase)
        signal.pidfd_send_signal(descriptor,signal.SIGKILL)
        return True
    except (OSError,N.GuardRefused):
        return False  # Already exited or identity changed: never signal a reused PID.
    finally:
        if descriptor is not None:
            os.close(descriptor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='mode', required=True)
    n = sub.add_parser('native')
    for arg in ('journal', 'defer', 'missing', 'receipts', 'pci'): n.add_argument(arg)
    w = sub.add_parser('watch')
    for arg in ('receipt', 'pgid', 'phase', 'fatal'): w.add_argument(arg)
    r = sub.add_parser('record-init')
    for arg in ('phase', 'diag', 'arm'): r.add_argument(arg)
    a = parser.parse_args()
    try:
        if a.mode == 'record-init':
            record_windows_init(a.phase, a.diag, a.arm)
            return 0
        if a.mode == 'native':
            lines = verify_native_run(N.bounded(a.journal), N.bounded(a.defer), N.bounded(a.missing), json.loads(N.bounded(a.receipts)), a.pci)
            print(json.dumps({'approved_native_xid_lines': lines, 'pci': a.pci, 'rule': 'source controls only; route names recorded, not a die constant'}))
            return 0
        root = int(a.pgid)
        if root <= 1 or os.getpgid(root) != root or os.getpgrp() != root:
            raise N.GuardRefused('watcher is not in its owned controller group')
        receipt = json.loads(N.bounded(a.receipt))
        accepted = tuple(receipt['approved_native_xid_lines'])
        phase_key = None
        owned_phase = None
        phase_lines = ()
        atomic_json(a.receipt + '.ready.json', {'owned_pgid':root})
        while True:
            try:
                if Path(a.fatal).exists():
                    raise N.GuardRefused('prior fatal marker')
                phase = json.loads(N.bounded(a.phase)) if Path(a.phase).exists() else {}
                key = (phase.get('arm'), phase.get('pid'))
                if key != phase_key:
                    if key[0] is not None:
                        phase['expected_pci']=receipt['pci']
                        validate_phase_owner(phase)
                        owned_phase = dict(phase)
                    phase_lines = ()
                    phase_key = key
                proof = phase_init_time(phase)
                journal = N.kernel_snapshot()
                for line in N.xid_lines(journal)[len(accepted):]:
                    if receipt.get('pci') and ('NVRM: Xid (PCI:'+receipt['pci']+'):') not in line:
                        raise N.GuardRefused('fault on another device')
                lines = evaluate_snapshot(journal, accepted, phase_lines, *key, proof)
                phase_lines += lines[len(accepted):]
                accepted = lines
                atomic_json(a.receipt + '.accepted.json', {'approved_native_xid_lines': accepted, 'phase': phase_key, 'init_utc': proof})
            except Exception as error:  # Any observer failure is fatal, including subprocess timeouts.
                try:
                    stopped = abort_owned_qemu(owned_phase)
                    atomic_json(a.fatal, {'STOP': str(error), 'phase': phase_key, 'owned_pgid': root, 'detached_qemu_stopped':stopped})
                finally:
                    N.terminate_owned_tree(root)
                raise
            time.sleep(1)  # Bench observer; no product timer changes.
    except (N.GuardRefused, OSError, ValueError, subprocess.SubprocessError) as error:
        print('PHASE_XID_STOP: ' + str(error), file=sys.stderr)
        return 7


if __name__ == '__main__':
    sys.exit(main())
