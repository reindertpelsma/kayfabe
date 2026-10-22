#!/usr/bin/env python3
"""Verify the optional wrapper preserves all arms and stops at first failure."""
import importlib.util
import json
import os
import shutil
import sys
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
    def run_suite(self, failing_at, lose_completion_at=None, lose_proof_at=None):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        baseline = json.loads((ROOT / 'traces/sweep_windows_20261021/lease_57081143/native/native_xid_baseline.json').read_text())
        baseline['guest_proofs'] = []
        baseline['completed_arms'] = []
        calls = []
        original_run = subprocess.run
        with tempfile.TemporaryDirectory(prefix='s2-suite-driver-') as folder:
            receipt = Path(folder) / 'baseline.json'; receipt.write_text(json.dumps(baseline))
            args = SimpleNamespace(tag='unit', receipt=str(receipt), revision=rev,
                                   binary='/bench/kf3-bins/' + rev[:8] + '/qemu-system-x86_64',
                                   fastguest_dir=str(Path(folder) / 'images'))
            images = Path(args.fastguest_dir); images.mkdir()
            for name in ('vmlinuz', 'initrd.cpio.gz'): (images / name).write_text('fixture')
            def run(argv, **kwargs):
                if argv[0] != 'bash':
                    return original_run(argv, **kwargs)
                calls.append(argv[-1])
                fail = len(calls) == failing_at
                if not fail and len(calls) != lose_completion_at:
                    baseline['completed_arms'].append({'arm': argv[-1], 'revision': rev, 'pid': 1234, 'starttime': 1})
                    if argv[-1] in ('--defer-liveness', '--missing-page-fault') and len(calls) != lose_proof_at:
                        baseline['guest_proofs'].append({'arm': argv[-1], 'accepted_faults': 4 if argv[-1] == '--defer-liveness' else 1})
                    receipt.write_text(json.dumps(baseline))
                kwargs['stdout'].write('FAST_SUITE_PASS=1 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=1\n' if not fail else 'FAST_SUITE_FAIL=1\n')
                return SimpleNamespace(returncode=int(fail))
            with mock.patch.object(S.argparse.ArgumentParser, 'parse_args', return_value=args), \
                 mock.patch.dict(S.os.environ, {'BENCH_DIR': folder}), \
                 mock.patch.object(S.subprocess, 'run', side_effect=run), \
                 mock.patch.object(S.G, 'kernel_snapshot', return_value='\n'.join(baseline['approved_native_xid_lines'])):
                try: code = S.main()
                except S.G.GuardRefused: code = 7
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

    def test_shell_pass_without_timer_supervisor_completion_stops_immediately(self):
        code, calls = self.run_suite(0, lose_completion_at=1)
        self.assertEqual(code, 7); self.assertEqual(calls, ['--timer'])

    def test_shell_pass_without_engines_completion_stops_before_third(self):
        code, calls = self.run_suite(0, lose_completion_at=2)
        self.assertEqual(code, 7); self.assertEqual(calls, ['--timer', '--engines'])

    def test_shell_pass_without_defer_proof_cannot_start_successor(self):
        code, calls = self.run_suite(0, lose_completion_at=7)
        self.assertEqual(code, 7); self.assertEqual(len(calls), 7)
        self.assertEqual(calls[-1], '--defer-liveness')

    def test_completion_without_defer_recovery_proof_stops_before_successor(self):
        code, calls = self.run_suite(0, lose_proof_at=7)
        self.assertEqual(code, 7); self.assertEqual(len(calls), 7)

    def test_completion_without_missing_page_recovery_stops_before_successor(self):
        code, calls = self.run_suite(0, lose_proof_at=13)
        self.assertEqual(code, 7); self.assertEqual(len(calls), 13)


