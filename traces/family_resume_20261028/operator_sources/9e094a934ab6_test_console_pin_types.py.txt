"""B1g (MEASURED lease 57443671): consoles publish DSA/ECDSA/RSA/ED25519 fingerprint lines.

The guard scans ed25519 only; a fingerprint labelled with another key type must not count as a
published ed25519 pin. Unlabelled pins still count (unknown type stays fail-closed).
"""

import base64
import hashlib
import unittest

import console_pin as C
import test_console_pin as T
from test_console_pin import FP, KEY, OTHER


def fp_of(blob):
    raw = base64.b64decode(blob)
    return "SHA256:" + base64.b64encode(hashlib.sha256(raw).digest()).decode().rstrip("=")


OTHER_FP = fp_of(OTHER)
DSA_FP = "SHA256:" + "D" * 43
RSA_FP = "SHA256:" + "R" * 43
ECDSA_FP = "SHA256:" + "E" * 43


def line(fp, kind):
    return "<14>Oct 24 00:35:01 cloud-init: 1024 %s root@ubuntu (%s)" % (fp, kind)


class TypedPins(unittest.TestCase):
    def test_dsa_fingerprint_is_not_an_ed25519_pin(self):
        self.assertEqual(C.console_pins(line(DSA_FP, "DSA"), KEY), set())

    def test_rsa_and_ecdsa_fingerprints_are_not_ed25519_pins(self):
        text = "\n".join([line(RSA_FP, "RSA"), line(ECDSA_FP, "ECDSA")])
        self.assertEqual(C.console_pins(text, KEY), set())

    def test_full_console_shape_yields_only_the_ed25519_pin(self):
        text = "\n".join(
            [line(DSA_FP, "DSA"), line(ECDSA_FP, "ECDSA"), line(FP, "ED25519"), line(RSA_FP, "RSA")]
        )
        self.assertEqual(C.console_pins(text, KEY), {FP})

    def test_labelled_ed25519_mismatch_still_counts(self):
        self.assertEqual(C.console_pins(line(OTHER_FP, "ED25519"), KEY), {OTHER_FP})

    def test_unlabelled_fingerprint_still_counts(self):
        self.assertEqual(C.console_pins("fingerprint " + DSA_FP, KEY), {DSA_FP})


class TypedWait(unittest.TestCase):
    setUp = T.ConsolePin.setUp
    wait = T.ConsolePin.wait

    def test_dsa_line_before_ed25519_publication_is_absent_not_mismatch(self):
        first = line(DSA_FP, "DSA")
        result, calls, sleeps, receipts = self.wait([first, first + "\n" + line(FP, "ED25519")])
        self.assertEqual(result["fingerprint"], FP)
        self.assertEqual([r["state"] for r in receipts], ["absent", "match"])

    def test_different_ed25519_line_still_refuses(self):
        with self.assertRaises(C.PinMismatch):
            self.wait([line(DSA_FP, "DSA") + "\n" + line(OTHER_FP, "ED25519")])


if __name__ == "__main__":
    unittest.main()
