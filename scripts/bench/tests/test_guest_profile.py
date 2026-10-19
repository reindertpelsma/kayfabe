#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The fat-image driver axis must not silently become the host's driver axis."""
from pathlib import Path
import subprocess
import tempfile
import unittest


HELPER = Path(__file__).resolve().parents[1] / "guest_profile.sh"


class GuestProfileTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.image = Path(self.tmp.name) / "guest with spaces.qcow2"
        self.receipt = Path(str(self.image) + ".STAGED")

    def resolve(self, extra=""):
        return subprocess.run(
            ["bash", "-c", '. "$1"; kf_guest_driver_arg "$2" "$3"',
             "bash", str(HELPER), str(self.image), extra],
            capture_output=True, text=True, check=False,
        )

    def test_the_image_receipt_selects_580_independently_of_a_595_host(self):
        self.receipt.write_text("version=580.159.04\nimage=guest.qcow2\nstaged=2026-10-15\n")
        result = self.resolve("display=on")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "guest-driver=580.159.04\n")
        self.assertIn("source=receipt", result.stderr)

    def test_unknown_image_profile_is_refused_before_qemu(self):
        result = self.resolve()
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")
        self.assertIn("guest profile refused", result.stderr)

    def test_explicit_profile_overrides_the_receipt(self):
        self.receipt.write_text("version=580.159.04\n")
        result = self.resolve("display=on,guest-driver=595.91.07,fb-mb=4096")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "guest-driver=595.91.07\n")
        self.assertIn("source=explicit", result.stderr)

    def test_two_part_version_is_valid(self):
        result = self.resolve("guest-driver=595.84")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "guest-driver=595.84\n")

    def test_duplicate_profile_properties_are_refused(self):
        self.assertEqual(self.resolve("guest-driver=580.159.04,guest-driver=595.84").returncode, 2)

    def test_duplicate_receipt_versions_are_refused(self):
        self.receipt.write_text("version=580.159.04\nversion=595.84\n")
        self.assertEqual(self.resolve().returncode, 2)

    def test_similar_property_names_do_not_select_a_profile(self):
        self.assertEqual(self.resolve("other-guest-driver=580.159.04").returncode, 2)

    def test_empty_and_malformed_versions_are_refused(self):
        for version in ("", "580", "580.159.04,display=on", "580.159.04 extra"):
            with self.subTest(version=version):
                self.receipt.write_text(f"version={version}\n")
                self.assertEqual(self.resolve().returncode, 2)

    def test_receipt_contents_are_never_executed(self):
        marker = Path(self.tmp.name) / "must-not-exist"
        self.receipt.write_text(f"version=$(touch '{marker}')\n")
        self.assertEqual(self.resolve().returncode, 2)
        self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
