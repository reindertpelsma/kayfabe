# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The committed bundle.pin must name a bundle built from THIS tree's inputs (offline, no network).

2026-11-20 (measured): apps.json (#470, #471, #475) and guest/py/win_cuda_equiv.py (#475) changed after the 2026-11-08 pin,
and every fresh box then failed `windows_apps` with PUBLIC_BUNDLE_FAIL. A change to a hashed input now needs a rebuilt and
republished bundle and a new pin in the same change (docs/DOWNLOADS.md, Windows app bundle, "Republish").
"""
from pathlib import Path
import unittest

import helpers as H
import package_bundle as B
import bundle_ci as C


class CommittedPin(unittest.TestCase):
    def test_pin_inputs_equal_the_tree_input_hash(self):
        tree = B.input_hash(B.load(Path(H.AM) / "manifest.json"), B.load(Path(H.AM) / "gate_profiles.json"))
        C.check_pin_inputs((Path(H.AM) / "bundle.pin").read_text(encoding="utf-8"), tree)

    def test_the_pin_carries_an_image_and_an_inputs_line(self):
        pin = C.read_pin((Path(H.AM) / "bundle.pin").read_text(encoding="utf-8"))
        self.assertEqual(sorted(pin), ["inputs", "inventory", "sha256"])


if __name__ == "__main__":
    unittest.main()
