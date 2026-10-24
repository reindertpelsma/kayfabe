"""Synthetic preparation/overlay contracts; no provider, SSH or guest is contacted."""

import hashlib
import pathlib
from pathlib import Path
import tempfile
from types import SimpleNamespace
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
    return {
        "sha256": "05d3d46f4e8429f5ffebacbd7ca9f5156f7d6f1141b4b10ce84732b174347448",
        "kf3_device": "fixed-device-config",
        "harness_checkout": "fixed-harness",
        "flags": {"KF3_BASELINE": "1"},
    }


def probe_receipt():
    row = receipt()
    row["flags"]["KF3_COMPLETION_PROBE"] = "1000"
    return row


def arm_fixture():
    return (
        "#!/bin/bash\n"
        "export WINVM_QEMU=$binary\n"
        'for f in "$enable_script" diag_min.ps1; do true; done\n'
        'python3 bind_receipt.py "arm_${run}" "$binary" "$profile"\n'
        'python3 - "$profile" "$run" "$source" "$binary" <<\'PY\'\n'
        "rec={'harness_sha256':old_hash}\nPY\n"
        'enable_cmd="powershell -File C:\\kf\\$enable_script"\n'
        'cat "${run}_enable.txt"\n'
        "while [ $((SECONDS-start_seconds)) -lt 600 ]; do break; done\n"
    )


class KitTests(unittest.TestCase):
    def source_receipt(self, data):
        text = arm_overlay(arm_fixture())
        body = text.split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        with tempfile.TemporaryDirectory(prefix="bw8-receipt-") as folder:
            root = Path(folder)
            (root / "binary").write_bytes(b"different baseline binary")
            (root / "operator").write_bytes(data)
            args = ["", "", "", "", str(root / "binary"), str(root / "operator")]
            env = {
                "hashlib": hashlib,
                "pathlib": pathlib,
                "sys": SimpleNamespace(argv=args),
                "old_hash": "baseline-hash",
            }
            exec(compile(body, "synthetic-receipt", "exec"), env)
            return env["rec"]

    def test_receipt_records_operator_bytes(self):
        row = self.source_receipt(b"operator one")
        self.assertEqual(row["harness_sha256"], hashlib.sha256(b"operator one").hexdigest())
        self.assertEqual(row["baseline_harness_sha256"], "baseline-hash")

    def test_receipt_distinguishes_operator_revision(self):
        row = self.source_receipt(b"operator two")
        self.assertEqual(row["harness_sha256"], hashlib.sha256(b"operator two").hexdigest())
        self.assertEqual(row["baseline_harness_sha256"], "baseline-hash")

    def test_overlay_receipt_names_baseline_hash(self):
        self.assertIn("'baseline_harness_sha256':", arm_overlay(arm_fixture()))

    def test_overlay_receipt_hashes_actual_operator(self):
        text = arm_overlay(arm_fixture())
        self.assertIn('"${BASH_SOURCE[0]}"', text)
        self.assertIn("pathlib.Path(sys.argv[5]).read_bytes()", text)

    def test_overlay_retains_acceptance_hold(self):
        self.assertIn("-lt 600", arm_overlay(arm_fixture()))

    def test_overlay_waits_for_inherited_absolute_readiness(self):
        text = arm_overlay(arm_fixture())
        self.assertIn('wait "$init_observer_pid"', text)
        self.assertLess(text.index('wait "$init_observer_pid"'), text.index("/bw8_observer.py"))

    def test_overlay_readiness_failure_propagates(self):
        text = arm_overlay(arm_fixture())
        self.assertIn('wait "$init_observer_pid"\npython3', text)
        self.assertNotIn('wait "$init_observer_pid" || true', text)

    def test_overlay_missing_export_refused(self):
        with self.assertRaises(ValueError):
            arm_overlay(arm_fixture().replace("export WINVM_QEMU=$binary\n", ""))

    def test_overlay_duplicate_export_refused(self):
        with self.assertRaises(ValueError):
            arm_overlay(arm_fixture() + "export WINVM_QEMU=$binary\n")

    def test_overlay_gates_before_formal_hold(self):
        text = arm_overlay(arm_fixture())
        self.assertLess(text.index("/bw8_observer.py"), text.index("-lt 600"))

    def test_overlay_verifies_effective_receipt(self):
        text = arm_overlay(arm_fixture())
        self.assertIn("--verify-receipt", text)
        self.assertLess(text.index("--verify-receipt"), text.index("enable_cmd="))

    def test_overlay_missing_capture_refused(self):
        text = arm_fixture().replace('cat "${run}_enable.txt"\n', "")
        with self.assertRaises(ValueError):
            arm_overlay(text)

    def test_overlay_duplicate_capture_refused(self):
        with self.assertRaises(ValueError):
            arm_overlay(arm_fixture() + 'cat "${run}_enable.txt"\n')

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

    def test_baseline_binary_changed(self):
        baseline = receipt()
        baseline["sha256"] = "0" * 64
        self.assertFalse(verify_process_receipt(baseline, probe_receipt()))

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
        self.assertEqual(
            inventory().get("budget"),
            {
                "stop_usd": 2.25,
                "cap_usd": 2.75,
                "max_hourly_usd": 1,
                "max_seconds": 7200,
                "reserve_usd": 8,
                "leases": 1,
            },
        )

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
        text = (
            (HERE / "enable_observe.ps1").read_text()
            if (HERE / "enable_observe.ps1").exists()
            else ""
        )
        self.assertIn("Problem -ne 22", text)
        self.assertIn("enable580_once.ps1", text)

    def test_guest_keeps_application_channel_separate(self):
        text = (
            (HERE / "enable_observe.ps1").read_text()
            if (HERE / "enable_observe.ps1").exists()
            else ""
        )
        self.assertIn("LogName='System'", text)
        self.assertIn("LogName='Application'", text)
        self.assertNotIn("Get-Content -LiteralPath $dump", text)


if __name__ == "__main__":
    unittest.main()
