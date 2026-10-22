#!/usr/bin/env python3
"""Verify the optional wrapper preserves all arms and stops at first failure."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('guarded_suite_test', ROOT / 'scripts/bench/guarded_fast_suite.py')
S = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(S)


class GuardedSuiteTests(unittest.TestCase):
    def run_suite(self, failing_at):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        baseline = json.loads((ROOT / 'traces/sweep_windows_20261021/lease_57081143/native/native_xid_baseline.json').read_text())
        baseline['guest_proofs'] = [{'accepted_faults': 4}, {'accepted_faults': 1}]
        calls = []
        original_run = subprocess.run
        with tempfile.TemporaryDirectory(prefix='s2-suite-driver-') as folder:
            receipt = Path(folder) / 'baseline.json'; receipt.write_text(json.dumps(baseline))
            args = SimpleNamespace(tag='unit', receipt=str(receipt), revision=rev,
                                   binary='/bench/kf3-bins/' + rev[:8] + '/qemu-system-x86_64')
            def run(argv, **kwargs):
                if argv[0] != 'bash':
                    return original_run(argv, **kwargs)
                calls.append(argv[-1])
                fail = len(calls) == failing_at
                kwargs['stdout'].write('FAST_SUITE_PASS=1 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=1\n' if not fail else 'FAST_SUITE_FAIL=1\n')
                return SimpleNamespace(returncode=int(fail))
            with mock.patch.object(S.argparse.ArgumentParser, 'parse_args', return_value=args), \
                 mock.patch.dict(S.os.environ, {'BENCH_DIR': folder}), \
                 mock.patch.object(S.subprocess, 'run', side_effect=run), \
                 mock.patch.object(S.G, 'kernel_snapshot', return_value='\n'.join(baseline['approved_native_xid_lines'])):
                code = S.main()
        return code, calls

    def test_first_arm_failure_stops_before_second(self):
        code, calls = self.run_suite(1)
        self.assertEqual(code, 7); self.assertEqual(calls, ['--timer'])

    def test_seventh_arm_failure_stops_before_eighth(self):
        code, calls = self.run_suite(7)
        self.assertEqual(code, 7); self.assertEqual(len(calls), 7)

    def test_all_thirty_distinct_pinned_arms_preserved(self):
        code, calls = self.run_suite(0)
        self.assertEqual(code, 0); self.assertEqual(len(calls), 30); self.assertEqual(len(set(calls)), 30)
        self.assertIn('--defer-liveness', calls); self.assertIn('--missing-page-fault', calls)


if __name__ == '__main__': unittest.main()
