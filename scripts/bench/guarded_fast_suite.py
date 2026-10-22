#!/usr/bin/env python3
"""Run every pinned fast-suite arm, stopping before any successor on failure."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('guard', ROOT / 'scripts/bench/native_xid_guard.py')
G = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(G)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('tag'); p.add_argument('receipt'); p.add_argument('revision'); p.add_argument('binary')
    p.add_argument('--fastguest-dir', required=True, help='directory built for the selected guest driver')
    a = p.parse_args()
    images = Path(a.fastguest_dir).resolve()
    for name in ('vmlinuz', 'initrd.cpio.gz'):
        if not (images / name).is_file():
            raise G.GuardRefused('missing selected guest image: ' + str(images / name))
    G.pin_suite(ROOT, a.revision); G.verify_binary_revision(a.binary, a.revision)
    text = (ROOT / 'scripts/fastguest/fast_suite.sh').read_text()
    block = re.search(r'ARMS=\((--timer .*?)\)\nfi', text, re.S)
    if block is None:
        raise G.GuardRefused('pinned default arm list absent')
    arms = block[1].split()
    if len(arms) != 30 or len(set(arms)) != 30:
        raise G.GuardRefused('pinned suite must contain all 30 distinct arms')
    env = dict(os.environ, KF_GUARD_RECEIPT=str(Path(a.receipt).resolve()),
               KF_GUARD_REVISION=a.revision, KF_GUARD_QEMU=str(Path(a.binary).absolute()),
               QEMU_BIN=str(ROOT / 'scripts/bench/guest_xid_qemu.sh'), KF_FASTGUEST_DIR=str(images))
    bench = Path(env.get('BENCH_DIR', '/workspace/bench'))
    completed = []
    for arm in arms:
        print('GUARDED_ARM_START ' + arm, flush=True)
        name = a.tag + '_' + arm[2:]
        log = bench / (name + '_controller.log')
        with log.open('w') as out:
            result = subprocess.run(['bash', str(ROOT / 'scripts/fastguest/fast_suite.sh'), name, '180', arm],
                                    env=env, stdout=out, stderr=subprocess.STDOUT, check=False)
        cell = log.read_text()
        if result.returncode or 'FAST_SUITE_PASS=1 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=1' not in cell:
            print('GUARDED_SUITE_STOP arm=' + arm + ' rc=' + str(result.returncode), flush=True)
            return 7
        state = json.loads(G.bounded(a.receipt))
        records = state.get('completed_arms', [])
        if ([record['arm'] for record in records] != completed + [arm] or
                any(record['revision'] != a.revision or record['pid'] <= 1 or record['starttime'] <= 0
                    for record in records)):
            raise G.GuardRefused('arm has no fresh successful supervisor completion')
        if arm in ('--defer-liveness', '--missing-page-fault'):
            expected = [4] if arm == '--defer-liveness' else [4, 1]
            if [proof['accepted_faults'] for proof in state.get('guest_proofs', [])] != expected:
                raise G.GuardRefused('guest control lacks durable exact recovery proof')
        completed.append(arm)
        print('GUARDED_ARM_PASS ' + arm, flush=True)
    state = json.loads(G.bounded(a.receipt))
    G.check_unchanged(G.kernel_snapshot(), state.get('accepted_xid_lines', state['approved_native_xid_lines']))
    if [proof['accepted_faults'] for proof in state.get('guest_proofs', [])] != [4, 1]:
        raise G.GuardRefused('both complete guest-control proofs required')
    print('FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30', flush=True)
    print('GUARDED_SUITE_RC=0', flush=True)
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (G.GuardRefused, OSError, ValueError, subprocess.SubprocessError) as error:
        print('GUARDED_SUITE_STOP: ' + str(error), file=sys.stderr)
        sys.exit(7)
