# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""gate_profiles.json against apps.json and manifest.json: what the release gate runs, and what it never does."""
import json
import os
import unittest

import helpers as H
import gate_profiles as G

# packages whose terms do not allow us to put them in our storage (docs/DOWNLOADS.md): never in the R2 bundle
NEVER_OPEN = {"gravitymark", "furmark2", "gputest", "heaven", "valley", "vulkan_sdk", "ffmpeg"}


class GateProfiles(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.apps, cls.man, cls.prof = H.load_apps(), H.load_manifest(), G.load("gate_profiles.json")

    def test_consistency_check_is_clean(self):
        self.assertEqual(G.check(), [])

    def test_the_bundle_never_holds_a_package_without_redistribution(self):
        self.assertFalse(NEVER_OPEN & set(self.prof["open_packages"]))
        for p in self.man["packages"]:
            if p["id"] in self.prof["open_packages"]:
                txt = (p["redistribution"] + " " + p["license"]).lower()
                self.assertTrue("redistributable" in txt, p["id"])
                self.assertNotIn("not granted", txt, p["id"])
                self.assertNotIn("not redistributed", txt, p["id"])

    def test_every_app_is_in_exactly_one_class(self):
        opn, owner, manual = G.classes(self.apps, self.man, self.prof)
        ids = {a["id"] for a in self.apps["apps"]}
        self.assertEqual(opn | owner | manual, ids)
        self.assertEqual(len(opn) + len(owner) + len(manual), len(ids))

    def test_no_open_app_needs_a_package_outside_the_bundle(self):
        needs = {p["id"]: p.get("needs", []) for p in self.man["packages"]}
        for x in G.resolve("full", self.apps, self.man, self.prof):
            app = next(a for a in self.apps["apps"] if a["id"] == x)
            self.assertLessEqual(G.closure(app["pkgs"], needs), set(self.prof["open_packages"]), x)

    def test_small_and_mid_are_8_to_10_representative_apps_and_nested(self):
        sm, mid = G.resolve("small", self.apps, self.man, self.prof), G.resolve("mid", self.apps, self.man, self.prof)
        self.assertTrue(8 <= len(sm) <= 10 and 8 <= len(mid) <= 10)
        self.assertTrue(set(sm) <= set(mid))
        cats = {a["id"]: a["category"] for a in self.apps["apps"]}
        self.assertGreaterEqual(len({cats[x] for x in sm}), 5)
        for x in sm + mid:                        # nothing interactive-heavy or long in the smoke profiles
            app = next(a for a in self.apps["apps"] if a["id"] == x)
            self.assertLessEqual(app["timeout_s"], 300, x)

    def test_full_is_the_whole_open_set_minus_documented_exclusions(self):
        full = G.resolve("full", self.apps, self.man, self.prof)
        opn = G.open_apps(self.apps, self.man, self.prof)
        self.assertEqual(set(full), set(opn) - set(self.prof["excluded"]))
        for x, why in self.prof["excluded"].items():
            self.assertTrue(why and len(why) > 20, x)

    def test_an_explicit_list_must_name_known_apps(self):
        self.assertEqual(G.resolve("ids:nvidia_smi,vectorAdd_demo", self.apps, self.man, self.prof), ["nvidia_smi", "vectorAdd_demo"])
        with self.assertRaises(SystemExit):
            G.resolve("ids:nope", self.apps, self.man, self.prof)
        with self.assertRaises(SystemExit):
            G.resolve("ids:", self.apps, self.man, self.prof)
        with self.assertRaises(SystemExit):
            G.resolve("everything", self.apps, self.man, self.prof)

    def test_manual_list_has_an_evidence_template_and_the_licensed_items(self):
        self.assertIn("evidence_template", self.prof["manual"])
        ids = {i["id"] for i in self.prof["manual"]["items"]}
        self.assertTrue({"3dmark", "geekbench6", "tensorrt"} <= ids)

    def test_budgets_grow_with_the_profile(self):
        b = [G.budget(x, self.prof) for x in ("small", "mid", "full")]
        self.assertEqual(b, sorted(b))
        self.assertEqual(G.budget("ids:nvidia_smi", self.prof), b[2])

    def test_november_full_run_failures_stay_in_full(self):
        failed = {"managed_t", "nbody_demo_gl", "overlay_yuv_probe"} | {
            "overlay_yuv_" + x for x in ("hide", "occlude", "resize", "move", "recreate", "fullscreen")}
        self.assertLessEqual(failed, set(G.resolve("full", self.apps, self.man, self.prof)))
        self.assertFalse(failed & set(self.prof["excluded"]))

    def test_overlay_applicability_is_not_resolved_by_reclassification(self):
        opn, owner, manual = G.classes(self.apps, self.man, self.prof)
        for name in ("managed_t", "nbody_demo_gl", "overlay_yuv_probe", "overlay_yuv_soak"):
            self.assertIn(name, opn)
            self.assertNotIn(name, owner | manual)


if __name__ == "__main__":
    unittest.main()
