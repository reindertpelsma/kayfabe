"""Transfer and billed-credit stops supplement the fixed precreate time bound."""

import datetime as dt
import unittest

import precreate_watchdog as W
import round_budget as B

NOW = dt.datetime(2026, 10, 24, 23, 40, tzinfo=dt.timezone.utc)


class CostWatch(unittest.TestCase):
    def test_network_download_stop(self):
        import network_guard as N

        self.assertTrue(N.exceeded(12_000_000_001, 0))

    def test_network_download_boundary(self):
        import network_guard as N

        self.assertTrue(N.exceeded(12_000_000_000, 0))

    def test_network_upload_stop(self):
        import network_guard as N

        self.assertTrue(N.exceeded(0, 1_000_000_001))

    def test_network_upload_boundary(self):
        import network_guard as N

        self.assertTrue(N.exceeded(0, 1_000_000_000))

    def test_counter_reset_refused(self):
        import network_guard as N

        with self.assertRaises(ValueError):
            N.exceeded(-1, 0)

    def test_other_counter_reset_refused(self):
        import network_guard as N

        with self.assertRaises(ValueError):
            N.exceeded(0, -1)

    def test_normal_transfer_within_allocation(self):
        import network_guard as N

        self.assertFalse(N.exceeded(8_000_000_000, 500_000_000))

    def check_credit(self, credit):
        plan = B.plan(0.34, 23, NOW)
        events = []
        clock = [NOW]

        def status(iid):
            return {"id": iid, "absent": bool(events), "rate": 0.34, "credit": credit}

        def pause(seconds):
            clock[0] += dt.timedelta(seconds=seconds)

        result = W.watch(
            plan, ledger=lambda: [{"id": 123456}], ready=lambda row: None,
            now=lambda: clock[0], pause=pause, status=status,
            destroy=lambda iid: events.append((iid, clock[0])),
        )
        self.assertEqual(result["reason"], "billed allocation stop")
        self.assertEqual(events, [(123456, NOW)])

    def test_credit_delta_plus_margin_stops(self):
        self.check_credit(21.99)

    def test_credit_delta_another_stop(self):
        self.check_credit(21.5)


if __name__ == "__main__":
    unittest.main()