class GuestImagePathTests(unittest.TestCase):
    """Real shell launch boundary; the only QEMU is an argv recorder, no GPU."""
    def run_real_launch(self, image_name, missing=None):
        with tempfile.TemporaryDirectory(prefix='s2-image-path-') as folder:
            root = Path(folder)
            for relative in ('scripts/fastguest/fast_suite.sh', 'scripts/fastguest/run_fast_guest.sh',
                             'scripts/bench/guest_profile.sh'):
                target = root / relative; target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, target)
            images = root / image_name; images.mkdir()
            for name in ('vmlinuz', 'initrd.cpio.gz'):
                if name != missing: (images / name).write_text('GPU-free fixture')
            (images / 'driver.STAGED').write_text('version=595.91.07\n')
            record = root / 'qemu-argv.json'
            wrapper = root / 'scripts/bench/guest_xid_qemu.sh'
            wrapper.write_text('#!' + sys.executable + '\nimport json,sys\n'
                               + 'open(' + repr(str(record)) + ',"w").write(json.dumps(sys.argv[1:]))\n'
                               + 'sys.exit(1)\n')
            wrapper.chmod(0o755)
            commands = root / 'commands'; commands.mkdir()
            for name, body in [('pgrep', 'exit 1'), ('nvidia-smi', 'echo 0')]:
                command = commands / name; command.write_text('#!/bin/sh\n' + body + '\n'); command.chmod(0o755)
            args = SimpleNamespace(tag='path', receipt=str(root / 'receipt.json'), revision='a' * 40,
                                   binary='/unused/qemu', fastguest_dir=str(images))
            env = dict(BENCH_DIR=str(root), KF_LOCK=str(root / 'own.lock'), KF3_FB_MB='4096',
                       PATH=str(commands) + ':' + os.environ['PATH'], KF_FASTGUEST_DIR=str(root / 'wrong-inherited-path'))
            refused = None
            with mock.patch.object(S, 'ROOT', root), \
                 mock.patch.object(S.argparse.ArgumentParser, 'parse_args', return_value=args), \
                 mock.patch.object(S.G, 'pin_suite'), mock.patch.object(S.G, 'verify_binary_revision'), \
                 mock.patch.dict(S.os.environ, env):
                try: code = S.main()
                except S.G.GuardRefused as error: code = 7; refused = str(error)
            argv = json.loads(record.read_text()) if record.exists() else None
            return code, argv, str(images), refused

    def assert_image_launch(self, name):
        code, argv, images, refused = self.run_real_launch(name)
        self.assertIsNotNone(argv, 'selected built images must reach the real QEMU launch boundary')
        self.assertEqual(argv[argv.index('-kernel') + 1], images + '/vmlinuz')
        self.assertEqual(argv[argv.index('-initrd') + 1], images + '/initrd.cpio.gz')
        self.assertEqual(code, 7, 'the fake QEMU cannot manufacture a passing GPU verdict')
        self.assertIsNone(refused)

    def test_controller_image_directory_reaches_real_shell_launch(self):
        self.assert_image_launch('fastguest595')

    def test_explicit_image_directory_with_spaces_overrides_inherited_path(self):
        self.assert_image_launch('images with spaces')

    def test_missing_kernel_refused_before_arm(self):
        code, argv, images, refused = self.run_real_launch('missing-kernel', 'vmlinuz')
        self.assertIsNone(argv); self.assertEqual(code, 7)
        self.assertIsNotNone(refused, 'preflight must name the missing selected image')
        self.assertIn(images + '/vmlinuz', refused)

    def test_missing_initrd_refused_before_arm(self):
        code, argv, images, refused = self.run_real_launch('missing-initrd', 'initrd.cpio.gz')
        self.assertIsNone(argv); self.assertEqual(code, 7)
        self.assertIsNotNone(refused, 'preflight must name the missing selected image')
        self.assertIn(images + '/initrd.cpio.gz', refused)

    def test_image_option_required_before_provenance_or_gpu_access(self):
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/bench/guarded_fast_suite.py'),
                                 'tag', '/unused/receipt', 'a' * 40, '/unused/qemu'],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn('--fastguest-dir', result.stderr)

    def test_real_cli_missing_images_reports_selected_path(self):
        with tempfile.TemporaryDirectory(prefix='s2-cli-images-') as folder:
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/bench/guarded_fast_suite.py'),
                                     'tag', '/unused/receipt', 'a' * 40, '/unused/qemu',
                                     '--fastguest-dir', folder], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 7)
            self.assertIn(str(Path(folder) / 'vmlinuz'), result.stderr)


if __name__ == '__main__': unittest.main()
