#!/usr/bin/env python3
"""S2 EXPECTED RED: the supervisor must bind real owned QEMU identity.

FALSIFIER: it accepts another process, a changed process identity, an unpinned
multi-arm launch, or a failure that can advance the suite.
"""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
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

    def test_parent_is_not_owned_qemu(self):
        with self.assertRaises(G.GuardRefused):
            G.capture_guest(os.getppid(), sys.executable, 'a' * 40, '--defer-liveness', ROOT)

    def test_unrelated_process_is_not_owned_qemu(self):
        with self.assertRaises(G.GuardRefused):
            G.capture_guest(os.getpid(), sys.executable, 'a' * 40, '--defer-liveness', ROOT)

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


if __name__ == '__main__': unittest.main()
