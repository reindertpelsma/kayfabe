# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Falsifier: empty output, a stale/wrong reference, CPU inference, or a missing package passes.

All fixtures are synthetic; these tests do not qualify software on a GPU.
"""
import copy
import hashlib
import json
import math
from pathlib import Path
import unittest

import helpers as H
import sys
sys.path.insert(0, str(Path(H.AM) / "guest/py"))
import release_oracles as O


class Oracles(unittest.TestCase):
    pixels = [0.1, 0.2, 0.3, 1.0, 0.4, 0.5, 0.6, 1.0] * 2

    def test_image_exact_match_and_small_difference_pass(self):
        self.assertEqual(O.compare_pixels(self.pixels, self.pixels, 2, 2)["mae"], 0)
        near = [x + 0.001 if i % 4 != 3 else x for i, x in enumerate(self.pixels)]
        self.assertLess(O.compare_pixels(near, self.pixels, 2, 2)["mae"], 1 / 255)

    def test_empty_wrong_size_and_black_images_cannot_pass(self):
        for actual, ref in (([], []), (self.pixels[:-1], self.pixels), ([0, 0, 0, 1] * 4, [0, 0, 0, 1] * 4),
                            ([.1, .2, .3, 1] * 4, [.1, .2, .3, 1] * 4)):
            with self.assertRaises(ValueError):
                O.compare_pixels(actual, ref, 2, 2)

    def test_image_mismatch_and_nonfinite_values_fail(self):
        for bad in (0.9, math.nan, math.inf, -math.inf):
            actual = list(self.pixels)
            actual[0] = bad
            with self.assertRaises(ValueError):
                O.compare_pixels(actual, self.pixels, 2, 2)

    def test_image_limits_apply_to_mean_and_single_pixel_error(self):
        for actual in ([x + .01 if i % 4 != 3 else x for i, x in enumerate(self.pixels)],
                       [self.pixels[0] + .04] + self.pixels[1:]):
            with self.assertRaises(ValueError):
                O.compare_pixels(actual, self.pixels, 2, 2)

    def test_image_dimensions_cannot_be_relabelled(self):
        with self.assertRaises(ValueError):
            O.compare_pixels(self.pixels, self.pixels, 3, 2)

    def test_nonfinite_reference_is_rejected(self):
        ref = list(self.pixels)
        ref[1] = math.nan
        with self.assertRaises(ValueError):
            O.compare_pixels(self.pixels, ref, 2, 2)

    def test_spatially_flat_coloured_output_is_not_a_scene(self):
        flat = [.1, .2, .3, 1] * 4
        with self.assertRaises(ValueError):
            O.compare_pixels(flat, flat, 2, 2)

    def test_a_localized_error_cannot_hide_in_a_large_image_mean(self):
        ref = self.pixels * 32
        actual = list(ref)
        actual[0] += .04
        with self.assertRaises(ValueError):
            O.compare_pixels(actual, ref, 16, 8)

    def test_mean_error_cannot_hide_below_the_individual_pixel_limit(self):
        actual = [x + .02 if i % 4 != 3 else x for i, x in enumerate(self.pixels)]
        with self.assertRaises(ValueError):
            O.compare_pixels(actual, self.pixels, 2, 2)

    def test_llm_parity_compares_nonempty_normalized_text(self):
        text = "A fixed prompt produced a nonempty deterministic answer."
        self.assertEqual(O.compare_text(text, text.replace(" ", "\n")), hashlib.sha256(text.encode()).hexdigest())

    def test_empty_short_and_different_llm_outputs_fail(self):
        for actual, ref in (("", ""), ("ok", "ok"), ("A sufficiently long wrong answer.", "A sufficiently long right answer.")):
            with self.assertRaises(ValueError):
                O.compare_text(actual, ref)

    def test_reference_requires_bare_metal_identity_contract_and_hash(self):
        raw = b"synthetic reference bytes"
        doc = dict(schema=1, kind="bare-metal", revision="1" * 40, date="2026-11-07", evidence="synthetic/test-only",
                   sha256=hashlib.sha256(raw).hexdigest(), backend="CUDA", seed=1)
        O.check_reference(doc, {"backend": "CUDA", "seed": 1}, raw)
        for key, value in (("kind", "guest"), ("revision", ""), ("date", ""), ("evidence", ""),
                           ("sha256", "0" * 64), ("backend", "OPTIX"), ("seed", 2)):
            bad = dict(doc, **{key: value})
            with self.subTest(key=key), self.assertRaises(ValueError):
                O.check_reference(bad, {"backend": "CUDA", "seed": 1}, raw)

    def test_reference_never_accepts_missing_provenance_or_empty_bytes(self):
        with self.assertRaises(ValueError):
            O.check_reference({}, {}, b"")
        raw = b"reference"
        doc = dict(schema=1, kind="bare-metal", revision="1" * 40, date="2026-11-07", evidence="test-only",
                   sha256=hashlib.sha256(raw).hexdigest(), seed=1)
        for key in list(doc):
            bad = dict(doc)
            del bad[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                O.check_reference(bad, {"seed": 1}, raw)

    def test_llm_requires_positive_full_cuda_offload_and_nvidia(self):
        O.check_llm_run(0, "Device 0: NVIDIA GeForce RTX 4090\noffloaded 29/29 layers to GPU")
        for rc, log in ((1, "Device 0: NVIDIA\noffloaded 29/29 layers to GPU"), (0, ""),
                        (0, "Device 0: NVIDIA\noffloaded 0/29 layers to GPU"),
                        (0, "Device 0: NVIDIA\noffloaded 1/29 layers to GPU"),
                        (0, "Device 0: CPU\noffloaded 29/29 layers to GPU")):
            with self.assertRaises(ValueError):
                O.check_llm_run(rc, log)

    def test_cpu_and_failed_inference_cannot_be_parity(self):
        for rc in (0, 1):
            with self.assertRaises(ValueError):
                O.check_llm_run(rc, "CPU generation completed with the expected answer")


class Definitions(unittest.TestCase):
    def setUp(self):
        self.apps = {a["id"]: a for a in H.load_apps()["apps"]}
        self.root = Path(H.AM)

    def test_every_approved_definition_is_unverified_and_bounded(self):
        required = {"blender_cycles", "llama_cuda_parity", "nvenc_h264", "nvdec_h264", "d3d12_dxr",
                    "d3d12_mesh_shaders", "d3d12_vrs", "d3d12_sampler_feedback", "gravitymark_d3d11",
                    "gravitymark_d3d12", "gravitymark_vk", "gravitymark_gl", "edge_webgl_conformance",
                    "edge_webgpu_conformance", "deviceQuery_demo", "bandwidthTest_demo", "torch_release_smoke"}
        prof = json.loads((self.root / "gate_profiles.json").read_text())
        self.assertEqual(set(prof["release_software"]["apps"]), required)
        for name in required:
            app = self.apps[name]
            self.assertEqual(app["release"]["status"], "unverified", name)
            self.assertTrue(app["release"]["verdict"] and app["release"]["classification"], name)
            self.assertGreater(app["timeout_s"], app["expect_s"], name)
            self.assertTrue(app["proof"] and app["rx"], name)

    def test_no_licensed_gui_bench_is_added_to_release_profile(self):
        prof = json.loads((self.root / "gate_profiles.json").read_text())
        for name in prof["release_software"]["apps"]:
            self.assertFalse(any(x in name for x in ("heaven", "valley", "superposition", "3dmark")))

    def test_generated_release_metadata_agrees_with_profile_and_packaging(self):
        import gate_profiles as G
        self.assertEqual(G.check(), [])

    def test_release_profile_selects_exactly_the_approved_definitions(self):
        import gate_profiles as G
        cfg = json.loads((self.root / "release_software.json").read_text())
        self.assertEqual(set(G.resolve("release")), set(cfg["apps"]))
        self.assertEqual(G.budget("release"), 18000)

    def test_release_profile_refuses_an_empty_duplicate_or_unknown_list(self):
        import gate_profiles as G
        prof = G.load("gate_profiles.json")
        for names in ([], ["nvidia_smi", "nvidia_smi"], ["unknown_release_app"]):
            prof["release_software"]["apps"] = names
            with self.assertRaises(SystemExit):
                G.resolve("release", prof=prof)

    def test_packaging_metadata_never_claims_hardware_verification(self):
        cfg = json.loads((self.root / "release_software.json").read_text())
        for spec in cfg["apps"].values():
            self.assertEqual(spec["status"], "unverified")

    def test_blender_requires_both_backends_and_image_comparisons(self):
        a = self.apps["blender_cycles"]
        self.assertIn("blender_release.py", a["ps"])
        self.assertIn("CUDA", a["ps"])
        self.assertIn("OPTIX", a["ps"])
        self.assertIn("CYCLES_PARITY_ALL_PASS", a["rx"])
        self.assertNotRegex(a["rx"], r"BLENDER_OK")

    def test_llm_is_fixed_prompt_seed_and_reference_parity(self):
        a = self.apps["llama_cuda_parity"]
        self.assertIn("llama_release.py", a["ps"])
        self.assertIn("LLM_PARITY_PASS", a["rx"])
        self.assertIn("bare-metal", a["release"]["verdict"])

    def test_every_directx_row_requires_survival_and_process_gpu_activity(self):
        for name in ("d3d12_dxr", "d3d12_mesh_shaders", "d3d12_vrs", "d3d12_sampler_feedback"):
            a = self.apps[name]
            self.assertEqual(a["proof"], ["pdh:3D|Compute"])
            self.assertIn("Invoke-KfSurvive", a["ps"])
            self.assertIn("-Seconds 30", a["ps"])
            self.assertEqual(a["session"], "interactive")
            self.assertIn("-Tag " + name, a["ps"])
            self.assertNotIn("warp", a["ps"].lower())

    def test_media_and_cuda_definitions_have_checked_output(self):
        for name in ("nvenc_h264", "nvdec_h264"):
            self.assertIn("frame=", self.apps[name]["rx"])
            self.assertIn("ffmpeg", self.apps[name]["pkgs"])
        self.assertIn("Result = PASS", self.apps["deviceQuery_demo"]["rx"])
        self.assertIn("Result = PASS", self.apps["bandwidthTest_demo"]["rx"])
        self.assertIn("CUDA_RELEASE_PASS", self.apps["torch_release_smoke"]["rx"])

    def test_browser_conformance_uses_offline_known_answer_pages(self):
        for api in ("webgl", "webgpu"):
            app = self.apps["edge_" + api + "_conformance"]
            self.assertIn("-Page " + api + "_conformance", app["ps"])
            self.assertTrue((self.root / "guest/web" / (api + "_conformance.html")).is_file())
            self.assertIn('"ok":true', app["rx"])

    def test_packaging_lists_every_release_app_and_pins_directx_source(self):
        cfg = json.loads((self.root / "release_software.json").read_text())
        self.assertEqual(set(cfg["apps"]), {n for n, a in self.apps.items() if "release" in a})
        self.assertRegex(cfg["directx"]["commit"], r"^[0-9a-f]{40}$")
        self.assertEqual(cfg["directx"]["repository"], "https://github.com/microsoft/DirectX-Graphics-Samples.git")
        for name, spec in cfg["apps"].items():
            self.assertTrue(spec["packages"] or spec["assets"], name)


if __name__ == "__main__":
    unittest.main()
