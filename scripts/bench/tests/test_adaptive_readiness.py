"""Owner-authorized bench deadlines: calibration, finite budgets and actual replies."""
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

WINDOWS = Path(__file__).resolve().parents[1] / 'windows'
sys.path.insert(0, str(WINDOWS))


class Scaling(unittest.TestCase):
    def setUp(self):
        self.r = importlib.import_module('readiness')

    def profile(self, boot=60, query=2):
        return self.r.make_profile(boot, query, box_id=1234, source='a' * 40)

    def test_fast_box_keeps_minimum_fourfold_margin(self):
        self.assertEqual(self.profile(30, .2)['scale'], 4)

    def test_small_query_alone_does_not_remove_boot_slowdown(self):
        self.assertEqual(self.profile(361, .1)['scale'], 7)

    def test_slow_query_alone_scales_every_step(self):
        self.assertEqual(self.profile(30, 13)['scale'], 7)

    def test_extreme_boot_caps_at_ten(self):
        self.assertEqual(self.profile(10000, 1)['scale'], 10)

    def test_extreme_query_caps_at_ten(self):
        self.assertEqual(self.profile(20, 1000)['scale'], 10)

    def test_nonfinite_measurements_refused(self):
        for boot, query in [(float('nan'), 1), (1, float('inf')), (0, 1), (1, -2)]:
            with self.subTest(boot=boot, query=query), self.assertRaises(ValueError):
                self.profile(boot, query)

    def test_step_obeys_absolute_ceiling(self):
        self.assertEqual(self.r.step_seconds(20, self.profile(600, 2), 90, 500), 90)

    def test_second_step_also_obeys_its_ceiling(self):
        self.assertEqual(self.r.step_seconds(120, self.profile(600, 2), 300, 1000), 300)

    def test_remaining_stage_can_shorten_a_step(self):
        self.assertEqual(self.r.step_seconds(20, self.profile(), 180, 11), 11)

    def test_no_step_starts_after_stage_exhaustion(self):
        with self.assertRaises(self.r.BudgetExpired):
            self.r.step_seconds(20, self.profile(), 180, 0)

    def test_export_reserve_caps_stage_before_scaled_bound(self):
        self.assertEqual(self.r.stage_seconds(600, self.profile(), 3600, 1500, 1200), 300)

    def test_a_slower_box_cannot_spend_the_export_reserve(self):
        self.assertEqual(self.r.stage_seconds(600, self.profile(600, 2), 3600, 1350, 1200), 150)

    def test_calibration_is_written_once(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / 'box.json'
            self.r.write_profile(p, self.profile())
            with self.assertRaises(FileExistsError):
                self.r.write_profile(p, self.profile(600, 2))
            self.assertEqual(json.loads(p.read_text())['scale'], 4)


class Queries(Scaling):
    def query(self, replies, remaining=lambda: 500, marker='KF_READY'):
        self.calls = []
        replies = iter(replies)
        def operation(limit):
            self.calls.append(limit)
            value = next(replies)
            if isinstance(value, BaseException):
                raise value
            if callable(value):
                return value()
            return value
        return self.r.run_query(operation, profile=self.profile(), nominal=10,
                                ceiling=90, remaining=remaining, marker=marker)

    @staticmethod
    def result(code=0, text='KF_READY\n', error=''):
        return subprocess.CompletedProcess(['owned-query'], code, text, error)

    def test_first_timeout_retries_and_accepts_only_fresh_output(self):
        r = self.query([subprocess.TimeoutExpired('owned-query', 40, output='STALE'), self.result()])
        self.assertEqual(r.stdout, 'KF_READY\n')
        self.assertEqual(self.calls, [40, 40])

    def test_first_banner_failure_retries_once(self):
        r = self.query([self.result(255, '', 'Connection timed out during banner exchange'), self.result()])
        self.assertEqual(r.returncode, 0)
        self.assertEqual(len(self.calls), 2)

    def test_second_timeout_stops_without_a_third_query(self):
        with self.assertRaises(self.r.ReadinessTimeout):
            self.query([subprocess.TimeoutExpired('owned-query', 40)] * 3)
        self.assertEqual(len(self.calls), 2)

    def test_second_banner_failure_stops_without_a_third_query(self):
        with self.assertRaises(self.r.ReadinessTimeout):
            self.query([self.result(255, '', 'Connection timed out during banner exchange')] * 3)
        self.assertEqual(len(self.calls), 2)

    def test_unknown_command_failure_is_not_retried(self):
        with self.assertRaises(self.r.CommandFailed):
            self.query([self.result(13), self.result()])
        self.assertEqual(len(self.calls), 1)

    def test_unknown_ssh_denial_is_not_a_banner_retry(self):
        with self.assertRaises(self.r.CommandFailed):
            self.query([self.result(255, '', 'Permission denied (publickey)'), self.result()])
        self.assertEqual(len(self.calls), 1)

    def test_outer_timeout_output_is_not_success(self):
        with self.assertRaises(self.r.CommandFailed):
            self.query([self.result(124), self.result(0, 'NOT_READY\n')])

    def test_partial_banner_output_is_not_success(self):
        with self.assertRaises(self.r.CommandFailed):
            self.query([self.result(255, 'KF_READY\n', 'banner exchange timed out'), self.result(0, '')])

    def test_retry_uses_remaining_budget_without_reset(self):
        left = [500]
        def timeout():
            left[0] = 7
            raise subprocess.TimeoutExpired('owned-query', 40)
        self.query([timeout, self.result()], remaining=lambda: left[0])
        self.assertEqual(self.calls, [40, 7])

    def test_actual_result_after_stage_deadline_is_refused(self):
        left = [1]
        def late():
            left[0] = 0
            return self.result()
        with self.assertRaises(self.r.BudgetExpired):
            self.query([late], remaining=lambda: left[0])
        self.assertEqual(len(self.calls), 1)

    def test_second_actual_result_after_stage_deadline_is_refused(self):
        left = [50]
        def late():
            left[0] = 0
            return self.result()
        with self.assertRaises(self.r.BudgetExpired):
            self.query([subprocess.TimeoutExpired('owned-query', 40), late], remaining=lambda: left[0])
        self.assertEqual(len(self.calls), 2)


class SingleAttempt(Scaling):
    def test_enable_timeout_is_never_retried(self):
        calls = []
        def operation(limit):
            calls.append(limit)
            raise subprocess.TimeoutExpired('owned-enable', limit)
        with self.assertRaises(self.r.ReadinessTimeout):
            self.r.run_once(operation, profile=self.profile(), nominal=20, ceiling=300, remaining=lambda: 500)
        self.assertEqual(calls, [80])

    def test_installer_outer_timeout_is_never_retried(self):
        calls = []
        def operation(limit):
            calls.append(limit)
            return subprocess.CompletedProcess(['owned-install'], 124, 'PARTIAL', '')
        with self.assertRaises(self.r.ReadinessTimeout):
            self.r.run_once(operation, profile=self.profile(), nominal=30, ceiling=300, remaining=lambda: 500)
        self.assertEqual(calls, [120])


class SshWiring(unittest.TestCase):
    def connection_timeout(self, seconds=None):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d); fake = root / 'ssh'; observed = root / 'args'
            fake.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$S4_CAPTURE_ARGS"\nprintf "KF_SSH_UP\\n"\n')
            fake.chmod(0o755)
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ['PATH'],
                       S4_CAPTURE_ARGS=str(observed), WINVM_ROOT=str(root / 'vm'))
            if seconds is not None:
                env['WINVM_SSH_CONNECT_TIMEOUT'] = str(seconds)
            p = subprocess.run(['bash', str(WINDOWS / 'win_vm.sh'), 'ssh', 'echo KF_SSH_UP'],
                               env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(p.returncode, 0, p.stderr)
            return observed.read_text().splitlines()

    def test_default_ssh_policy_remains_eight(self):
        self.assertIn('ConnectTimeout=8', self.connection_timeout())

    def test_fourfold_profile_reaches_inner_ssh(self):
        self.assertIn('ConnectTimeout=80', self.connection_timeout(80))

    def test_slower_profile_reaches_inner_ssh(self):
        self.assertIn('ConnectTimeout=160', self.connection_timeout(160))


class CliEvidence(unittest.TestCase):
    def invoke(self, mode):
        r = importlib.import_module('readiness')
        with tempfile.TemporaryDirectory() as d:
            root = Path(d); profile = root / 'profile.json'; stage = root / 'stage.json'
            receipt = root / 'receipt.json'; count = root / 'count'; command = root / 'query.py'
            value = r.make_profile(60, 2, box_id=1234, source='a' * 40)
            profile.write_text(json.dumps(value))
            import time
            stage.write_text(json.dumps(dict(profile_sha256=r.digest(value), box_id=1234,
                                             end_monotonic=time.monotonic() + 60)))
            command.write_text("from pathlib import Path\nimport sys\np=Path(sys.argv[1]);n=int(p.read_text())+1 if p.exists() else 1;p.write_text(str(n))\nprint('PARTIAL' if n==1 else 'ACTUAL')\nprint('KF_READY')\nraise SystemExit(124 if n==1 else 0)\n")
            p = subprocess.run([sys.executable, str(WINDOWS / 'readiness.py'), mode,
                                '--profile', str(profile), '--stage', str(stage),
                                '--nominal', '5', '--ceiling', '30', '--marker', 'KF_READY',
                                '--receipt', str(receipt), '--', sys.executable, str(command), str(count)],
                               capture_output=True, text=True, timeout=20)
            return p, int(count.read_text()), json.loads(receipt.read_text())

    def test_real_readonly_retry_discards_partial_command_output(self):
        p, count, receipt = self.invoke('query')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(count, 2)
        self.assertNotIn('PARTIAL', p.stdout)
        self.assertIn('ACTUAL', p.stdout)
        self.assertEqual([x['returncode'] for x in receipt['attempts']], [124, 0])

    def test_real_side_effect_timeout_does_not_retry_or_publish_partial(self):
        p, count, receipt = self.invoke('once')
        self.assertEqual(p.returncode, 75, p.stderr)
        self.assertEqual(count, 1)
        self.assertNotIn('PARTIAL', p.stdout)
        self.assertNotIn('KF_READY', p.stdout)
        self.assertEqual(receipt['result'], 'READINESS_INCONCLUSIVE')


if __name__ == '__main__':
    unittest.main()
