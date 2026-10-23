"""The watchdog signals armed before a contract exists and retires only its bound id."""

import datetime as dt
import importlib.util
from pathlib import Path
import unittest

import round_budget as B

SPEC = importlib.util.spec_from_file_location(
    "precreate_watchdog", Path(__file__).with_name("precreate_watchdog.py")
)
W = None
if Path(SPEC.origin).exists():
    W = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(W)

NOW = dt.datetime(2026, 10, 23, 16, 30, tzinfo=dt.timezone.utc)
OWN = 12345678


class PrecreateWatchdog(unittest.TestCase):
    def run_case(
        self, *, delay=0, rate=0.4, credit=28.8, absent=False, close=False, absence_after=1
    ):
        self.assertIsNotNone(W, "precreate watchdog not implemented")
        events = []
        clock = [NOW]
        row = {"id": OWN, "destroyed": close}

        def ledger():
            events.append(("ledger", clock[0]))
            return [] if (clock[0] - NOW).total_seconds() < delay else [row]

        def pause(seconds):
            clock[0] += dt.timedelta(seconds=seconds)

        def status(iid):
            events.append(("status", iid))
            destroyed = sum(e == ("destroy", iid) for e in events)
            return {
                "id": iid,
                "absent": absent or destroyed >= absence_after,
                "rate": rate,
                "credit": credit,
            }

        result = W.watch(
            B.plan(0.4, 28.8, NOW),
            ledger=ledger,
            ready=lambda plan: events.append(("ready", plan)),
            now=lambda: clock[0],
            pause=pause,
            status=status,
            destroy=lambda iid: events.append(("destroy", iid)),
        )
        return result, events, clock[0]

    def test_ready_is_published_before_first_empty_ledger_read(self):
        result, events, end = self.run_case(delay=40)
        self.assertEqual(events[0][0], "ready")
        self.assertEqual(events[1][0], "ledger")

    def test_ready_is_also_first_when_contract_publication_is_immediate(self):
        result, events, end = self.run_case()
        self.assertEqual(events[0][0], "ready")
        self.assertEqual(events[1][0], "ledger")

    def test_no_provider_query_or_destroy_before_a_contract_is_bound(self):
        result, events, end = self.run_case(delay=40)
        bound = next(i for i, e in enumerate(events) if e[0] == "status")
        self.assertGreaterEqual(sum(e[0] == "ledger" for e in events[:bound]), 3)

    def test_readiness_delay_does_not_extend_the_precreate_deadline(self):
        result, events, end = self.run_case(delay=100)
        self.assertEqual(end, NOW + dt.timedelta(seconds=9000))
        self.assertEqual(result["reason"], "fixed deadline")

    def test_long_readiness_still_uses_the_original_deadline(self):
        result, events, end = self.run_case(delay=1200)
        self.assertEqual(end, NOW + dt.timedelta(seconds=9000))

    def test_destroy_is_by_the_one_exact_owned_id(self):
        result, events, end = self.run_case()
        self.assertEqual([e for e in events if e[0] == "destroy"], [("destroy", OWN)])

    def test_absence_is_queried_after_destroy_before_completion(self):
        result, events, end = self.run_case()
        self.assertEqual(events[-2:], [("destroy", OWN), ("status", OWN)])
        self.assertTrue(result["absence_verified"])

    def test_destroy_acknowledgement_without_absence_is_retried(self):
        result, events, end = self.run_case(absence_after=2)
        self.assertEqual(sum(e[0] == "destroy" for e in events), 2)
        self.assertTrue(result["absence_verified"])

    def test_multiple_still_present_receipts_never_claim_early_absence(self):
        result, events, end = self.run_case(absence_after=3)
        self.assertEqual(sum(e[0] == "destroy" for e in events), 3)
        self.assertTrue(result["absence_verified"])

    def test_rate_beyond_reserved_bound_stops_immediately(self):
        result, events, end = self.run_case(rate=0.51)
        self.assertEqual(end, NOW)
        self.assertEqual(result["reason"], "price beyond reservation")

    def test_rate_above_owner_maximum_is_also_fatal(self):
        result, events, end = self.run_case(rate=2.51)
        self.assertEqual(end, NOW)
        self.assertIn(("destroy", OWN), events)

    def test_credit_below_reserve_stops_immediately(self):
        result, events, end = self.run_case(credit=7.99)
        self.assertEqual(end, NOW)
        self.assertEqual(result["reason"], "account reserve")

    def test_zero_credit_is_a_reserve_stop_not_an_idle_wait(self):
        result, events, end = self.run_case(credit=0)
        self.assertEqual(end, NOW)
        self.assertIn(("destroy", OWN), events)

    def test_already_absent_id_is_recorded_without_another_destroy(self):
        result, events, end = self.run_case(absent=True)
        self.assertFalse(any(e[0] == "destroy" for e in events))
        self.assertTrue(result["absence_verified"])

    def test_closed_ledger_cannot_suppress_destruction_of_a_still_present_id(self):
        result, events, end = self.run_case(close=True)
        self.assertEqual(end, NOW)
        self.assertIn(("destroy", OWN), events)

    def test_closed_ledger_at_reserve_boundary_still_requires_api_absence(self):
        result, events, end = self.run_case(close=True, credit=8)
        self.assertEqual(end, NOW)
        self.assertIn(("destroy", OWN), events)

    def test_no_allocation_before_expiry_means_no_provider_mutation(self):
        result, events, end = self.run_case(delay=10000)
        self.assertFalse(any(e[0] in ("status", "destroy") for e in events))
        self.assertEqual(result["reason"], "no contract before deadline")


if __name__ == "__main__":
    unittest.main()
