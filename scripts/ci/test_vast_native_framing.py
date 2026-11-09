"""RV2 physical-record framing through the real hook and fake dmesg; no GPU."""

import unittest
import test_vast_payload_native_fault as fixture
from test_vast_payload_native_fault import BLACKWELL, LEGACY, foreign_fault


class NativeFramingHook(unittest.TestCase):
    """RV2: every watcher-selected fragment must stay visible to admission."""

    def reject(self, line):
        rc, failed, allowed, output = fixture.NativeExecHook().invoke(line)
        self.assertIn("NEW HOST Xid ACROSS THE COMMAND", output)
        self.assertIn("VOID", output)
        self.assertEqual((rc, failed, allowed), (6, True, False), output)

    def variants(self, line, kind):
        other = foreign_fault(line)
        if kind == "spacing":
            return [
                other.replace("NVRM: Xid", prefix) + " " + line
                for prefix in ("NVRM:  Xid", "NVRM:\tXid", "NVRM: \t Xid")
            ]
        if kind == "case":
            return [
                other.replace("NVRM: Xid", prefix) + " " + line
                for prefix in ("nvrm: xid", "NvRm: xID")
            ]
        if kind == "concatenation":
            return [other + " " + line, line + " " + other]
        if kind == "trailing":
            return [line + " trailing text", line + " nvrm: xid 79 unparsed"]
        if kind == "later-line":
            return [
                line + "\n" + other.replace("NVRM: Xid", prefix)
                for prefix in ("NVRM:  Xid", "nvrm: xid")
            ]
        if kind == "prefix":
            return [
                "opaque prefix " + line,
                "2026-11-09T02:20:00+0000 ubuntu kernel: "
                + other.replace("NVRM: Xid", "nvrm: xid")
                + " "
                + line,
                "mMu FaUlT unparsed " + line,
                "GPU has fallen off the bus " + line,
            ]
        if kind == "pci":
            return [line.replace("PCI:0000:00:00", "PCI:0000:00:00 nvrm: xid 79")]
        if kind == "selector-prefix":
            return ["2026-11-09T02:20:00+0000 nvrm-xid79 kernel: " + line.split("] ", 1)[1]]
        if kind == "line-control":
            return [
                line + sep + other.replace("NVRM: Xid", "nvrm: xid")
                for sep in ("\r", "\v", "\f", "\x85", "\u2028", "\u2029")
            ]
        if kind == "trailing-control":
            return [line + sep for sep in ("\r", "\v", "\f", "\x85", "\u2028", "\u2029")]
        raise AssertionError(kind)

    def reject_variants(self, line, kind):
        for index, variant in enumerate(self.variants(line, kind)):
            with self.subTest(kind=kind, variant=index):
                self.reject(variant)

    def test_blackwell_spacing_cannot_hide_a_record(self):
        self.reject_variants(BLACKWELL, "spacing")

    def test_legacy_spacing_cannot_hide_a_record(self):
        self.reject_variants(LEGACY, "spacing")

    def test_blackwell_case_cannot_hide_a_record(self):
        self.reject_variants(BLACKWELL, "case")

    def test_legacy_case_cannot_hide_a_record(self):
        self.reject_variants(LEGACY, "case")

    def test_blackwell_concatenation_voids(self):
        self.reject_variants(BLACKWELL, "concatenation")

    def test_legacy_concatenation_voids(self):
        self.reject_variants(LEGACY, "concatenation")

    def test_blackwell_trailing_fragment_voids(self):
        self.reject_variants(BLACKWELL, "trailing")

    def test_legacy_trailing_fragment_voids(self):
        self.reject_variants(LEGACY, "trailing")

    def test_blackwell_later_line_voids(self):
        self.reject_variants(BLACKWELL, "later-line")

    def test_legacy_later_line_voids(self):
        self.reject_variants(LEGACY, "later-line")

    def test_blackwell_record_inside_prefix_voids(self):
        self.reject_variants(BLACKWELL, "prefix")

    def test_legacy_record_inside_prefix_voids(self):
        self.reject_variants(LEGACY, "prefix")

    def test_blackwell_record_inside_pci_voids(self):
        self.reject_variants(BLACKWELL, "pci")

    def test_legacy_record_inside_pci_voids(self):
        self.reject_variants(LEGACY, "pci")

    def test_blackwell_selector_inside_supported_prefix_voids(self):
        self.reject_variants(BLACKWELL, "selector-prefix")

    def test_legacy_selector_inside_supported_prefix_voids(self):
        self.reject_variants(LEGACY, "selector-prefix")

    def test_blackwell_physical_line_controls_cannot_hide_a_record(self):
        self.reject_variants(BLACKWELL, "line-control")

    def test_legacy_physical_line_controls_cannot_hide_a_record(self):
        self.reject_variants(LEGACY, "line-control")

    def test_blackwell_trailing_physical_controls_void(self):
        self.reject_variants(BLACKWELL, "trailing-control")

    def test_legacy_trailing_physical_controls_void(self):
        self.reject_variants(LEGACY, "trailing-control")

    def accept_prefixes(self, line):
        body = line.split("] ", 1)[1]
        for prefix in (
            "",
            "[ 1193.744521] ",
            "[Sun Nov  9 02:20:00 2026] ",
            "2026-11-09T02:20:00.123456+0000 ubuntu kernel: ",
            "Nov  9 02:20:00 ubuntu kernel: ",
            "kernel: ",
        ):
            with self.subTest(prefix=prefix):
                rc, failed, allowed, output = fixture.NativeExecHook().invoke(prefix + body)
                self.assertEqual((rc, failed, allowed), (0, False, True), output)

    def test_blackwell_supported_prefixes_accept(self):
        self.accept_prefixes(BLACKWELL)

    def test_legacy_supported_prefixes_accept(self):
        self.accept_prefixes(LEGACY)


if __name__ == "__main__":
    unittest.main()
