#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded hand mutants; each must be killed by at least two named core tests."""
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / 'crates/kf-prof/src'
MUTANTS = [
    ('seqlock ordering', 'publication.rs',
     [('s.fence(Ordering::Release);', ''), ('s.fence(Ordering::Acquire);', '')]),
    ('ring slot wrap', 'ring.rs', [('(self.next as usize) & (N - 1)', '0usize')]),
    ('histogram bucket', 'histogram.rs', [('(64 - ns.leading_zeros()) as usize', '(63 - ns.leading_zeros()) as usize')]),
    ('ring overflow count', 'ring.rs', [('self.dropped.saturating_add(1)', 'self.dropped')]),
    ('clock calibration', 'clock.rs', [('.checked_mul(1_000_000_000)', '.checked_mul(1_000_000_001)')]),
    ('ledger slot reuse', 'ledger.rs', [('k.generations[slot] + 1', '1')]),
    ('ledger overflow count', 'ledger.rs', [('self.dropped.saturating_add(1)', 'self.dropped')]),
    ('exclusive residence', 'thread.rs', [('now - self.segment_ns', 'now - self.current.enter_ns')]),
    ('header geometry', 'layout.rs', [('get(b, 16) != FILE_BYTES as u64', 'false')]),
]

def run():
    target = os.environ.get('CARGO_TARGET_DIR')
    if not target:
        raise SystemExit('set CARGO_TARGET_DIR to a lane-owned target directory')
    report = ['MEASURED: each mutant changes only production source; test names below killed it.']
    for name, filename, replacements in MUTANTS:
        path = SRC / filename
        original = path.read_text()
        changed = original
        for before, after in replacements:
            if before not in changed:
                raise SystemExit(f'{name}: mutation anchor missing')
            changed = changed.replace(before, after)
        try:
            path.write_text(changed)
            subprocess.run(['df', '-h', '/'], check=True, stdout=subprocess.DEVNULL)
            if shutil.disk_usage('/').free < 4 * 1024**3:
                raise SystemExit('disk guard: mutation build stopped')
            result = subprocess.run(['nice','-n','19','cargo','test','-p','kf-prof','--test','core','-j2','--release'],
                                    cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90)
            kills = re.findall(r'^test (\S+) \.\.\. FAILED$', result.stdout, re.M)
            if result.returncode == 0 or len(kills) < 2:
                raise SystemExit(f'{name}: insufficient kill evidence: {kills}\n{result.stdout[-4000:]}')
            report.append(f'{name}: {len(kills)} killing tests: ' + ', '.join(kills))
            print(report[-1], flush=True)
        finally:
            path.write_text(original)
    (Path(__file__).parent / 'core_mutations.txt').write_text('\n'.join(report) + '\n')

if __name__ == '__main__':
    run()
