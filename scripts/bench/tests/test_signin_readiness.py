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
        inner = next(x.split('=', 1)[1] for x in self.calls[0][0] if x.startswith('--timeout='))
        self.assertEqual(float(inner), 160)

    def test_inner_qga_timeout_receives_remaining_stage(self):
        self.probe(scale=10, remaining=37)
        outer = self.calls[0][1]
        inner = next(x.split('=', 1)[1] for x in self.calls[0][0] if x.startswith('--timeout='))
        self.assertGreater(outer, 0)
        self.assertLessEqual(outer, 37)
        self.assertEqual(float(inner), outer)

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


class SigninContextTests(unittest.TestCase):
    def context(self, wrong_box=False, wrong_digest=False):
        import json
        import tempfile
        from unittest.mock import patch
        import readiness
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            profile = readiness.make_profile(60, 2, box_id=123, source='a' * 40)
            (root / 'profile').write_text(json.dumps(profile))
            stage = {'box_id': 999 if wrong_box else 123,
                     'profile_sha256': 'bad' if wrong_digest else readiness.digest(profile),
                     'end_monotonic': 1000}
            (root / 'stage').write_text(json.dumps(stage))
            env = {'WINVM_READINESS_PROFILE': str(root / 'profile'),
                   'WINVM_READINESS_STAGE': str(root / 'stage')}
            with patch.dict(signin.os.environ, env, clear=True):
                return signin.readiness_context()

    def test_stage_from_another_owned_box_refuses(self):
        with self.assertRaises(ValueError):
            self.context(wrong_box=True)

    def test_stage_from_another_calibration_refuses(self):
        with self.assertRaises(ValueError):
            self.context(wrong_digest=True)

    def test_unconfigured_context_preserves_legacy_default(self):
        from unittest.mock import patch
        with patch.dict(signin.os.environ, {}, clear=True):
            self.assertEqual(signin.readiness_context(), (None, None))


class SigninArmWiringTests(unittest.TestCase):
    def run_arm_call(self):
        import os
        import tempfile
        arm = WINDOWS.parents[0] / 'diesweep/desktop/arm.sh'
        call = arm.read_text().split('if KF_GUEST_PW=', 1)[1].split('; then', 1)[0]
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / 'secrets').mkdir()
            (root / 'secrets/win_password').write_text('fixture-only')
            call = call.replace('/root/prov/', td + '/')
            script = '''set -e
VM=$FIXTURE
READINESS_STAGE=owned-stage.json
run=fixture
readiness_step() { echo "MODE=$1 STAGE=$WINVM_READINESS_STAGE"; }
KF_GUEST_PW=''' + call
            env = dict(os.environ, FIXTURE=td)
            p = subprocess.run(['bash', '-c', script], env=env,
                               capture_output=True, text=True, timeout=5)
            out = root / 'arm_fixture_signin.txt'
            return out.read_text() if out.exists() else p.stdout + p.stderr

    def test_signin_action_uses_single_attempt_wrapper(self):
        self.assertIn('MODE=once', self.run_arm_call())

    def test_signin_observer_receives_persisted_arm_stage(self):
        self.assertIn('STAGE=owned-stage.json', self.run_arm_call())
