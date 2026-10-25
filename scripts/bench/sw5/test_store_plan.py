"""Sweep store sizing preserves host headroom on Turing and larger adapters."""

import unittest

import store_plan


class StorePlan(unittest.TestCase):
    def test_six_gib_turing_uses_four_gib(self):
        self.assertEqual(store_plan.framebuffer_mib(6144), 4096)

    def test_eight_gib_turing_uses_four_gib(self):
        self.assertEqual(store_plan.framebuffer_mib(8192), 4096)

    def test_twelve_gib_ampere_uses_eight_gib(self):
        self.assertEqual(store_plan.framebuffer_mib(12288), 8192)

    def test_sixteen_gib_ada_uses_eight_gib(self):
        self.assertEqual(store_plan.framebuffer_mib(16380), 8192)

    def test_four_gib_cannot_reserve_headroom(self):
        with self.assertRaises(ValueError):
            store_plan.framebuffer_mib(4096)

    def test_five_gib_cannot_reserve_headroom(self):
        with self.assertRaises(ValueError):
            store_plan.framebuffer_mib(5120)


if __name__ == "__main__":
    unittest.main()
