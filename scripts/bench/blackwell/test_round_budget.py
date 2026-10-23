"""Round B1 cost bounds are fixed before allocation, never extended by readiness."""

import datetime as dt
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    "round_budget", Path(__file__).with_name("round_budget.py")
)
B = None
if Path(SPEC.origin).exists():
    B = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(B)

NOW = dt.datetime(2026, 10, 23, 16, 30, tzinfo=dt.timezone.utc)


class RoundBudget(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(B, "round cost planner not implemented")

    def plan(self, rate=0.4, credit=28.8):
        return B.plan(rate, credit, NOW)

    def test_cheap_lease_retains_150_minute_ceiling(self):
        row = self.plan()
        self.assertEqual(row["seconds"], 9000)
        self.assertEqual(row["deadline_utc"], "2026-10-23T19:00:00+00:00")

    def test_rate_bound_includes_at_least_ten_cents_per_hour(self):
        self.assertAlmostEqual(self.plan()["rate_bound"], 0.5)

    def test_rate_bound_also_covers_fifteen_percent(self):
        self.assertAlmostEqual(self.plan(2)["rate_bound"], 2.3)

    def test_high_rate_shortens_cost_deadline_before_stop(self):
        row = self.plan(2.5)
        self.assertLess(row["seconds"], 2500)
        self.assertLess(row["reservation_usd"], 2.5)

    def test_every_accepted_rate_keeps_stop_and_hard_cap(self):
        for rate in (0.01, 0.4, 0.8, 1.1, 2, 2.5):
            row = self.plan(rate)
            self.assertLess(row["reservation_usd"], row["stop_usd"])
            self.assertEqual(row["stop_usd"], 2.5)
            self.assertEqual(row["cap_usd"], 3.0)
            self.assertEqual(row["reserve_usd"], 8.0)
            self.assertLessEqual(row["seconds"], 9000)

    def test_reservation_includes_fifty_cent_nonruntime_margin(self):
        row = self.plan()
        self.assertAlmostEqual(row["reservation_usd"], 1.75)

    def test_teardown_has_one_minute_before_cost_stop(self):
        row = self.plan(2)
        self.assertLessEqual(row["seconds"] + 60, int((2.5 - 0.5) * 3600 / row["rate_bound"]))

    def test_reserve_is_checked_before_any_allocation(self):
        for credit in (0, 7.99, 8, 9, 9.74):
            with self.assertRaises(ValueError):
                self.plan(credit=credit)

    def test_reserve_boundary_passes_only_when_exposure_is_covered(self):
        row = self.plan(credit=9.75)
        self.assertEqual(row["credit_after_reservation"], 8.0)

    def test_high_price_reservation_preserves_the_same_account_reserve(self):
        with self.assertRaises(ValueError):
            self.plan(rate=2.5, credit=10.4)

    def test_invalid_rate_never_creates_a_plan(self):
        for rate in (0, -1, 2.50001, float("inf"), float("nan"), True, "0.4"):
            with self.assertRaises(ValueError):
                self.plan(rate)

    def test_invalid_credit_never_creates_a_plan(self):
        for credit in (-1, float("inf"), float("nan"), True, "28.8"):
            with self.assertRaises(ValueError):
                self.plan(credit=credit)

    def test_rate_above_owner_limit_refuses_even_with_plenty_of_credit(self):
        with self.assertRaises(ValueError):
            self.plan(rate=3, credit=100)

    def test_naive_arm_time_is_refused(self):
        with self.assertRaises(ValueError):
            B.plan(0.4, 28.8, NOW.replace(tzinfo=None))

    def test_delay_after_create_cannot_restart_original_deadline(self):
        row = self.plan()
        self.assertEqual(B.remaining(row, NOW + dt.timedelta(minutes=40)), 6600)
        self.assertEqual(row["deadline_utc"], "2026-10-23T19:00:00+00:00")

    def test_expired_deadline_never_gains_time(self):
        row = self.plan()
        for minutes in (150, 151, 200):
            self.assertEqual(B.remaining(row, NOW + dt.timedelta(minutes=minutes)), 0)

    def test_readiness_cannot_replace_fixed_deadline_with_a_later_one(self):
        row = self.plan()
        row["deadline_utc"] = (NOW + dt.timedelta(minutes=170)).isoformat()
        with self.assertRaises(ValueError):
            B.remaining(row, NOW)

    def test_clock_before_watchdog_arm_is_refused(self):
        with self.assertRaises(ValueError):
            B.remaining(self.plan(), NOW - dt.timedelta(seconds=1))


if __name__ == "__main__":
    unittest.main()
