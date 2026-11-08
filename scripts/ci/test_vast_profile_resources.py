"""Falsifiers: an added download stage or understated profile total passes the cost guard."""

import copy
import json
from pathlib import Path
import sys
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import profile_resources as guard

DATA = json.loads((guard.ROOT / "ci/vast-broker/defaults.json").read_text())
SOURCE = (HERE / "vast_payload.py").read_text()


class ProfileResources(unittest.TestCase):
    def test_live_payload_stages_have_budget_rows(self):
        self.assertEqual(guard.validate(DATA, SOURCE), [])

    def test_windows_apps_download_cannot_lose_its_budget_row(self):
        data = copy.deepcopy(DATA)
        del data["payloadStageResources"]["windows_apps"]
        self.assertIn(
            "unbudgeted payload stage: windows_apps", guard.validate(data, SOURCE)
        )

    def test_windows_bundle_is_allocated_inside_existing_media_headroom(self):
        table = DATA["payloadStageResources"]
        self.assertEqual(table["windows_apps"]["bandwidthDownGb"], 8)
        self.assertEqual(table["media_and_margin"]["bandwidthDownGb"], 9)
        self.assertEqual(
            [
                DATA["payloadProfiles"][name]["bandwidthDownGb"]
                for name in ["small", "middle", "large", "adhoc"]
            ],
            [73, 87, 130, 38],
        )

    def test_new_direct_download_stage_is_rejected(self):
        source = (
            SOURCE
            + '\nmake_stage("extra_download", ["curl", "example.invalid"], None, [], None)\n'
        )
        self.assertIn(
            "unbudgeted payload stage: extra_download", guard.validate(DATA, source)
        )

    def test_new_conditional_download_stage_is_rejected(self):
        source = (
            SOURCE
            + '\nif enabled:\n    make_stage("media_fetch", ["wget", "example.invalid"], None, [], None)\n'
        )
        self.assertIn(
            "unbudgeted payload stage: media_fetch", guard.validate(DATA, source)
        )

    def test_opaque_stage_name_is_rejected(self):
        self.assertTrue(
            guard.validate(
                DATA, SOURCE + "\nmake_stage(dynamic_name, [], None, [], None)\n"
            )
        )

    def test_large_total_cannot_drop_the_spec_download(self):
        data = copy.deepcopy(DATA)
        data["payloadProfiles"]["large"]["bandwidthDownGb"] -= 43
        self.assertIn(
            "profile stage sum: large/bandwidthDownGb", guard.validate(data, SOURCE)
        )

    def test_small_total_cannot_drop_windows_media(self):
        data = copy.deepcopy(DATA)
        data["payloadProfiles"]["small"]["bandwidthDownGb"] -= 35
        self.assertIn(
            "profile stage sum: small/bandwidthDownGb", guard.validate(data, SOURCE)
        )

    def test_disk_floor_applies_to_large_profile(self):
        data = copy.deepcopy(DATA)
        data["payloadProfiles"]["large"]["diskGb"] = 120
        self.assertIn("profile disk floor: large", guard.validate(data, SOURCE))

    def test_network_caps_are_owner_data(self):
        for name in ["small", "middle", "large", "adhoc"]:
            data = copy.deepcopy(DATA)
            data["payloadProfiles"][name]["maxInetUsdPerGb"] = -1
            self.assertIn("profile per-GB cap: " + name, guard.validate(data, SOURCE))

    def test_profile_disk_nan_is_refused(self):
        data = copy.deepcopy(DATA)
        data["payloadProfiles"]["large"]["diskGb"] = float("nan")
        self.assertTrue(guard.validate(data, SOURCE))

    def test_profile_disk_infinity_is_refused(self):
        data = copy.deepcopy(DATA)
        data["payloadProfiles"]["middle"]["diskGb"] = float("inf")
        self.assertTrue(guard.validate(data, SOURCE))

    def test_spec_required_rows_missing_is_refused(self):
        data = copy.deepcopy(DATA)
        del data["stageSlots"]["specviewperf"]["requiredRowsPerDie"]
        self.assertTrue(guard.validate(data, SOURCE))

    def test_spec_required_rows_empty_is_refused(self):
        data = copy.deepcopy(DATA)
        data["stageSlots"]["specviewperf"]["requiredRowsPerDie"] = []
        self.assertTrue(guard.validate(data, SOURCE))

    def test_spec_lanes_missing_is_refused(self):
        data = copy.deepcopy(DATA)
        del data["stageSlots"]["specviewperf"]["lanes"]
        self.assertTrue(guard.validate(data, SOURCE))

    def test_spec_lanes_empty_is_refused(self):
        data = copy.deepcopy(DATA)
        data["stageSlots"]["specviewperf"]["lanes"] = []
        self.assertTrue(guard.validate(data, SOURCE))


if __name__ == "__main__":
    unittest.main()
