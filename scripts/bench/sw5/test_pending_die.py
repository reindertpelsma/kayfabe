"""A rental never repeats a completed die or promotes a marketing forecast."""

import unittest

import pending_die


class PendingDie(unittest.TestCase):
    def test_ga104_is_already_covered_by_sw3(self):
        with self.assertRaises(ValueError):
            pending_die.require_pending({"die": "GA104"}, [])

    def test_blackwell_stays_outside_sw5(self):
        with self.assertRaises(ValueError):
            pending_die.require_pending({"die": "GB202"}, [])

    def test_completed_ga102_is_not_repeated(self):
        with self.assertRaises(ValueError):
            pending_die.require_pending({"die": "GA102"}, ["GA102"])

    def test_completed_ad106_is_not_repeated(self):
        with self.assertRaises(ValueError):
            pending_die.require_pending({"die": "AD106"}, ["GA102", "AD106"])

    def test_actual_ga106_is_eligible(self):
        binding = {"die": "GA106", "product_name": "NVIDIA GeForce RTX 3060"}
        self.assertEqual(pending_die.require_pending(binding, ["GA102"]), binding)

    def test_a_different_pending_actual_die_is_eligible(self):
        binding = {"die": "AD103", "product_name": "advisory only"}
        self.assertEqual(pending_die.require_pending(binding, ["GA102"]), binding)


if __name__ == "__main__":
    unittest.main()
