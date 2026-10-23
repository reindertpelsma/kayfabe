"""Owner-authorized bench deadlines: calibration, finite budgets and actual replies."""
import importlib
import json
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
        left = iter([500, 500, 7, 6])
        self.query([subprocess.TimeoutExpired('owned-query', 40), self.result()], remaining=lambda: next(left))
        self.assertEqual(self.calls, [40, 7])

    def test_actual_result_after_stage_deadline_is_refused(self):
        left = iter([1, 0])
        with self.assertRaises(self.r.BudgetExpired):
            self.query([self.result()], remaining=lambda: next(left))

    def test_second_actual_result_after_stage_deadline_is_refused(self):
        left = iter([50, 40, 1, 0])
        with self.assertRaises(self.r.BudgetExpired):
            self.query([subprocess.TimeoutExpired('owned-query', 40), self.result()], remaining=lambda: next(left))


if __name__ == '__main__':
    unittest.main()
