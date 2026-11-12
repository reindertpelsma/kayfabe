"""2026-11-12, release rehearsal scope (docs/design/V3_RELEASE_REHEARSAL.md), GPU-free.

FALSIFIER (written before the code): KF_RELEASE_REHEARSAL relaxes the refusal of a real release matrix leg (a
matrix leg runs, alone or labelled as a rehearsal); the marker is accepted outside the large tier or with a value other
than "1"; a large run does not plan the middle apps pair; a large run runs the other apps profile than middle; or the
payload stops declaring PAYLOAD_RELEASE_REHEARSAL while still claiming PAYLOAD_RELEASE_MATRIX_EXECUTOR = 0.
"""

import io
import re
import unittest
from pathlib import Path
from unittest.mock import patch

import test_vast_payload_apps as apps
import test_vast_payload_winapps as winapps
import test_vast_payload_v7 as v7
import test_vast_release_matrix as rm


class RehearsalFlag(unittest.TestCase):
    setUp = v7.V7Tests.setUp

    def test_absent_is_not_a_rehearsal(self):
        env = {}
        self.assertFalse(self.m.rehearsal_requested(env, "large"))
        self.assertFalse(self.m.rehearsal_requested(env, "middle"))

    def test_exactly_one_on_large_is_a_rehearsal_and_is_consumed(self):
        env = {"KF_RELEASE_REHEARSAL": "1"}
        self.assertTrue(self.m.rehearsal_requested(env, "large"))
        self.assertNotIn("KF_RELEASE_REHEARSAL", env)

    def test_marker_beside_any_matrix_field_is_refused_by_itself(self):
        for name in (
            "KF_HOST_DRIVER",
            "KF_LINUX_GUEST_DRIVER",
            "KF_WINDOWS_GUEST_DRIVER",
            "KF_WINGATE_GUEST_DRIVER",
            "KF_RELEASE_SUBRUNS",
            "KF_RELEASE_DRIVER_PINS",
        ):
            with self.assertRaises(self.m.EvidenceError, msg=name) as caught:
                self.m.rehearsal_requested({"KF_RELEASE_REHEARSAL": "1", name: "x"}, "large")
            self.assertEqual(caught.exception.name, "release_retry_matrix_invalid", name)

    def test_matrix_fields_without_the_marker_are_not_this_functions_business(self):
        env = {"KF_HOST_DRIVER": "580.88"}
        self.assertFalse(self.m.rehearsal_requested(env, "large"))
        self.assertIn("KF_HOST_DRIVER", env)

    def test_other_tiers_and_values_are_refused(self):
        for tier in ("small", "middle", "adhoc", "unknown"):
            with self.assertRaises(self.m.EvidenceError, msg=tier):
                self.m.rehearsal_requested({"KF_RELEASE_REHEARSAL": "1"}, tier)
        for value in ("0", "", "true", "yes", "11", " 1"):
            with self.assertRaises(self.m.EvidenceError, msg=value):
                self.m.rehearsal_requested({"KF_RELEASE_REHEARSAL": value}, "large")

    def test_module_constants_say_rehearsal_yes_matrix_executor_no(self):
        self.assertEqual(self.m.PAYLOAD_RELEASE_REHEARSAL, 1)
        self.assertEqual(self.m.PAYLOAD_RELEASE_MATRIX_EXECUTOR, 0)

    def test_declarations_say_rehearsal_yes_matrix_executor_no(self):
        text = (Path(__file__).with_name("vast_payload.py")).read_text()
        self.assertRegex(text, r"(?m)^PAYLOAD_RELEASE_REHEARSAL = 1$")
        self.assertRegex(text, r"(?m)^PAYLOAD_RELEASE_MATRIX_EXECUTOR = 0$")


class RehearsalMain(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture
    BASE = {
        "KF_TIER": "large",
        "KF_LINUX_APPS": "1",
        "KF_WINAPPS": "mid",
        "KF_RELEASE_REHEARSAL": "1",
    }

    def go(self, env=None, soft=300, names=None):
        names = apps.NAMES if names is None else names
        extra = dict(self.BASE)
        extra.update(env or {})
        extra = {k: v for k, v in extra.items() if v is not None}
        logs = {
            "apps": (apps.log(names=names), 0),
            "windows_apps": winapps.apps_log("PASS", profile="mid"),
        }
        out = io.StringIO()
        asked = []

        def listed(root, tier="large"):
            asked.append(tier)
            return names

        with patch.object(self.m, "apps_gate_list", side_effect=listed):
            with patch("sys.stdout", out):
                rc, result, calls = self._main_fixture(
                    windows="1", soft_ms=soft * 60000, extra_env=extra, stage_logs=logs
                )
        return rc, result, calls, out.getvalue(), asked

    def failed_with(self, result, name):
        return any(row["name"] == name and row["fail"] for row in result["suites"])

    def test_rehearsal_plans_the_middle_apps_pair_with_the_middle_profile(self):
        rc, result, calls, text, asked = self.go()
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-3:], ["apps", "windows_gate", "windows_apps"])
        self.assertEqual(asked, ["middle"])
        self.assertRegex(text, r"KF_PAYLOAD_TIER .* tier=large rehearsal=1 .* winapps=mid linuxapps=1\n")

    def test_large_without_the_marker_also_plans_the_apps_pair(self):
        rc, result, calls, text, asked = self.go({"KF_RELEASE_REHEARSAL": None})
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-3:], ["apps", "windows_gate", "windows_apps"])
        self.assertRegex(text, r"tier=large rehearsal=0 ")
        self.assertEqual(asked, ["middle"])

    def test_small_and_adhoc_still_cannot_request_the_apps_pair(self):
        for tier in ("small", "adhoc"):
            rc, result, calls, _, _ = self.go({"KF_TIER": tier, "KF_RELEASE_REHEARSAL": None})
            self.assertEqual(rc, 1, tier)
            self.assertTrue(self.failed_with(result, "preflight"), tier)
            self.assertEqual(calls, [], tier)

    def test_marker_outside_large_is_refused_before_any_stage(self):
        for tier in ("middle", "small"):
            rc, result, calls, _, _ = self.go({"KF_TIER": tier})
            self.assertEqual(rc, 1, tier)
            self.assertTrue(self.failed_with(result, "preflight"), tier)
            self.assertEqual(calls, [], tier)

    def test_a_real_matrix_leg_is_still_refused(self):
        env = rm.ReleaseEnvironment.setUp
        holder = type("H", (), {})()
        env(holder)
        # alone: refused as a proposed matrix (the unchanged refusal); labelled as a rehearsal: an invalid combination
        for marker, name in ((None, "release_retry_matrix_proposed"), ("1", "release_retry_matrix_invalid")):
            rc, result, calls, _, _ = self.go(dict(holder.env, KF_RELEASE_REHEARSAL=marker))
            self.assertEqual(rc, 1, marker)
            self.assertTrue(self.failed_with(result, name), (marker, result))
            self.assertEqual(calls, [], marker)

    def test_any_single_matrix_field_beside_the_marker_is_refused(self):
        for name in ("KF_HOST_DRIVER", "KF_RELEASE_SUBRUNS", "KF_RELEASE_DRIVER_PINS"):
            rc, result, calls, _, _ = self.go({name: "580.88"})
            self.assertEqual(rc, 1, name)
            self.assertEqual(calls, [], name)
            self.assertTrue(self.failed_with(result, "release_retry_matrix_invalid"), (name, result))


if __name__ == "__main__":
    unittest.main()
