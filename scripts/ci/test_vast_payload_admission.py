"""Authentic S4 native journals test the payload's admission replacement point."""

from pathlib import Path
import re
import unittest

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
