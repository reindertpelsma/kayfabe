#!/usr/bin/env python3
"""Run one Translated production mutant at a time; restore even on failure."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent if Path(__file__).parent.name == 'scripts' else Path.cwd()
SLOT = ROOT / 'crates/kf-qemu/src/translated_slot.rs'
CHAN = ROOT / 'crates/kf-qemu/src/chan.rs'
MUTANTS = [
 ('loser_drops_registration', SLOT, '        self.register();\n        self.changed_since(seen)', '        self.changed_since(seen)'),
 ('holder_omits_ring', SLOT, '            (self.cell.ring)(self.cell.identity.idx);', '            let _ = self.cell.identity.idx;'),
 ('ring_before_unlock', SLOT, '''        drop(self.g.take());
        if self.cell.retry.released() {
            (self.cell.ring)(self.cell.identity.idx);
        }''', '''        if self.cell.retry.released() {
            (self.cell.ring)(self.cell.identity.idx);
        }
        drop(self.g.take());'''),
 ('late_registration_no_recheck', SLOT, '        self.epoch.load(Ordering::SeqCst) != seen', '        { let _ = seen; false }'),
 ('release_never_advances_epoch', SLOT, '        self.epoch.fetch_add(1, Ordering::SeqCst);', '        let _ = &self.epoch;'),
 ('holder_checks_before_epoch_bump', SLOT, '        self.bump();\n        self.take()', '        let pending = self.take();\n        self.bump();\n        pending'),
 ('metadata_takes_pump_lock', SLOT, '        (!self.inner.is_poisoned()).then_some(self.identity)', '        self.inner.lock().ok().map(|_g| self.identity)'),
 ('worker_bypasses_handoff', CHAN, 'let Ok(mut g) = slot.enter() else {', 'let Ok(mut g) = slot.try_lock() else {'),
 ('schedule_blocks_on_drainer', CHAN, '        self.defer(\n            "schedule translated",', '        drop(slot.lock());\n        self.defer(\n            "schedule translated",'),
 ('diagnostic_blocks_on_drainer', CHAN, '''        let captured = self.slot(t.host);
        let answer = self.defer(''', '''        let captured = self.slot(t.host);
        if let Some(slot) = &captured { drop(slot.lock()); }
        let answer = self.defer('''),
]


def check():
    subprocess.run(['df', '-h', '/'], check=True, stdout=subprocess.PIPE)
    free = shutil.disk_usage('/').free
    if free < 4 * 1024**3:
        raise RuntimeError('disk guard below 4 GiB; STOP, clean own target and reassess')
    cmd = ['nice', '-n', '19', 'flock', '-w', '1200', os.environ['HW3_CARGO_LOCK'],
           'cargo', 'test', '-j2', '--release', '-p', 'kf-qemu', '--lib', 'translated_', '--', '--nocapture']
    t0 = time.monotonic()
    run = subprocess.run(cmd, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    failed = sorted(set(re.findall(r'^test (\S+) \.\.\. FAILED', run.stdout, re.M)))
    passed = len(re.findall(r'^test \S+ \.\.\. ok', run.stdout, re.M))
    built = 'could not compile' not in run.stdout and ('test result:' in run.stdout)
    return dict(built=built, passed=passed, failed=failed, code=run.returncode, seconds=round(time.monotonic()-t0, 2)), run.stdout


def main():
    selected = set(sys.argv[1:])
    baseline, log = check()
    print(json.dumps(dict(name='baseline', **baseline)), flush=True)
    if not baseline['built'] or baseline['code'] != 0:
        Path('/tmp/hw3-mutation-error.log').write_text(log)
        return 2
    bad = False
    for name, path, old, new in MUTANTS:
        if selected and name not in selected:
            continue
        original = path.read_text()
        if original.count(old) != 1:
            print(json.dumps(dict(name=name, error='patch mismatch', matches=original.count(old))), flush=True)
            return 2
        try:
            path.write_text(original.replace(old, new))
            result, log = check()
        finally:
            path.write_text(original)
        killed = result['built'] and len(result['failed']) >= 2
        print(json.dumps(dict(name=name, killed=killed, **result)), flush=True)
        if not killed:
            bad = True
            Path('/tmp/hw3-mutation-error.log').write_text(log)
    return int(bad)

if __name__ == '__main__':
    sys.exit(main())
