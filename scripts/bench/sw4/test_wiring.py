"""Exercise the executable adapter with a fake command provider before create."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import lifecycle
import test_sw4


class Wiring(unittest.TestCase):
    def test_complete_real_adapter_fake_transport(self):
        import operator_backend as O

        result, events = O.rehearse()
        self.assertTrue(result["api_absent"])
        self.assertTrue(result["capture_verified"])
        self.assertTrue(result["watch_cancelled_after_absence"])
        self.assertIn("native-binding", events)
        self.assertLess(events.index("bare30"), events.index("kf3-build"))
        self.assertLess(events.index("kf3-build"), events.index("guest30-180s"))

    def test_seven_cpu_preflight_refuses_before_build(self):
        import operator_backend as O

        result, events = O.rehearse(cpus=7)
        self.assertTrue(result["refused"])
        self.assertNotIn("native-binding", events)
        self.assertIn("destroy", events)

    def test_one_cpu_preflight_refuses_before_build(self):
        import operator_backend as O

        result, events = O.rehearse(cpus=1)
        self.assertTrue(result["refused"])
        self.assertNotIn("native-binding", events)

    def test_remote_native_pass_before_guest_build(self):
        import operator_backend as O

        script = O.remote_script()
        self.assertLess(script.index("SW4_NATIVE_PASS"), script.index("build_kf3.sh"))

    def test_remote_source_contract(self):
        import operator_backend as O

        script = O.remote_script()
        self.assertIn("714ec29d3fdca0a013d9d478755346b0f1af6fa7", script)
        self.assertIn("KF_FROM_HOST=1", script)
        self.assertIn("KF3_FB_MB=8192", script)
        self.assertIn("sw4_guest 180", script)
        self.assertLess(script.index("bare_metal_suite.sh"), script.index("build_kf3.sh"))

    def test_remote_binding_precedes_gates(self):
        import operator_backend as O

        script = O.remote_script()
        self.assertLess(script.index("die_binding.py"), script.index("v3_gates.sh"))
        self.assertNotIn("arch=(368, 6,", script)

    def test_source_lock_is_before_arming(self):
        import operator_backend as O

        result, events = O.rehearse()
        self.assertLess(events.index("source-lock"), events.index("arm"))

    def test_source_lock_failure_has_no_create(self):
        import operator_backend as O

        with self.assertRaises(ValueError):
            O.rehearse(bad_source=True)

    def test_source_lock_failure_second_fixture(self):
        import operator_backend as O

        with self.assertRaises(ValueError):
            O.rehearse(missing_source=True)

    def test_bad_capture_prevents_normal_success(self):
        import operator_backend as O

        with self.assertRaises(ValueError):
            O.rehearse(bad_capture=True)

    def test_wrong_native_binding_stops_guest(self):
        import operator_backend as O

        result, events = O.rehearse(native_refusal=True)
        self.assertNotIn("guest30-180s", events)
        self.assertIn("destroy", events)
        self.assertEqual(events.count("create"), 1)


if __name__ == "__main__":
    unittest.main()
