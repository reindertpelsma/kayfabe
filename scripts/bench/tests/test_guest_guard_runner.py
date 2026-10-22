#!/usr/bin/env python3
"""S2 EXPECTED RED: the supervisor must bind real owned QEMU identity.

FALSIFIER: it accepts another process, a changed process identity, an unpinned
multi-arm launch, or a failure that can advance the suite.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('runner_guard', ROOT / 'scripts/bench/native_xid_guard.py')
G = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(G)


class RunnerIdentityTests(unittest.TestCase):
    def test_actual_child_identity_is_captured(self):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(10)'])
        try:
            context = G.capture_guest(child.pid, sys.executable, rev, '--defer-liveness', ROOT)
            self.assertEqual(context['pid'], child.pid)
            self.assertEqual(context['starttime'], G.process_identity(child.pid)[1])
            G.verify_guest_identity(context)
        finally:
            child.terminate(); child.wait(timeout=5)

    def test_changed_starttime_is_fatal(self):
        with mock.patch.object(G, 'process_identity', return_value=(os.getpid(), 8)):
            with self.assertRaises(G.GuardRefused):
                G.verify_guest_identity({'pid': os.getpid(), 'starttime': 7, 'exe': sys.executable})

    def test_changed_executable_is_fatal(self):
        with mock.patch.object(G, 'process_identity', return_value=(os.getppid(), 7)):
            with self.assertRaises(G.GuardRefused):
                G.verify_guest_identity({'pid': os.getpid(), 'starttime': 7, 'exe': '/different/qemu'})

    def wrong_capture_binary(self, binary):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(10)'])
        try:
            with self.assertRaises(G.GuardRefused): G.capture_guest(child.pid, binary, rev, '--defer-liveness', ROOT)
        finally:
            child.terminate(); child.wait(timeout=5)

    def test_wrong_capture_executable_true_refused(self):
        self.wrong_capture_binary('/usr/bin/true')

    def test_wrong_capture_executable_false_refused(self):
        self.wrong_capture_binary('/usr/bin/false')

    def test_parent_is_not_owned_qemu(self):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        binary = Path('/proc', str(os.getppid()), 'exe').resolve()
        with self.assertRaises(G.GuardRefused):
            G.capture_guest(os.getppid(), binary, rev, '--defer-liveness', ROOT)

    def test_unrelated_process_is_not_owned_qemu(self):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        with self.assertRaises(G.GuardRefused):
            G.capture_guest(os.getpid(), sys.executable, rev, '--defer-liveness', ROOT)

    def test_single_negative_arm_is_parsed(self):
        self.assertEqual(G.guest_launch_arm(['-append', 'console=hvc0 KF_ARMS=--defer-liveness,--probe-launch-dma']), '--defer-liveness')

    def test_other_single_arm_is_guarded_without_exception(self):
        self.assertEqual(G.guest_launch_arm(['-append', 'KF_ARMS=--timer,--probe-launch-dma']), '--timer')

    def test_multiple_arms_in_one_boot_refused(self):
        with self.assertRaises(G.GuardRefused):
            G.guest_launch_arm(['-append', 'KF_ARMS=--defer-liveness,--timer,--probe-launch-dma'])

    def test_changed_primitive_refused(self):
        with self.assertRaises(G.GuardRefused):
            G.guest_launch_arm(['-append', 'KF_ARMS=--missing-page-fault'])

    def test_stale_binary_revision_refused(self):
        with self.assertRaises(G.GuardRefused):
            G.verify_binary_revision('/bench/kf3-bins/aaaaaaaa/qemu-system-x86_64', 'b' * 40)

    def test_arbitrary_binary_directory_refused(self):
        with self.assertRaises(G.GuardRefused):
            G.verify_binary_revision('/bench/qemu-system-x86_64', 'b' * 40)

    def test_selected_revision_directory_accepted(self):
        G.verify_binary_revision('/bench/kf3-bins/bbbbbbbb/qemu-system-x86_64', 'b' * 40)

    def supervise_failure(self, cause):
        rev = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        fix = ROOT / 'traces/sweep_windows_20261021/lease_57081143/native/native_xid_baseline.json'
        data = json.loads(fix.read_text()); base = '\n'.join(data['approved_native_xid_lines']) + '\n'
        with tempfile.TemporaryDirectory(prefix='s2-supervisor-test-') as folder:
            folder = Path(folder); binary = folder / rev[:8] / 'qemu-system-x86_64'
            binary.parent.mkdir(); binary.symlink_to(Path(sys.executable).resolve())
            receipt = folder / 'receipt.json'; receipt.write_text(json.dumps(data))
            serial = folder / 'serial'; serial.write_text('')
            qlog = folder / 'qlog'; qlog.write_text('')
            pidfile = folder / 'child.pid'
            script = 'import os,time,pathlib; pathlib.Path(' + repr(str(pidfile)) + ').write_text(str(os.getpid())); time.sleep(10)'
            snapshots = [base, base + data['approved_native_xid_lines'][0].replace(': 31,', ': 32,') + '\n']
            if cause == 'journal': snapshots[1] = G.GuardRefused('unreadable journal')
            original_popen = subprocess.Popen; children = []
            def spawn(argv, *args, **kwargs):
                child = original_popen(argv, *args, **kwargs)
                if str(argv[0]) == str(binary): children.append(child)
                return child
            try:
                with mock.patch.object(G, 'kernel_snapshot', side_effect=snapshots), \
                     mock.patch.object(G.subprocess, 'Popen', side_effect=spawn):
                    with self.assertRaises(G.GuardRefused):
                        G.supervise_guest(binary, ['-c', script, '-append', 'KF_ARMS=--timer,--probe-launch-dma'],
                                          receipt, rev, ROOT, serial, qlog)
                self.assertEqual(json.loads(receipt.read_text()), data, 'failure must not extend accepted history')
                self.assertTrue(children)
                self.assertTrue(all(child.poll() is not None for child in children), 'owned child survived refusal')
            finally:
                for child in children:
                    if child.poll() is None: child.terminate()
                    child.wait(timeout=5)

    def test_unrelated_fault_stops_child_and_cannot_advance_receipt(self):
        self.supervise_failure('xid')

    def test_unreadable_journal_stops_child_and_cannot_advance_receipt(self):
        self.supervise_failure('journal')


class ExitRaceTests(unittest.TestCase):
    def exercise(self, arm, live=False, exit_code=0):
        from test_guest_xid_guard import GuestGuardTests
        fixture = GuestGuardTests(); fixture.setUp()
        if arm == '--missing-page-fault':
            case = fixture.missing(); context = case['context']; lines = case['lines']; serial_text = case['serial']; qlog_text = case['qlog']
        else:
            context = fixture.context; lines = fixture.lines; serial_text = fixture.serial; qlog_text = fixture.qlog
        base = {'approved_native_xid_lines': fixture.base}
        child = mock.Mock()
        child.pid = context['pid']; child.poll.side_effect = [None, None if live else exit_code, None if live else exit_code]
        child.wait.return_value = exit_code
        with tempfile.TemporaryDirectory(prefix='s2-exit-race-') as folder:
            folder = Path(folder); receipt = folder / 'receipt.json'; receipt.write_text(json.dumps(base))
            serial = folder / 'serial'; serial.write_text(serial_text)
            qlog = folder / 'qlog'; qlog.write_text(qlog_text)
            refusal = None; result = None
            with mock.patch.object(G, 'pin_suite'), mock.patch.object(G, 'verify_binary_revision'), \
                 mock.patch.object(G, 'capture_guest', return_value=context), \
                 mock.patch.object(G, 'verify_guest_identity', side_effect=FileNotFoundError('/proc/owned/exe')), \
                 mock.patch.object(G, 'kernel_snapshot', side_effect=['\n'.join(fixture.base), '\n'.join(lines)]), \
                 mock.patch.object(G.subprocess, 'Popen', return_value=child):
                try:
                    result = G.supervise_guest('/unused/qemu', ['-append', 'KF_ARMS=' + arm + ',--probe-launch-dma'],
                                               receipt, context['revision'], ROOT, serial, qlog)
                except (G.GuardRefused, OSError) as error: refusal = error
            state = json.loads(receipt.read_text())
            return result, refusal, state, base, context

    def assert_completed_exit(self, arm):
        result, refusal, state, base, context = self.exercise(arm)
        self.assertIsNone(refusal, 'owned child exited during proc read; final exact proof must still be checked')
        self.assertEqual(result, 0)
        self.assertEqual(state['approved_native_xid_lines'], base['approved_native_xid_lines'])
        self.assertEqual(state['guest_proofs'][0]['pid'], context['pid'])
        self.assertIn('completed_arms', state)
        self.assertEqual(state['completed_arms'][-1]['arm'], arm)

    def test_defer_child_exit_during_exe_read_saves_final_proof(self):
        self.assert_completed_exit('--defer-liveness')

    def test_missing_child_exit_during_exe_read_saves_final_proof(self):
        self.assert_completed_exit('--missing-page-fault')

    def test_live_defer_missing_exe_is_still_fatal(self):
        result, refusal, state, base, context = self.exercise('--defer-liveness', live=True)
        self.assertIsNotNone(refusal); self.assertIsNone(result); self.assertEqual(state, base)

    def test_live_missing_page_missing_exe_is_still_fatal(self):
        result, refusal, state, base, context = self.exercise('--missing-page-fault', live=True)
        self.assertIsNotNone(refusal); self.assertIsNone(result); self.assertEqual(state, base)

    def test_failed_defer_exit_cannot_save_completion(self):
        result, refusal, state, base, context = self.exercise('--defer-liveness', exit_code=7)
        self.assertIsNotNone(refusal); self.assertIsNone(result); self.assertEqual(state, base)

    def test_failed_missing_exit_cannot_save_completion(self):
        result, refusal, state, base, context = self.exercise('--missing-page-fault', exit_code=7)
        self.assertIsNotNone(refusal); self.assertIsNone(result); self.assertEqual(state, base)


if __name__ == '__main__': unittest.main()
