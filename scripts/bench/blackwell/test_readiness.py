"""Bench readiness must adapt to a measured box, without changing fault admission."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest

try:
    import readiness as R
except ImportError:
    R = None


class AdaptiveReadiness(unittest.TestCase):
    def policy(self, seconds):
        self.assertIsNotNone(R, "adaptive readiness policy is missing")
        return R.policy(seconds)

    def test_fast_box_still_gets_four_times_first_query(self):
        self.assertEqual(self.policy(0.1)["first_query"], 40)

    def test_nominal_box_gets_four_times_first_query(self):
        self.assertEqual(self.policy(0.5)["factor"], 4)

    def test_probe_latency_scales_first_query(self):
        self.assertEqual(self.policy(3)["first_query"], 60)

    def test_probe_latency_scales_banner_deadline(self):
        self.assertEqual(self.policy(3)["banner_query"], 100)

    def test_slow_box_factor_has_ten_times_ceiling(self):
        self.assertEqual(self.policy(90)["factor"], 10)

    def test_slow_box_first_query_is_bounded(self):
        self.assertEqual(self.policy(90)["first_query"], 100)

    def test_bad_probe_duration_is_refused(self):
        for value in [0, -1, float("inf"), float("nan"), True]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.policy(value)

    def test_handshake_budget_fits_first_query_and_one_retry(self):
        p = self.policy(5)
        self.assertGreaterEqual(p["handshake"], 2 * p["first_query"] + 2)
        self.assertLessEqual(p["handshake"], 220)

    def test_stage_and_total_budget_have_absolute_ceilings(self):
        p = self.policy(90)
        self.assertEqual(p["observer_total"], 360)
        self.assertEqual(p["readiness_total"], 900)
        self.assertLessEqual(p["verify_stage"], 240)
        self.assertLessEqual(p["ssh_stage"], 600)

    def test_timeout_uses_remaining_stage_budget(self):
        self.policy(1)
        clock = [0]
        b = R.Budget(30, 100, clock=lambda: clock[0])
        clock[0] = 28
        self.assertEqual(b.timeout(100), 2)

    def test_timeout_uses_remaining_total_budget(self):
        self.policy(1)
        clock = [0]
        b = R.Budget(100, 20, clock=lambda: clock[0])
        clock[0] = 19
        self.assertEqual(b.timeout(100), 1)

    def test_deadline_does_not_restart_after_a_query(self):
        self.policy(1)
        clock = [0]
        b = R.Budget(30, 100, clock=lambda: clock[0])
        clock[0] = 30
        with self.assertRaises(TimeoutError):
            b.timeout(100)

    def test_late_success_does_not_become_readiness(self):
        self.policy(1)
        clock = [0]

        def fetch(timeout):
            clock[0] = 31
            return "actual"

        with self.assertRaises(TimeoutError):
            R.first_query(fetch, R.Budget(30, 100, clock=lambda: clock[0]), 40)

    def test_first_missing_query_is_retried_once(self):
        self.policy(1)
        calls = []

        def fetch(timeout):
            calls.append(timeout)
            if len(calls) == 1:
                raise subprocess.TimeoutExpired("fixture", timeout, output="partial")
            return "actual"

        self.assertEqual(R.first_query(fetch, R.Budget(100, 100), 40), "actual")
        self.assertEqual(len(calls), 2)

    def test_second_missing_query_stops(self):
        self.policy(1)
        calls = []

        def fetch(timeout):
            calls.append(timeout)
            raise subprocess.TimeoutExpired("fixture", timeout)

        with self.assertRaises(subprocess.TimeoutExpired):
            R.first_query(fetch, R.Budget(300, 900), 40)
        self.assertEqual(len(calls), 2)

    def check_reserved_retry(self, seconds):
        self.policy(1)
        clock = [0]
        calls = []

        def fetch(timeout):
            calls.append(timeout)
            if len(calls) == 1:
                clock[0] += timeout
                raise subprocess.TimeoutExpired("fixture", timeout)
            return "actual"

        result = R.first_query(fetch, R.Budget(seconds, 900, clock=lambda: clock[0]), 720)
        self.assertEqual(result, "actual")
        self.assertEqual(len(calls), 2)
        self.assertLessEqual(sum(calls), seconds)

    def test_verify_query_reserves_time_for_its_one_retry(self):
        self.check_reserved_retry(240)

    def test_short_remaining_budget_also_reserves_a_retry(self):
        self.check_reserved_retry(30)

    def test_unknown_failure_is_not_retried(self):
        self.policy(1)
        calls = []

        def fetch(timeout):
            calls.append(timeout)
            raise ValueError("bad owner")

        with self.assertRaisesRegex(ValueError, "bad owner"):
            R.first_query(fetch, R.Budget(300, 900), 40)
        self.assertEqual(len(calls), 1)

    def test_calibration_is_frozen_for_this_box(self):
        self.policy(1)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "calibration.json"
            first = R.calibration(path, 42, lambda: 3)

            def forbidden():
                raise AssertionError("calibration repeated")

            self.assertEqual(R.calibration(path, 42, forbidden), first)
            self.assertEqual(json.loads(path.read_text())["probe_seconds"], 3)

    def test_other_box_cannot_reuse_the_calibration(self):
        self.policy(1)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "calibration.json"
            R.calibration(path, 42, lambda: 3)
            with self.assertRaisesRegex(ValueError, "box"):
                R.calibration(path, 43, lambda: 3)

    def test_total_exhaustion_stops_even_with_stage_time_left(self):
        self.policy(1)
        clock = [0]
        b = R.Budget(100, 20, clock=lambda: clock[0])
        clock[0] = 20
        with self.assertRaises(TimeoutError):
            b.timeout(40)

    def test_foreign_box_calibration_never_calls_a_new_probe(self):
        self.policy(1)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "calibration.json"
            R.calibration(path, 42, lambda: 3)

            def forbidden():
                raise AssertionError("foreign calibration silently refreshed")

            with self.assertRaises(ValueError):
                R.calibration(path, 43, forbidden)

    def test_tampered_policy_cannot_supply_larger_deadlines(self):
        self.policy(1)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "calibration.json"
            R.calibration(path, 42, lambda: 3)
            row = json.loads(path.read_text())
            row["policy"]["first_query"] = 9999
            path.write_text(json.dumps(row))
            with self.assertRaises(ValueError):
                R.calibration(path, 42, lambda: 3)


if __name__ == "__main__":
    unittest.main()
