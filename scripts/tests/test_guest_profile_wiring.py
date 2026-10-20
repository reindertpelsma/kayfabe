#!/usr/bin/env python3
"""Source integration contracts complement the pure effective-config tests; no GPU."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]


class GuestProfileWiring(unittest.TestCase):
    def test_qom_property_reaches_the_realize_call(self):
        source = (ROOT / "qemu/hw/misc/kf3/kf3.c").read_text()
        self.assertTrue(re.search(r'DEFINE_PROP_STRING\("guest-os",\s*Kf3State,\s*guest_os\)', source),
                        "missing explicit guest-os QOM property")
        call = source[source.index("int32_t realize_rc = kf3_realize("):]
        self.assertTrue("s->guest_os" in call[:call.index(";")], "QOM property lost before FFI")

    def test_ffi_decodes_the_property_before_opening_a_host_device(self):
        source = (ROOT / "crates/kf-qemu/src/ffi_unsafe.rs").read_text()
        start = source.index('pub unsafe extern "C" fn kf3_realize(')
        source = source[start:]
        self.assertTrue("guest_os: *const c_char" in source[:source.index(") -> i32")],
                        "missing FFI guest-os argument")
        self.assertLess(source.index("GuestProfile::parse"), source.index("Device::realize"))

    def test_profile_is_not_a_process_global_environment_switch(self):
        source = (ROOT / "crates/kf-abi/src/guest_profile.rs").read_text()
        self.assertNotIn("std::env", source)
        self.assertNotIn("OnceLock", source)
        self.assertNotIn("static mut", source)

    def test_boot_profile_reaches_all_four_construction_boundaries(self):
        source = (ROOT / "crates/kf-qemu/src/device.rs").read_text()
        for call in [".with_guest_profile(cfg.guest_profile)", "served_policy_with_profile(",
                     "cfg.guest_profile.settings()", "pub guest_profile:"]:
            self.assertTrue(call in source, f"missing profile construction boundary: {call}")

    def test_captured_oracle_is_below_the_existing_test_module_boundary(self):
        source = (ROOT / "crates/kf-qemu/src/display.rs").read_text()
        boundary = source.index("#[cfg(test)]")
        for token in ["const CAPS_PROBE_AT:", "const CAPS_PROBE_WORDS:", "fn caps_probe_page("]:
            self.assertGreater(source.index(token), boundary)

    def test_production_display_cannot_select_a_captured_page(self):
        source = (ROOT / "crates/kf-qemu/src/display.rs").read_text().split("#[cfg(test)]")[0]
        self.assertNotIn("CAPS_PROBE_WORDS", source)
        self.assertNotIn("caps_probe_page(", source)
        self.assertNotIn('std::env::var("KF3_DISPLAY_CAPS_PROBE")', source)
        self.assertIn("if settings.derived_caps", source)


if __name__ == "__main__":
    unittest.main()
