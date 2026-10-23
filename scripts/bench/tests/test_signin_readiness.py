# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Sign-in observers reuse the measured profile without retrying keyboard input."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest

WINDOWS = Path(__file__).resolve().parents[1] / 'windows'
sys.path.insert(0, str(WINDOWS))
spec = importlib.util.spec_from_file_location('s4_signin_ready', WINDOWS / 'signin.py')
signin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signin)


class SigninReadinessTests(unittest.TestCase):
    def probe(self, scale=4, remaining=1000, replies=None):
        self.calls = []
        pending = list(replies or [(0, 'KFWIN\\kf\n')])
        def execute(argv, **kwargs):
            self.calls.append((argv, kwargs['timeout']))
            rc, text = pending.pop(0)
            return subprocess.CompletedProcess(argv, rc, text, '')
        return signin.session_probe(Path('qmp.py'), 'owned-qga.sock', 'kf',
                                    profile={'scale': scale}, remaining=lambda: remaining,
                                    execute=execute)

    def test_slow_profile_reaches_outer_query_deadline(self):
        self.assertTrue(self.probe(scale=8))
        self.assertEqual(self.calls[0][1], 160)

    def test_fourfold_profile_reaches_outer_query_deadline(self):
        self.assertTrue(self.probe())
        self.assertEqual(self.calls[0][1], 80)

    def test_inner_qga_timeout_receives_scaled_budget(self):
        self.probe(scale=8)
        self.assertIn('--timeout=160', self.calls[0][0])

    def test_inner_qga_timeout_receives_remaining_stage(self):
        self.probe(scale=10, remaining=37)
        self.assertEqual(self.calls[0][1], 37)
        self.assertIn('--timeout=37', self.calls[0][0])

    def test_timeout_partial_user_is_discarded_then_fresh_query_used(self):
        self.assertFalse(self.probe(replies=[(124, 'KFWIN\\kf\n'), (0, '')]))
        self.assertEqual(len(self.calls), 2)

    def test_two_timeouts_refuse_without_third_observer(self):
        with self.assertRaises(RuntimeError):
            self.probe(replies=[(124, 'KFWIN\\kf\n'), (124, 'KFWIN\\kf\n')])
        self.assertEqual(len(self.calls), 2)

    def test_expired_stage_never_invokes_observer(self):
        with self.assertRaises(RuntimeError):
            self.probe(remaining=0)
        self.assertEqual(self.calls, [])

    def test_default_observer_keeps_twenty_second_behavior(self):
        calls = []
        def execute(argv, **kwargs):
            calls.append(kwargs['timeout'])
            return subprocess.CompletedProcess(argv, 0, 'KFWIN\\kf\n', '')
        self.assertTrue(signin.session_probe(Path('qmp.py'), 'sock', 'kf', execute=execute))
        self.assertEqual(calls, [20])
