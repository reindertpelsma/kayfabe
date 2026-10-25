"""Synthetic grant5 budget propagation; no provider or hardware access."""

import datetime as dt
import unittest
from execution import require_offer, fit_plan
from test_execution import offer


class Amendment(unittest.TestCase):
    def plan(self, rate=1.09, **kw):
        now = dt.datetime(2026, 10, 25, 19, tzinfo=dt.timezone.utc)
        args = dict(hourly=rate, credit=32, now=now, transfer=.16927083333333334,
                    deadline=now + dt.timedelta(hours=3), estimated_seconds=4931,
                    shared_remaining=5.28595340625, grant="grant5")
        args.update(kw)
        return fit_plan(**args)

    def test_amended_offer(self):
        row = offer(); row.update(dph_base=1.04, storage_cost=.2)
        self.assertAlmostEqual(require_offer(row, set(), grant="grant5"), 1.09)

    def test_second_amended_offer(self):
        row = offer(); row.update(dph_base=1.2533333333333332, storage_cost=1/6)
        self.assertAlmostEqual(require_offer(row, set(), grant="grant5"), 1.295)

    def test_amended_plan(self):
        p = self.plan()
        self.assertEqual((p['stop_usd'], p['cap_usd'], p['rate_ceiling']), (3, 3.5, 1.4))
        self.assertGreaterEqual(p['seconds'], 4931)

    def test_second_amended_plan(self):
        self.assertGreaterEqual(self.plan(1.295)['seconds'], 4931)

    def test_amended_rate_refused(self):
        with self.assertRaises(ValueError): self.plan(1.401)

    def test_second_amended_rate_refused(self):
        with self.assertRaises(ValueError): self.plan(1.5)

    def test_unknown_grant(self):
        with self.assertRaises(ValueError): self.plan(grant="other")

    def test_unknown_offer_grant(self):
        with self.assertRaises(ValueError): require_offer(offer(), set(), grant="other")

    def test_daily_remaining_at_stop(self):
        with self.assertRaises(ValueError): self.plan(shared_remaining=3)

    def test_daily_remaining_below_stop(self):
        with self.assertRaises(ValueError): self.plan(shared_remaining=2.9)

    def test_amended_fit_still_required(self):
        with self.assertRaises(ValueError): self.plan(estimated_seconds=7000)

    def test_amended_transfer_still_counted(self):
        with self.assertRaises(ValueError): self.plan(transfer=2)

    def test_actual_amended_rate(self):
        from execution import actual_rate
        self.assertEqual(actual_rate(1.09, self.plan()), 1.09)

    def test_second_actual_amended_rate(self):
        from execution import actual_rate
        self.assertEqual(actual_rate(1.295, self.plan(1.295)), 1.295)

    def test_actual_over_ceiling(self):
        from execution import actual_rate
        with self.assertRaises(ValueError): actual_rate(1.41, self.plan(1.295))

    def test_actual_over_reservation(self):
        from execution import actual_rate
        with self.assertRaises(ValueError): actual_rate(1.3, self.plan())

    def test_ready_rate(self):
        from execution import ready_budget
        self.assertEqual(ready_budget(self.plan()).MAX_RATE, 1.4)

    def test_ready_stop(self):
        from execution import ready_budget
        self.assertEqual(ready_budget(self.plan()).STOP_USD, 3)

    def test_authority_required(self):
        from execution import config_grant
        with self.assertRaises(ValueError): config_grant({'grant':'grant5','grant_locks':{}})

    def test_second_authority_required(self):
        from execution import config_grant
        with self.assertRaises(ValueError): config_grant({'grant':'grant5'})
