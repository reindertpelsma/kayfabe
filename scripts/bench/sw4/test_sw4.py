"""SW4 authorization and full lifecycle must fail closed with fake providers."""

import datetime as dt
import hashlib
from pathlib import Path
import tempfile
import unittest

import round_budget as B
import die_binding as D
import offer_selector as O

NOW = dt.datetime(2026, 10, 24, 23, 40, tzinfo=dt.timezone.utc)


def offer(**changes):
    return {
        "id": 57427095,
        "machine_id": 99805,
        "host_id": 70221,
        "verification": "verified",
        "vms_enabled": True,
        "is_vm_deverified": False,
        "num_gpus": 1,
        "geolocation": "Quebec, CA",
        "dph_total": 0.34,
        "inet_down_cost": 0.033,
        "inet_up_cost": 0.033,
        **changes,
    }


class Sw4Scope(unittest.TestCase):
    def test_only_authorized_blackwell_dies(self):
        self.assertEqual(D.SCOPE, {"GB206", "GB207"})

    def test_previous_scope_not_inherited(self):
        self.assertFalse(D.SCOPE & {"GA104", "GA106", "GA102", "AD106"})

    def test_machine_blacklist(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(), {99805})

    def test_host_blacklist(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(), {70221})

    def test_host_blacklist_second_offer(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(id=50694293, machine_id=45428), {70221})

    def test_country_cn_refused(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(geolocation="Beijing, CN"), set())

    def test_country_missing_refused(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(geolocation=None), set())

    def test_ungranted_machine_refused(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(machine_id=34087), set())

    def test_wrong_host_refused(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(host_id=42), set())

    def test_expensive_requested_disk_rate_refused(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(dph_total=0.601), set())

    def test_nonfinite_requested_rate_refused(self):
        with self.assertRaises(ValueError):
            O.require_offer(offer(dph_total=float("nan")), set())


class Sw4Budget(unittest.TestCase):
    def test_fixed_two_hour_ceiling(self):
        self.assertLessEqual(B.plan(0.1, 23, NOW)["seconds"], 7200)

    def test_lower_stop_and_cap(self):
        p = B.plan(0.34, 23, NOW)
        self.assertEqual((p["stop_usd"], p["cap_usd"]), (1.5, 2))

    def test_rate_limit(self):
        with self.assertRaises(ValueError):
            B.plan(0.61, 23, NOW)

    def test_rate_limit_another_price(self):
        with self.assertRaises(ValueError):
            B.plan(1.0, 23, NOW)

    def test_transfer_explicit(self):
        p = B.plan(0.34, 23, NOW, transfer_usd=0.8)
        self.assertEqual(p["transfer_usd"], 0.8)
        self.assertLess(p["reservation_usd"], 1.5)

    def test_transfer_shortens_deadline(self):
        low = B.plan(0.34, 23, NOW, transfer_usd=0.1)
        high = B.plan(0.34, 23, NOW, transfer_usd=0.8)
        self.assertLess(high["seconds"], low["seconds"])

    def test_retained_margin_separate_from_transfer(self):
        p = B.plan(0.34, 23, NOW, transfer_usd=0.4)
        self.assertGreaterEqual(p["nonruntime_margin_usd"], 0.5)
        self.assertGreaterEqual(p["reservation_usd"], 0.9)

    def test_transfer_can_refuse_before_create(self):
        with self.assertRaises(ValueError):
            B.plan(0.34, 23, NOW, transfer_usd=1.01)


class Sw4Lifecycle(unittest.TestCase):
    def lifecycle(self, **changes):
        import lifecycle as L

        events = []
        payload = b"private text capture"
        state = {"absent": False, "receipt_hash": hashlib.sha256(payload).hexdigest()}
        state.update(changes)

        class Fake:
            def step(self, name, value=None):
                events.append((name, value))

            def arm(self, plan):
                self.step("arm", plan["deadline_utc"])
                return {"state": "ARMED", "pid": 99, "deadline_utc": plan["deadline_utc"]}

            def create(self):
                self.step("create", 123456)
                return 123456

            def report(self, row):
                self.step("report", dict(row))

            def ready(self, iid):
                self.step("ready", iid)
                if state.get("identity_refused"):
                    raise ValueError("real identity refusal")

            def run(self, iid):
                self.step("run", iid)
                if state.get("native_failed"):
                    raise ValueError("native failure; guest excluded")

            def capture(self, iid):
                self.step("capture", iid)
                return payload, state["receipt_hash"]

            def destroy(self, iid):
                self.step("destroy", iid)
                state["absent"] = not state.get("absence_unknown", False)

            def absent(self, iid):
                self.step("absence", iid)
                return state["absent"]

            def cancel(self, watch):
                self.step("cancel", watch["pid"])

        plan = B.plan(0.34, 23, NOW)
        try:
            result = L.execute(Fake(), plan)
            return events, result
        except Exception as error:
            return events, error

    def test_complete_fake_lifecycle(self):
        events, result = self.lifecycle()
        self.assertIsInstance(result, dict)
        self.assertEqual([x[0] for x in events], [
            "arm", "report", "create", "report", "ready", "run", "capture",
            "report", "destroy", "absence", "cancel", "report",
        ])

    def test_arm_precedes_create(self):
        events, result = self.lifecycle()
        self.assertLess([x[0] for x in events].index("arm"),
                        [x[0] for x in events].index("create"))

    def test_capture_hash_failure_still_tears_down(self):
        events, result = self.lifecycle(receipt_hash="wrong")
        self.assertIsInstance(result, Exception)
        self.assertIn(("destroy", 123456), events)
        self.assertNotIn(("cancel", 99), events)

    def test_native_refusal_no_guest_retry(self):
        events, result = self.lifecycle(native_failed=True)
        self.assertIsInstance(result, Exception)
        self.assertEqual(sum(x[0] == "create" for x in events), 1)
        self.assertIn(("destroy", 123456), events)

    def test_identity_refusal_no_run(self):
        events, result = self.lifecycle(identity_refused=True)
        self.assertIsInstance(result, Exception)
        self.assertFalse(any(x[0] == "run" for x in events))
        self.assertIn(("destroy", 123456), events)

    def test_absence_unknown_keeps_timer(self):
        events, result = self.lifecycle(absence_unknown=True)
        self.assertIsInstance(result, Exception)
        self.assertNotIn(("cancel", 99), events)

    def test_cancel_after_api_absence(self):
        events, result = self.lifecycle()
        names = [x[0] for x in events]
        self.assertLess(names.index("absence"), names.index("cancel"))

    def test_hash_verified_before_destroy(self):
        events, result = self.lifecycle()
        reports = [x[1] for x in events if x[0] == "report"]
        self.assertTrue(reports[-2]["capture_verified"])

    def test_source_lock_good(self):
        import lifecycle as L

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "source"
            path.write_bytes(b"pinned public source")
            L.source_lock({str(path): hashlib.sha256(path.read_bytes()).hexdigest()})

    def test_source_lock_changed(self):
        import lifecycle as L

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "source"
            path.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                L.source_lock({str(path): "wrong"})

    def test_source_lock_missing(self):
        import lifecycle as L

        with self.assertRaises((ValueError, FileNotFoundError)):
            L.source_lock({"/does/not/exist": "wrong"})


if __name__ == "__main__":
    unittest.main()
