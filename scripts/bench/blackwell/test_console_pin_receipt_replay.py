"""Receipt-relation rehearsal of the final operator; missing raw captures stay explicit."""

import base64
import hashlib
import json
from pathlib import Path
import unittest

import test_console_pin_operator as operator
from test_console_pin import C, FP, OTHER

ROOT = Path(__file__).resolve().parents[3]
TRACES = ROOT / "traces/blackwell_ab_20261023"
FINAL_KIT = TRACES / "UNVERIFIED-HOST_1345/run_kit"
PIN_PATH = TRACES / "UNVERIFIED-HOST_1320/lease_57363603/pin_receipt.jsonl"
ABSENT_PATH = TRACES / "observation_run/lease_57359730/readiness.txt"
OTHER_FP = "SHA256:" + base64.b64encode(
    hashlib.sha256(base64.b64decode(OTHER)).digest()
).decode().rstrip("=")


def inputs():
    receipt = json.loads(PIN_PATH.read_text().splitlines()[-1])
    expected = receipt["expected_fingerprint"]
    assert expected in receipt["fingerprints"]
    extra = next(pin for pin in receipt["fingerprints"] if pin != expected)
    # The scanned key bytes were not retained. Substitute known valid fixture keys
    # bijectively for opaque fingerprints, preserving all equality relationships.
    mapping = {expected: FP, extra: OTHER_FP}
    return receipt, mapping


class FinalOperatorReceiptReplay(unittest.TestCase):
    run_ready = operator.OperatorConsolePin.run_ready

    def setUp(self):
        self.old_kit = operator.KIT
        operator.KIT = FINAL_KIT

    def tearDown(self):
        operator.KIT = self.old_kit

    def test_recorded_false_refusal_pin_set_accepts_through_final_operator(self):
        receipt, mapping = inputs()
        console = "\n".join(mapping[pin] for pin in receipt["fingerprints"])
        code, calls, refusal = self.run_ready([console], elapsed=receipt["elapsed_since_create"])
        self.assertEqual(code, 0)
        self.assertIsNone(refusal)
        self.assertEqual(len([c for c in calls if c[0][0] == "ssh"]), 1)

    def test_recorded_absence_log_then_attestation_control_accepts_after_wait(self):
        self.assertIn(
            "host-key pin absent from authenticated instance console", ABSENT_PATH.read_text()
        )
        code, calls, refusal = self.run_ready(["", FP], elapsed=610)
        self.assertEqual(code, 0)
        reads = [at for argv, at in calls if argv == ("vastai", "logs")]
        self.assertEqual(len(reads), 2)
        self.assertEqual(reads[1] - reads[0], 20)
        self.assertIsNone(refusal)

    def test_derived_nonmatching_pin_control_refuses_without_ssh(self):
        receipt, mapping = inputs()
        extra = next(
            pin for pin in receipt["fingerprints"] if pin != receipt["expected_fingerprint"]
        )
        # This negative control is derived from the recorded extra fingerprint;
        # it is not represented as an independent live mismatching-host capture.
        with self.assertRaises(C.PinMismatch):
            self.run_ready([mapping[extra]], elapsed=receipt["elapsed_since_create"])
        self.assertFalse(any(argv[0] == "ssh" for argv, at in self.last_calls))


if __name__ == "__main__":
    unittest.main()
