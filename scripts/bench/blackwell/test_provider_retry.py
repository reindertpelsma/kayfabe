"""Provider retry is bounded, pre-SSH only, and retires exact own IDs first."""

import json
from pathlib import Path
import tempfile
import unittest

try:
    import provider_retry as P
except ImportError:
    P = None


class ProviderRetry(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(P, "provider retry policy missing")
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.bad = Path(self.temp.name) / "known_bad.json"
        self.state = Path(self.temp.name) / "stretch.json"
        self.row = {"id": 71, "machine_id": 15, "auto_destroy_verified": True}
        self.events = []

    def test_memory_timeout_is_infrastructure(self):
        self.assertEqual(
            P.failure({"status_msg": "Error: timed out waiting for memory to free"}, 10),
            "provider_host_error",
        )

    def test_gpu_startup_error_is_infrastructure(self):
        self.assertEqual(
            P.failure({"status_msg": "GPU error, unable to start instance"}, 10),
            "provider_host_error",
        )

    def test_provider_failed_state_is_infrastructure(self):
        self.assertEqual(P.failure({"actual_status": "failed"}, 10), "provider_host_error")

    def test_loading_inside_bound_is_not_a_failure(self):
        self.assertIsNone(P.failure({"actual_status": "loading"}, 500))

    def test_cold_start_has_an_absolute_twenty_minute_ceiling(self):
        self.assertEqual(P.boot_bound(None), 1200)
        self.assertEqual(P.failure({}, 1200), "provider_boot_timeout")

    def test_fast_banner_calibration_has_a_ten_minute_floor(self):
        self.assertEqual(P.boot_bound(0.5), 600)
        self.assertEqual(P.failure({}, 600, banner_seconds=0.5), "provider_boot_timeout")

    def test_slow_banner_scales_without_an_unbounded_boot(self):
        self.assertEqual(P.boot_bound(3), 720)
        self.assertEqual(P.boot_bound(100), 1200)

    def test_successful_ssh_is_not_a_provider_retry(self):
        self.assertIsNone(P.failure({"status_msg": "Error: old message"}, 1300, ssh_ready=True))

    def test_started_native_or_guest_work_is_never_retryable(self):
        self.assertIsNone(P.failure({"status_msg": "Error: host message"}, 1300, work_started=True))

    def retire(self, **kw):
        return P.retire(
            kw.pop("row", self.row),
            {71},
            "provider_host_error",
            self.bad,
            lambda iid: self.events.append(("destroy", iid)),
            kw.pop("absent", lambda iid: self.events.append(("absence", iid)) or True),
            "2026-10-23T01:15:00+00:00",
            **kw,
        )

    def test_destroy_exact_id_precedes_bad_machine_publication(self):
        self.retire()
        self.assertEqual(self.events, [("destroy", 71), ("absence", 71)])
        self.assertEqual(P.bad_machines(self.bad), {15})

    def test_api_presence_prevents_retry_and_bad_list_publication(self):
        with self.assertRaisesRegex(ValueError, "absence"):
            self.retire(absent=lambda _: False)
        self.assertFalse(self.bad.exists())

    def test_a_foreign_instance_cannot_be_destroyed(self):
        with self.assertRaises(ValueError):
            self.retire(row=dict(self.row, id=72))
        self.assertEqual(self.events, [])

    def test_an_ssh_ready_instance_cannot_enter_provider_retry(self):
        with self.assertRaises(ValueError):
            self.retire(row=dict(self.row, ssh_ready_utc="actual"))
        self.assertEqual(self.events, [])

    def test_a_started_native_instance_cannot_enter_provider_retry(self):
        with self.assertRaises(ValueError):
            self.retire(row=dict(self.row, work_started=True))
        self.assertEqual(self.events, [])

    def test_unknown_failure_class_is_not_retired_as_infra(self):
        with self.assertRaises(ValueError):
            P.retire(
                self.row,
                {71},
                "native_xid32",
                self.bad,
                lambda _: self.fail(),
                lambda _: True,
                "now",
            )

    def test_a_bad_machine_is_excluded_even_with_another_offer_id(self):
        self.retire()
        rows = [{"id": 1, "machine_id": 15}, {"id": 2, "machine_id": 16}]
        self.assertEqual(P.exclude_bad(rows, self.bad), [rows[1]])

    def test_all_offers_on_a_bad_machine_are_excluded(self):
        self.retire()
        self.assertEqual(P.exclude_bad([{"machine_id": 15}, {"machine_id": 15}], self.bad), [])

    def test_first_and_second_retry_are_persistently_charged(self):
        self.assertEqual(P.reserve_retry(self.state), 1)
        self.assertEqual(P.reserve_retry(self.state), 2)
        self.assertEqual(json.loads(self.state.read_text())["retries_used"], 2)

    def test_third_retry_is_refused(self):
        P.reserve_retry(self.state)
        P.reserve_retry(self.state)
        with self.assertRaisesRegex(ValueError, "retry"):
            P.reserve_retry(self.state)

    def test_restarting_the_controller_cannot_reset_the_retry_cap(self):
        self.state.write_text('{"retries_used":2}\n')
        with self.assertRaises(ValueError):
            P.reserve_retry(self.state)
        self.assertEqual(json.loads(self.state.read_text())["retries_used"], 2)


if __name__ == "__main__":
    unittest.main()
