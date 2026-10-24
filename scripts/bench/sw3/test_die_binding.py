"""Native binding uses the derived die tuple; marketing names are advisory."""

import copy
from pathlib import Path
import unittest

from die_binding import BindingRefused, bind

ROOT = Path(__file__).resolve().parents[3]
DIES = (ROOT / "crates/kf-chip/src/die.rs").read_text()
ARCHES = (ROOT / "crates/kf-chip/src/lib.rs").read_text()
PROBE = (
    "host_driver=595.91.07 arch=(368, 4, 161)\n"
    "PASS native read-only timer map/read/drop x3\n"
)
BINDING = "NVIDIA GeForce RTX 3060, 595.91.07, 0x248710DE\n"
PIN = {
    "driver": "595.91.07", "pci": "0x248710DE", "arch_impl": [368, 4],
    "ssh_verified": True,
    "identity": {"state": "match", "expected_fingerprint": "fixture-pin",
                 "fingerprint": "fixture-pin", "fingerprints": ["fixture-pin"]},
}


def check(probe=PROBE, binding=BINDING, pin=None):
    return bind(probe, binding, DIES, ARCHES, copy.deepcopy(PIN) if pin is None else pin)


class DieBindingTests(unittest.TestCase):
    def test_actual_ga104_under_rtx3060_name(self):
        self.assertEqual(check()["die"], "GA104")

    def test_marketing_name_cannot_override_native_tuple(self):
        self.assertEqual(check(binding=BINDING.replace("RTX 3060", "RTX 3070"))["die"], "GA104")

    def test_native_tuple_is_retained(self):
        result = check()
        self.assertEqual((result["architecture"], result["implementation"]), (368, 4))

    def test_unknown_native_tuple_refused(self):
        pin = copy.deepcopy(PIN)
        pin["arch_impl"] = [368, 9]
        with self.assertRaises(BindingRefused):
            check(probe=PROBE.replace("368, 4", "368, 9"), pin=pin)

    def test_different_unknown_native_tuple_refused(self):
        pin = copy.deepcopy(PIN)
        pin["arch_impl"] = [999, 4]
        with self.assertRaises(BindingRefused):
            check(probe=PROBE.replace("368, 4", "999, 4"), pin=pin)

    def test_out_of_scope_ga107_refused(self):
        pin = copy.deepcopy(PIN)
        pin["arch_impl"] = [368, 7]
        with self.assertRaises(BindingRefused):
            check(probe=PROBE.replace("368, 4", "368, 7"), pin=pin)

    def test_out_of_scope_blackwell_refused(self):
        pin = copy.deepcopy(PIN)
        pin["arch_impl"] = [432, 2]
        with self.assertRaises(BindingRefused):
            check(probe=PROBE.replace("368, 4", "432, 2"), pin=pin)

    def test_malformed_tuple_refused(self):
        with self.assertRaises(BindingRefused):
            check(probe=PROBE.replace("368, 4, 161", "368, four, 161"))

    def test_truncated_probe_refused(self):
        with self.assertRaises(BindingRefused):
            check(probe=PROBE.splitlines()[0])

    def test_duplicate_probe_refused(self):
        with self.assertRaises(BindingRefused):
            check(probe=PROBE + PROBE)

    def test_inconsistent_driver_refused(self):
        with self.assertRaises(BindingRefused):
            check(binding=BINDING.replace("595.91.07", "580.159.04"))

    def test_inconsistent_pci_binding_refused(self):
        with self.assertRaises(BindingRefused):
            check(binding=BINDING.replace("2487", "2504"))

    def test_inconsistent_tuple_binding_refused(self):
        with self.assertRaises(BindingRefused):
            check(probe=PROBE.replace("368, 4", "368, 6"))

    def test_real_ssh_failure_refused(self):
        pin = copy.deepcopy(PIN)
        pin["ssh_verified"] = False
        with self.assertRaises(BindingRefused):
            check(pin=pin)

    def test_console_mismatch_refused(self):
        pin = copy.deepcopy(PIN)
        pin["identity"]["state"] = "mismatch"
        with self.assertRaises(BindingRefused):
            check(pin=pin)

    def test_console_absence_refused(self):
        pin = copy.deepcopy(PIN)
        pin["identity"]["fingerprints"] = []
        with self.assertRaises(BindingRefused):
            check(pin=pin)

    def test_different_ssh_pin_refused(self):
        pin = copy.deepcopy(PIN)
        pin["identity"]["fingerprint"] = "different"
        with self.assertRaises(BindingRefused):
            check(pin=pin)


if __name__ == "__main__":
    unittest.main()
