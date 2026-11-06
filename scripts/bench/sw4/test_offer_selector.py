"""Regression fixtures for the lane's allocation boundary; no provider calls."""

import unittest

from offer_selector import OfferRefused, eligible, require_offer


def offer(**changes):
    row = {
        "id": 57427095,
        "machine_id": 99805,
        "host_id": 70221,
        "verification": "verified",
        "geolocation": "Quebec, CA",
        "vms_enabled": True,
        "is_vm_deverified": False,
        "num_gpus": 1,
        "dph_total": 0.20055555555555554,
    }
    return {**row, **changes}


class OfferSelectorTests(unittest.TestCase):
    def test_verified_direct(self):
        row = offer()
        self.assertIs(require_offer(row, set()), row)

    def test_verified_in_candidate_list(self):
        row = offer()
        self.assertEqual(eligible([row], set()), [row])

    def test_deverified_direct(self):
        with self.assertRaises(OfferRefused):
            require_offer(offer(verification="deverified"), set())

    def test_deverified_removed_from_candidate_list(self):
        refused = offer(id=51156488, machine_id=54516, verification="deverified")
        accepted = offer()
        self.assertEqual(eligible([refused, accepted], set()), [accepted])

    def test_unverified_direct(self):
        with self.assertRaises(OfferRefused):
            require_offer(offer(verification="unverified"), set())

    def test_unverified_removed_from_candidate_list(self):
        refused = offer(id=1, verification="unverified")
        self.assertEqual(eligible([refused], set()), [])

    def test_blacklisted_machine_direct(self):
        with self.assertRaises(OfferRefused):
            require_offer(offer(), {99805})

    def test_blacklisted_machine_removed_from_candidate_list(self):
        refused = offer()
        accepted = offer(id=50694293, machine_id=45428)
        self.assertEqual(eligible([refused, accepted], {99805}), [accepted])

    def test_vm_deverified_direct(self):
        with self.assertRaises(OfferRefused):
            require_offer(offer(is_vm_deverified=True), set())

    def test_vm_deverified_removed_from_candidate_list(self):
        self.assertEqual(eligible([offer(is_vm_deverified=True)], set()), [])

    def test_missing_vm_status_direct(self):
        row = offer()
        del row["is_vm_deverified"]
        with self.assertRaises(OfferRefused):
            require_offer(row, set())

    def test_missing_vm_status_removed_from_candidate_list(self):
        row = offer()
        del row["is_vm_deverified"]
        self.assertEqual(eligible([row], set()), [])

    def test_missing_verification_direct(self):
        row = offer()
        del row["verification"]
        with self.assertRaises(OfferRefused):
            require_offer(row, set())

    def test_missing_verification_removed_from_candidate_list(self):
        row = offer()
        del row["verification"]
        self.assertEqual(eligible([row], set()), [])


if __name__ == "__main__":
    unittest.main()
