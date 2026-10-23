"""Authentic S4 native journals test the payload's admission replacement point."""

from pathlib import Path
import io
import json
import os
import re
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import vast_payload as payload
from test_vast_payload import watch

ROOT = Path(__file__).resolve().parents[2]


def replay(die):
    if die == "GA102":
        base = ROOT / "traces/sweep_dies_20261018/lease_57209908"
        return (
            (base / "safety_stopped/native_kernel_snapshot.log").read_text(),
            (base / "native_controls/defer-liveness.log").read_text(),
            (base / "native_controls/missing-page-fault.log").read_text(),
        )
    base = ROOT / "traces/sweep_ada_20261018/lease_57144505/native_complete"
    return (
        (base / "kernel_xids.log").read_text(),
        (base / "sweepf98ceae6_ad102_native595_defer-liveness.log").read_text(),
        (base / "sweepf98ceae6_ad102_native595_missing-page-fault.log").read_text(),
    )


class NativeArmAdmission(unittest.TestCase):
    def invoke(self, die, change=lambda s: s, source_change=lambda s: s, arm="--defer-liveness"):
        journal, defer, _ = replay(die)
        lines = [s for s in journal.splitlines() if "NVRM: Xid" in s and "ENGINE CE0" in s]
        delta = change("\n".join(lines))
        count = len(delta.splitlines())
        text = watch(delta, 0, count).replace("\nfixture\n", "\n" + source_change(defer) + "\n")
        return payload.xid_ok(text, 6, arm, source_change(defer))

    def test_authentic_ad102_defer_four_exact_ce_faults_admit(self):
        self.assertTrue(self.invoke("AD102"))

    def test_authentic_ga102_defer_four_exact_ce_faults_admit(self):
        self.assertTrue(self.invoke("GA102"))

    def test_wrong_source_channel_refuses_on_both_dies(self):
        for die in ("AD102", "GA102"):
            with self.subTest(die=die):
                self.assertFalse(
                    self.invoke(
                        die,
                        lambda s: re.sub(r"channel 0x[0-9a-f]+", "channel 0x000000ff", s, count=1),
                    )
                )

    def test_wrong_source_va_refuses_on_both_dies(self):
        for die in ("AD102", "GA102"):
            with self.subTest(die=die):
                self.assertFalse(
                    self.invoke(die, lambda s: s.replace("0x60_00010000", "0x60_00020000", 1))
                )

    def test_wrong_fault_type_refuses_on_both_dies(self):
        for die in ("AD102", "GA102"):
            with self.subTest(die=die):
                self.assertFalse(self.invoke(die, lambda s: s.replace("FAULT_PTE", "FAULT_PDE", 1)))

    def test_non_ce_client_refuses_on_both_dies(self):
        for die in ("AD102", "GA102"):
            with self.subTest(die=die):
                self.assertFalse(
                    self.invoke(
                        die, lambda s: re.sub(r"HUBCLIENT_CE[0-9]+", "HUBCLIENT_PMU", s, count=1)
                    )
                )

    def test_extra_exact_record_refuses_on_both_dies(self):
        for die in ("AD102", "GA102"):
            with self.subTest(die=die):
                self.assertFalse(self.invoke(die, lambda s: s + "\n" + s.splitlines()[0]))

    def test_other_xid_refuses_on_both_dies(self):
        for die in ("AD102", "GA102"):
            with self.subTest(die=die):
                self.assertFalse(self.invoke(die, lambda s: s.replace(": 31,", ": 32,", 1)))

    def test_missing_run_source_marker_refuses(self):
        self.assertFalse(
            self.invoke("AD102", source_change=lambda s: s.replace("FAULT_MARK=", "NO_MARK="))
        )

    def test_missing_positive_recovery_refuses(self):
        self.assertFalse(
            self.invoke(
                "GA102", source_change=lambda s: s.replace("DEFER_RIGCTL=PASS", "DEFER_RIGCTL=FAIL")
            )
        )

    def test_timer_cannot_borrow_a_complete_defer_proof(self):
        self.assertFalse(self.invoke("AD102", arm="--timer"))

    def test_executor_cannot_borrow_a_complete_defer_proof(self):
        self.assertFalse(self.invoke("GA102", arm="--executor-vas"))

    def test_negative_control_cannot_pass_without_kernel_records(self):
        _, defer, _ = replay("AD102")
        self.assertFalse(payload.xid_ok(watch(), 0, "--defer-liveness", defer))


class NativeReceipt(unittest.TestCase):
    def invoke(self, die, corrupt=lambda s: s):
        journal, defer, missing = replay(die)
        lines = [s for s in journal.splitlines() if "NVRM: Xid" in s]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = {
                "KF_PAYLOAD_LOG": str(root / "output"),
                "KF_PAYLOAD_CONTROL_DIR": str(root / "controls"),
                "KF_PAYLOAD_NATIVE_RECEIPT": str(root / "receipt"),
                "KF_PAYLOAD_DIAGNOSTIC_STATE": str(root / "diagnostics"),
                "KF_PAYLOAD_XID_ALLOWED": str(root / "allowed"),
                "KF_PAYLOAD_XID_FAILED": str(root / "failed"),
            }
            with patch.dict(os.environ, env), patch.object(
                payload.xid_guard, "kernel_snapshot", return_value=corrupt(journal)
            ), redirect_stdout(io.StringIO()):
                result = None
                for arm, source, records in (
                    ("--defer-liveness", defer, [s for s in lines if "ENGINE CE0" in s]),
                    ("--missing-page-fault", missing, [s for s in lines if "ENGINE CE0" not in s]),
                ):
                    text = watch("\n".join(records), 0, len(records)).replace(
                        "\nfixture\n", "\n" + source + "\n"
                    )
                    log = root / "watch"
                    log.write_text(text)
                    result = payload.guard_command(arm, log, 6)
            receipt = (
                json.loads((root / "receipt").read_text()) if (root / "receipt").exists() else None
            )
            return result, receipt, (root / "failed").exists(), lines

    def test_ad102_exact_five_are_frozen_for_guest_supervision(self):
        rc, receipt, failed, lines = self.invoke("AD102")
        self.assertEqual(rc, 0)
        self.assertFalse(failed)
        self.assertEqual(receipt["approved_native_xid_lines"], lines)

    def test_ga102_exact_five_are_frozen_for_guest_supervision(self):
        rc, receipt, failed, lines = self.invoke("GA102")
        self.assertEqual(rc, 0)
        self.assertFalse(failed)
        self.assertEqual(receipt["approved_native_xid_lines"], lines)

    def test_extra_ad102_kernel_record_stops_without_a_receipt(self):
        rc, receipt, failed, _ = self.invoke(
            "AD102", lambda s: s + "\n" + next(x for x in s.splitlines() if "NVRM: Xid" in x)
        )
        self.assertNotEqual(rc, 0)
        self.assertTrue(failed)
        self.assertIsNone(receipt)

    def test_changed_ga102_kernel_record_stops_without_a_receipt(self):
        rc, receipt, failed, _ = self.invoke("GA102", lambda s: s.replace(": 31,", ": 32,", 1))
        self.assertNotEqual(rc, 0)
        self.assertTrue(failed)
        self.assertIsNone(receipt)
