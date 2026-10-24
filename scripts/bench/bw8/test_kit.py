"""Synthetic preparation/overlay contracts; no provider, SSH or guest is contacted."""

import copy
from pathlib import Path
import unittest

try:
    from prepare import arm_overlay, inventory, validate_inventory
except ModuleNotFoundError:
    def inventory():
        return {}

    def validate_inventory(plan):
        return False

    def arm_overlay(text):
        return text


HERE = Path(__file__).resolve().parent

try:
    from prepare import verify_process_receipt
except ImportError:
    def verify_process_receipt(baseline, actual):
        return False


def receipt():
    return {"sha256": "05d3d46f4e8429f5ffebacbd7ca9f5156f7d6f1141b4b10ce84732b174347448",
            "kf3_device": "fixed-device-config", "harness_checkout": "fixed-harness",
            "flags": {"KF3_BASELINE": "1"}}


def probe_receipt():
    row = receipt()
    row["flags"]["KF3_COMPLETION_PROBE"] = "1000"
    return row


class KitTests(unittest.TestCase):
    def test_matched_process_receipt(self):
        self.assertTrue(verify_process_receipt(receipt(), probe_receipt()))

    def test_pid_path_changes_do_not_change_configuration(self):
        actual = probe_receipt()
        actual.update(pid=1234, exe="owned-binary")
        self.assertTrue(verify_process_receipt(receipt(), actual))

    def test_unexpected_effective_flag(self):
        actual = probe_receipt()
        actual["flags"]["KF3_OTHER"] = "1"
        self.assertFalse(verify_process_receipt(receipt(), actual))

    def test_baseline_flag_missing(self):
        actual = probe_receipt()
        del actual["flags"]["KF3_BASELINE"]
        self.assertFalse(verify_process_receipt(receipt(), actual))

    def test_process_binary_changed(self):
        actual = probe_receipt()
        actual["sha256"] = "0" * 64
        self.assertFalse(verify_process_receipt(receipt(), actual))

    def test_device_config_changed(self):
        actual = probe_receipt()
        actual["kf3_device"] = "changed"
        self.assertFalse(verify_process_receipt(receipt(), actual))

    def test_harness_changed(self):
        actual = probe_receipt()
        actual["harness_checkout"] = "changed"
        self.assertFalse(verify_process_receipt(receipt(), actual))

    def test_plan_valid(self):
        self.assertTrue(validate_inventory(inventory()))

    def test_no_rental_authority(self):
        self.assertIs(inventory().get("rental_authorized"), False)

    def test_no_sw3_authority(self):
        self.assertEqual(inventory().get("authority"), "SEPARATE_CONTROLLER_GRANT_REQUIRED")

    def test_sole_probe_delta(self):
        self.assertEqual(inventory().get("effective_flag_delta"), {"KF3_COMPLETION_PROBE": "1000"})

    def test_no_second_probe(self):
        plan = inventory()
        plan["effective_flag_delta"] = {"KF3_COMPLETION_PROBE": "1000", "OTHER": "1"}
        self.assertFalse(validate_inventory(plan))

    def test_probe_cannot_be_off(self):
        plan = inventory()
        plan["effective_flag_delta"] = {}
        self.assertFalse(validate_inventory(plan))

    def test_budget_inputs(self):
        self.assertEqual(inventory().get("budget"), {
            "stop_usd": 2.25, "cap_usd": 2.75, "max_hourly_usd": 1,
            "max_seconds": 7200, "reserve_usd": 8, "leases": 1,
        })

    def test_budget_cap_cannot_grow(self):
        plan = inventory()
        plan["budget"] = {"cap_usd": 3}
        self.assertFalse(validate_inventory(plan))

    def test_lease_count_cannot_grow(self):
        plan = inventory()
        plan["budget"] = {"leases": 2}
        self.assertFalse(validate_inventory(plan))

    def test_no_new_source(self):
        plan = inventory()
        plan["source"] = "0" * 40
        self.assertFalse(validate_inventory(plan))

    def test_no_new_binary(self):
        plan = inventory()
        plan["qemu_sha256"] = "0" * 64
        self.assertFalse(validate_inventory(plan))

    def test_guard_scope_fixed(self):
        self.assertEqual(inventory().get("admission"), "BW6 legacy; BW7 native-template-only")

    def test_verified_only(self):
        self.assertEqual(inventory().get("verification"), "verified")

    def test_authority_cannot_be_added(self):
        plan = inventory()
        plan["rental_authorized"] = True
        self.assertFalse(validate_inventory(plan))

    def test_channel_correlation_and_falsifiers(self):
        plan = inventory()
        self.assertIn("H1-T", plan.get("falsifiers", {}))
        self.assertIn("H1-G", plan.get("falsifiers", {}))
        self.assertEqual(plan.get("inconclusive_or_survives"), "UNRESOLVED")

    def test_hold_is_not_acceptance(self):
        self.assertEqual(inventory().get("regression_seconds"), 60)
        self.assertEqual(inventory().get("acceptance_seconds"), 600)

    def test_missing_overlay_anchor_refused(self):
        with self.assertRaises(ValueError):
            arm_overlay("#!/bin/bash\n")

    def test_duplicate_overlay_anchor_refused(self):
        with self.assertRaises(ValueError):
            arm_overlay("enable_cmd=bad\nenable_cmd=bad\n")

    def test_guest_observer_starts_disabled(self):
        text = (HERE / "enable_observe.ps1").read_text() if (
            HERE / "enable_observe.ps1").exists() else ""
        self.assertIn("Problem -ne 22", text)
        self.assertIn("enable580_once.ps1", text)

    def test_guest_keeps_application_channel_separate(self):
        text = (HERE / "enable_observe.ps1").read_text() if (
            HERE / "enable_observe.ps1").exists() else ""
        self.assertIn("LogName='System'", text)
        self.assertIn("LogName='Application'", text)
        self.assertNotIn("Get-Content -LiteralPath $dump", text)


if __name__ == "__main__":
    unittest.main()
