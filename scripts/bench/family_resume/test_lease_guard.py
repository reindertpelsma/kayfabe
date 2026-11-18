"""Capture precedes teardown; account-wide debits cannot impersonate lease spend."""

import datetime as dt
import unittest

import lease_guard as G


class GuardTests(unittest.TestCase):
    def test_transfer_is_reserved_even_before_runtime(self):
        plan = dict(
            armed_utc="2026-10-28T00:00:00+00:00",
            rate_bound=0.6,
            transfer_usd=0.2,
            nonruntime_margin_usd=0.5,
        )
        now = dt.datetime.fromisoformat(plan["armed_utc"])
        self.assertAlmostEqual(G.own_cost(plan, now), 0.7)

    def test_elapsed_runtime_uses_reserved_rate_without_account_credit(self):
        plan = dict(
            armed_utc="2026-10-28T00:00:00+00:00",
            rate_bound=0.6,
            transfer_usd=0.2,
            nonruntime_margin_usd=0.5,
            credit_before=40,
        )
        now = dt.datetime.fromisoformat(plan["armed_utc"]) + dt.timedelta(hours=1)
        self.assertAlmostEqual(G.own_cost(plan, now), 1.3)
        plan["credit_before"] = 20
        self.assertAlmostEqual(G.own_cost(plan, now), 1.3)

    def retire(self, capture):
        events = []

        def pull(iid):
            events.append(("capture", iid))
            return capture()

        result = G.retire(
            42,
            capture=pull,
            destroy=lambda iid: events.append(("destroy", iid)),
            status=lambda iid: dict(id=iid, absent=True),
            pause=lambda _: None,
        )
        return result, events

    def test_capture_is_verified_before_exact_destroy(self):
        result, events = self.retire(lambda: dict(id=42, capture_verified=True))
        self.assertEqual(events, [("capture", 42), ("destroy", 42)])
        self.assertTrue(result["capture_verified"])
        self.assertTrue(result["absence_verified"])

    def test_failed_pull_still_tears_down_and_never_claims_capture(self):
        def fail():
            raise OSError("network")

        result, events = self.retire(fail)
        self.assertEqual(events, [("capture", 42), ("destroy", 42)])
        self.assertFalse(result["capture_verified"])
        self.assertEqual(result["capture_error"], "OSError")

    def test_wrong_capture_id_is_not_a_verified_final_capture(self):
        result, _ = self.retire(lambda: dict(id=43, capture_verified=True))
        self.assertFalse(result["capture_verified"])
        self.assertTrue(result["absence_verified"])

    def test_wrong_absence_id_never_completes(self):
        with self.assertRaises(RuntimeError):
            G.retire(
                42,
                capture=lambda _: dict(id=42, capture_verified=True),
                destroy=lambda _: None,
                status=lambda _: dict(id=43, absent=True),
                pause=lambda _: None,
            )


if __name__ == "__main__":
    unittest.main()
