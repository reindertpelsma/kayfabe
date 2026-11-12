import unittest

from tests import helpers  # noqa: F401
from kfml import registry, rows, versions, vram
from kfml.spec import Spec, Upstream, guest_estimate_s, tier_problems

SHA = "a" * 40


class Core(unittest.TestCase):
    def test_versions(self):
        self.assertEqual(versions.parse("2.7.1+cu128"), (2, 7, 1))
        self.assertTrue(versions.ge("2.7.1+cu128", "2.7"))
        self.assertTrue(versions.ge("2.12.1", "2.7"))
        self.assertFalse(versions.ge("2.6.9", "2.7"))
        self.assertFalse(versions.ge(None, "2.7"))

    def test_vram_scale(self):
        self.assertEqual(vram.scale_factor(8, 4), 1.0)
        self.assertEqual(vram.scale_factor(1, 4), 1.0)  # never below the floor size
        self.assertEqual(vram.scale_factor(80, 4, max_scale=8), 8.0)
        self.assertAlmostEqual(vram.scale_factor(24, 4), 3.0)
        self.assertEqual(vram.pow2_floor(1000), 512)

    def test_upstream_pin_rules(self):
        for bad in ("master", "v1.0", "abc123", "A" * 40, SHA[:39]):
            with self.assertRaises(ValueError):
                Upstream("x", "https://e/x", bad, "MIT", "fetched")
        with self.assertRaises(ValueError):
            Upstream("x", "https://e/x", SHA, "GPL-3.0-only", "vendored", vendor_dir="x")
        with self.assertRaises(ValueError):
            Upstream("x", "https://e/x", SHA, "MIT", "vendored")  # no vendor_dir
        Upstream("x", "https://e/x", SHA, "Apache-2.0", "vendored", vendor_dir="x")

    def test_spec_floor(self):
        with self.assertRaises(ValueError):
            Spec("a", "m", "d", "mid", 10, min_vram_gb=4)
        with self.assertRaises(ValueError):
            Spec("a", "m", "d", "huge", 10)

    def test_tier_derivation(self):
        lb = Spec("lb", "m", "d", "mid", 10, launch_bound=True)
        big = Spec("big", "m", "d", "mid", 10)
        self.assertEqual(guest_estimate_s(lb, {"lb": 100}), 400.0)
        self.assertEqual(guest_estimate_s(big, {"big": 100}), 150.0)
        self.assertIsNone(guest_estimate_s(big, {}))
        self.assertEqual(tier_problems([lb, big], {"lb": 100, "big": 100}), [])
        p = tier_problems([lb], {"lb": 130})  # 520 > 480
        self.assertTrue(any("lb" in x for x in p))
        many = [Spec("s%d" % i, "m", "d", "mid", 10) for i in range(4)]
        p = tier_problems(many, {s.name: 200 for s in many})  # 4*300 = 1200 > 900
        self.assertTrue(any("total" in x for x in p))
        large = Spec("l", "m", "d", "large", 10, launch_bound=True)
        self.assertEqual(tier_problems([large], {"l": 1000}), [])

    def test_registry_specs_sane(self):
        specs = registry.all_specs()
        self.assertEqual(len({s.name for s in specs}), len(specs))
        for s in specs:
            self.assertGreaterEqual(s.min_vram_gb, 8.0)
            self.assertTrue(s.download_note or not s.gate)
            for u in s.upstreams:
                self.assertEqual(len(u.commit), 40)
        self.assertEqual([s.name for s in specs if not s.gate], ["managed_oversub"])
        self.assertEqual(tier_problems(specs, registry.load_timings()), [])

    def test_row_shape(self):
        e = helpers.fake_env()
        r = rows.make_row("t", "PASS", "", 1.23456, e, {"a": 1})
        for k in ("name", "status", "reason", "wall_s", "gpu_name", "compute_cap", "vram_gb", "driver",
                  "torch", "cuda", "metrics"):
            self.assertIn(k, r)
        self.assertEqual(r["compute_cap"], "8.9")
        self.assertEqual(r["wall_s"], 1.235)
        for k in ("cudnn", "triton", "lock_sha256", "upstream_commits", "patch_sha256", "dataset_sha256"):
            self.assertIn(k, r)
        with self.assertRaises(ValueError):
            rows.make_row("t", "MAYBE", "", 0, e)
        import json
        json.dumps(r)


if __name__ == "__main__":
    unittest.main()
