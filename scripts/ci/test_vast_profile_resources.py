"""Falsifiers: an added download stage or understated profile total passes the cost guard."""
import copy
import json
from pathlib import Path
import sys
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import profile_resources as guard

DATA = json.loads((guard.ROOT / 'ci/vast-broker/defaults.json').read_text())
SOURCE = (HERE / 'vast_payload.py').read_text()


class ProfileResources(unittest.TestCase):
    def test_live_payload_stages_have_budget_rows(self):
        self.assertEqual(guard.validate(DATA, SOURCE), [])

    def test_new_direct_download_stage_is_rejected(self):
        source = SOURCE + '\nmake_stage("extra_download", ["curl", "example.invalid"], None, [], None)\n'
        self.assertIn('unbudgeted payload stage: extra_download', guard.validate(DATA, source))

    def test_new_conditional_download_stage_is_rejected(self):
        source = SOURCE + '\nif enabled:\n    make_stage("media_fetch", ["wget", "example.invalid"], None, [], None)\n'
        self.assertIn('unbudgeted payload stage: media_fetch', guard.validate(DATA, source))

    def test_opaque_stage_name_is_rejected(self):
        self.assertTrue(guard.validate(DATA, SOURCE + '\nmake_stage(dynamic_name, [], None, [], None)\n'))

    def test_large_total_cannot_drop_the_spec_download(self):
        data = copy.deepcopy(DATA)
        data['payloadProfiles']['large']['bandwidthDownGb'] -= 43
        self.assertIn('profile stage sum: large/bandwidthDownGb', guard.validate(data, SOURCE))

    def test_small_total_cannot_drop_windows_media(self):
        data = copy.deepcopy(DATA)
        data['payloadProfiles']['small']['bandwidthDownGb'] -= 35
        self.assertIn('profile stage sum: small/bandwidthDownGb', guard.validate(data, SOURCE))

    def test_disk_floor_applies_to_large_profile(self):
        data = copy.deepcopy(DATA)
        data['payloadProfiles']['large']['diskGb'] = 120
        self.assertIn('profile disk floor: large', guard.validate(data, SOURCE))

    def test_network_caps_are_owner_data(self):
        for name in ['small', 'middle', 'large', 'adhoc']:
            data = copy.deepcopy(DATA)
            data['payloadProfiles'][name]['maxInetUsdPerGb'] = -1
            self.assertIn('profile per-GB cap: ' + name, guard.validate(data, SOURCE))

if __name__ == '__main__':
    unittest.main()
