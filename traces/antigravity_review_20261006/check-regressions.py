#!/usr/bin/env python3
"""Show that the repaired regression tests reject the preserved Antigravity code.

Run only in a disposable worktree at 781f2f9f. The reference worktree is read-only.
This runs filtered Rust logic tests, with no QEMU or GPU experiment.
"""
import argparse
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--reference', type=Path, required=True)
parser.add_argument('--mutant', type=Path, required=True)
parser.add_argument('--target', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
revision = subprocess.check_output(
    ['git', '-C', str(args.mutant), 'rev-parse', 'HEAD'], text=True).strip()
if not revision.startswith('781f2f9f'):
    parser.error('The disposable worktree must be the preserved Antigravity tip')
if subprocess.check_output(['git', '-C', str(args.mutant), 'status', '--porcelain']):
    parser.error('The disposable worktree must start clean')
args.output.mkdir(mode=0o700, exist_ok=False)
for relative in ['crates/kf-disp/src/caps.rs', 'crates/kf-rm/src/display.rs']:
    path = args.mutant / relative
    old = path.read_text()
    reference = (args.reference / relative).read_text()
    marker = '#[cfg(test)]\nmod tests {'
    if old.count(marker) != 1 or reference.count(marker) != 1:
        raise RuntimeError('Unexpected test module layout: ' + relative)
    path.write_text(old.split(marker)[0] + marker + reference.split(marker)[1])
env = dict(os.environ, CARGO_TARGET_DIR=str(args.target), CARGO_BUILD_JOBS='6',
           KAYFABE_KVM_DEVICE='/dev/null')
cases = [('kf-rm', 'unsupported_windows_controls_never_get_a_success_reply'),
         ('kf-disp', 'caps::tests::')]
for package, test_filter in cases:
    result = subprocess.run(['cargo', 'test', '--lib', '-p', package, test_filter],
                            cwd=args.mutant, env=env, capture_output=True,
                            text=True, timeout=300)
    log = result.stdout + result.stderr
    (args.output / (package + '.log')).write_text(log)
    if result.returncode != 101 or 'test result: FAILED.' not in log:
        raise RuntimeError('Expected an executed test failure, not a build error: ' + package)
    print('REGRESSION_DETECTED', package, 'original=' + revision, flush=True)
print('REGRESSION_CHECK_EXIT=0', flush=True)
