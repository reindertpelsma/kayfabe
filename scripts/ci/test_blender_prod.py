"""Blender production lane fixtures. Uses the standard library and no Blender build."""

import contextlib
import io
import json
from pathlib import Path
import struct
import sys
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "apps" / "blender_prod"))
import blender_case
import image_compare
import runner


def tga(path, pixels, width=1, flags=0x20):
    header = struct.pack("<BBBHHBHHHHBB", 0, 0, 2, 0, 0, 0, 0, 0,
                         width, len(pixels) // (width * 3), 24, flags)
    path.write_bytes(header + bytes(pixels))
    return path


def marker(status="pass", **kv):
    return runner.MARKER + json.dumps(dict(status=status, reason="-", kv=kv))


class Fixtures(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.case = dict(case="render/shot", area="render", value="shot")
        self.hosts = [self.root / "host-1", self.root / "host-2"]
        self.guest = self.root / "guest"
        for directory in [*self.hosts, self.guest]:
            directory.mkdir()
            (directory / "inputs.json").write_text('{"asset":"synthetic"}')

    def parse(self, code, log, directory=None):
        return runner.result(self.case, code, log, directory or self.guest, 10, 12, 2)

    def parity(self, host1, host2, guest, margin=0):
        return runner.compare_case(self.case, [host1, host2], guest, self.hosts, self.guest, margin)


class Inventory(unittest.TestCase):
    def test_keeps_all_new_feature_identifiers(self):
        rows = blender_case.case_list({"modifier": ["CLOTH", "FUTURE_MODIFIER", "CLOTH"],
                                       "geometry_node": ["GeometryNodeSimulate", "FunctionNodeExtra"]})
        self.assertEqual([row["case"] for row in rows],
                         ["geometry_node/FunctionNodeExtra", "geometry_node/GeometryNodeSimulate",
                          "modifier/CLOTH", "modifier/FUTURE_MODIFIER"])

    def test_one_identifier_is_a_case(self):
        self.assertEqual(blender_case.case_list({"bake": ["UNRECOGNIZED"]}),
                         [{"case": "bake/UNRECOGNIZED", "area": "bake", "value": "UNRECOGNIZED"}])

    def test_gpu_proof_and_denoiser_selection(self):
        rows = {row["case"]: row for row in blender_case.case_list(
            {"device": ["CPU", "HIP"], "denoiser": ["OPTIX", "OPENIMAGEDENOISE"],
             "gpu_backend": ["VULKAN"], "engine": ["BLENDER_EEVEE"]})}
        self.assertNotIn("gpu_proof", rows["device/CPU"])
        self.assertEqual(rows["device/HIP"]["gpu_proof"], "HIP")
        self.assertEqual(rows["denoiser/OPTIX"]["device"], "OPTIX")
        self.assertEqual(rows["denoiser/OPTIX"]["gpu_proof"], "OPTIX_DENOISE")
        self.assertNotIn("gpu_proof", rows["denoiser/OPENIMAGEDENOISE"])
        self.assertEqual(rows["gpu_backend/VULKAN"]["gpu_proof"], "VULKAN")
        self.assertEqual(rows["engine/BLENDER_EEVEE"]["gpu_proof"], "BLENDER_EEVEE")

    def test_api_inventory_queries_dynamic_devices_and_node_tree_poll(self):
        def property_(*values):
            return NS(enum_items=[], enum_items_static=[NS(identifier=item) for item in values])

        def rna(**properties):
            return NS(bl_rna=NS(properties={name: property_(*values)
                                             for name, values in properties.items()}))

        class Engine:
            pass

        class Cycles(Engine):
            bl_idname = "CYCLES"

        class Node:
            @classmethod
            def is_registered_node_type(cls):
                return cls is not Node

        class Shared(Node):
            bl_rna = NS(identifier="FunctionNodeFixture")

            @classmethod
            def poll(cls, tree):
                return tree == "GeometryNodeTree"

        types = NS(RenderEngine=Engine, Node=Node, Shared=Shared,
                   Modifier=rna(type=["CLOTH", "FLUID"]), FieldSettings=rna(type=["WIND"]),
                   RigidBodyObject=rna(type=["ACTIVE"], collision_shape=["BOX"]),
                   RigidBodyConstraint=rna(type=["HINGE"]), FluidDomainSettings=rna(domain_type=["GAS"]),
                   FluidFlowSettings=rna(flow_type=["SMOKE"]), View3DShading=rna(type=["RENDERED"]),
                   Object=rna(type=["CURVES", "VOLUME", "GREASEPENCIL"]))
        groups_removed = []
        prefs = NS(system=rna(gpu_backend=["VULKAN"]),
                   addons={"cycles": NS(preferences=NS(get_device_types=lambda _: [("HIP",), ("NONE",)]))})
        bpy = NS(types=types, data=NS(node_groups=NS(new=lambda _, kind: kind, remove=groups_removed.append)),
                 context=NS(preferences=prefs, scene=NS(render=rna(engine=["BLENDER_EEVEE"]),
                                                       cycles=rna(denoiser=[], bake_type=["COMBINED"]))),
                 ops=NS(sequencer=NS(effect_strip_add=NS(get_rna_type=lambda: NS(
                     properties={"type": NS(enum_items=[NS(identifier="GLOW")])})))))
        result = blender_case.inventory(bpy)
        self.assertEqual(result["engine"], {"BLENDER_EEVEE", Cycles.bl_idname})
        self.assertEqual(result["device"], {"CPU", "HIP"})
        self.assertIn("OPTIX", result["denoiser"])
        self.assertEqual(result["geometry_node"], ["FunctionNodeFixture"])
        self.assertEqual(result["shader_node"], [])
        self.assertEqual(result["sequencer_effect"], ["GLOW"])
        self.assertEqual(result["fluid_flow"], ["SMOKE"])
        self.assertEqual(len(groups_removed), 3)

    def test_unmapped_contextual_feature_stays_visible(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            blender_case.execute(None, {"area": "modifier"}, None)
        self.assertIn('"status": "not_run"', output.getvalue())

    def test_cycles_disables_cpu_and_enables_optix_denoising(self):
        cpu, optix = NS(type="CPU", use=True), NS(type="OPTIX", use=False)
        prefs = NS(devices=[cpu, optix], refresh_devices=lambda: None)
        scene = NS(cycles=NS())
        self.assertTrue(blender_case.configure_cycles(scene, prefs, "OPTIX", "OPTIX"))
        self.assertFalse(cpu.use)
        self.assertTrue(optix.use)
        self.assertEqual(scene.cycles.device, "GPU")
        self.assertTrue(scene.cycles.use_denoising)
        self.assertTrue(scene.cycles.denoising_use_gpu)
        self.assertEqual(scene.cycles.denoiser, "OPTIX")
        self.assertFalse(blender_case.configure_cycles(scene, prefs, "HIP", None))


class ImageTests(Fixtures):
    def test_zero_margin_rejects_one_channel_lsb(self):
        h1 = tga(self.hosts[0] / "render.tga", [0, 0, 0])
        h2 = tga(self.hosts[1] / "render.tga", [0, 0, 0])
        g = tga(self.guest / "render.tga", [0, 1, 0])
        result = image_compare.compare_images([h1, h2], g, 0)
        self.assertFalse(result["match"])
        self.assertEqual(result["distance"], 1 / 255)

    def test_zero_margin_keeps_measured_noise_as_bound(self):
        h1 = tga(self.hosts[0] / "render.tga", [0, 0, 0])
        h2 = tga(self.hosts[1] / "render.tga", [0, 2, 0])
        g = tga(self.guest / "render.tga", [0, 3, 0])
        result = image_compare.compare_images([h1, h2], g, 0)
        self.assertEqual(result["limit"], 2 / 255)
        self.assertFalse(result["match"])
        self.assertTrue(image_compare.compare_images([h1, h2], g, 1 / 255)["match"])

    def test_orientation_is_normalized(self):
        pixels = [2, 0, 0, 0, 2, 0, 0, 0, 2, 3, 0, 0]
        top = tga(self.hosts[0] / "render.tga", pixels, width=2)
        opposite = tga(self.guest / "render.tga",
                       pixels[9:12] + pixels[6:9] + pixels[3:6] + pixels[:3], width=2, flags=0x10)
        self.assertEqual(image_compare.max_abs(top, opposite), 0)

    def test_rejects_bad_input_and_short_raster(self):
        host = tga(self.hosts[0] / "render.tga", [0, 0, 0])
        broken = self.guest / "render.tga"
        broken.write_bytes(host.read_bytes()[:-1])
        with self.assertRaisesRegex(ValueError, "truncated TGA"):
            image_compare.max_abs(host, broken)
        with self.assertRaises(ValueError):
            image_compare.compare_images([host], host, 0)
        with self.assertRaises(ValueError):
            image_compare.compare_images([host, host], host, float("nan"))

    def test_checks_every_animation_frame(self):
        self.case["frames"] = [1, 2]
        for directory in [*self.hosts, self.guest]:
            tga(directory / "frame-0001.tga", [0, 0, 0])
            tga(directory / "frame-0002.tga", [0, 0, directory == self.guest])
        rows = [self.parse(0, marker(), directory) for directory in [*self.hosts, self.guest]]
        result = self.parity(*rows)
        self.assertEqual([frame["match"] for frame in result["images"]], [True, False])
        self.assertFalse(result["match"])
        (self.guest / "frame-0001.tga").unlink()
        self.assertEqual(self.parse(0, marker())["kv"]["error_class"], "artifact")


class OutcomeTests(Fixtures):
    def test_cuda_and_optix_oom_disagree(self):
        cuda = self.parse(17, "CUDA_ERROR_OUT_OF_MEMORY")
        optix = self.parse(17, "OPTIX_ERROR_DEVICE_OUT_OF_MEMORY")
        self.assertEqual(cuda["kv"]["error_class"], "cuda_oom")
        self.assertEqual(optix["kv"]["error_class"], "optix_oom")
        self.assertFalse(self.parity(cuda, cuda, optix)["match"])

    def test_generic_oom_does_not_substitute_for_driver_oom(self):
        generic = self.parse(0, "Error: Out of memory")
        vulkan = self.parse(0, "VK_ERROR_OUT_OF_DEVICE_MEMORY")
        self.assertTrue(self.parity(generic, generic, generic)["match"])
        self.assertFalse(self.parity(vulkan, vulkan, generic)["match"])

    def test_timeout_signal_exit_and_missing_marker(self):
        self.assertEqual(self.parse(None, "")["status"], "timeout")
        self.assertEqual(self.parse(-11, "out of memory")["kv"]["error_class"], "signal_11")
        self.assertEqual(self.parse(17, marker())["kv"]["error_class"], "exit")
        self.assertEqual(self.parse(0, "")["kv"]["error_class"], "protocol")
        self.assertEqual(self.parse(0, marker() + "\n" + marker())["kv"]["error_class"], "protocol")

    def test_missing_image_and_unstable_host(self):
        broken = self.parse(0, marker())
        unavailable = self.parse(0, marker("not_supported"))
        self.assertEqual(broken["kv"]["error_class"], "artifact")
        self.assertIsNone(self.parity(broken, unavailable, unavailable)["match"])

    def test_host_repeat_cannot_reuse_guest_or_host_artifacts(self):
        row = self.parse(0, marker("not_supported"))
        for guest in [self.guest, self.hosts[0]]:
            hosts = [self.hosts[0], self.hosts[0]] if guest == self.guest else self.hosts
            with self.assertRaisesRegex(ValueError, "distinct"):
                runner.compare_case(self.case, [row, row], row, hosts, guest, 0)

    def test_requires_matching_provenance(self):
        same = self.parse(17, "CUDA_ERROR_OUT_OF_MEMORY")
        (self.guest / "inputs.json").write_text('{"asset":"changed"}')
        self.assertIsNone(self.parity(same, same, same)["match"])

    def test_gpu_proof_window_and_backend(self):
        self.case.update(image=False, gpu_proof="OPTIX_DENOISE")
        rows = [self.parse(0, marker())] * 3
        self.assertIsNone(self.parity(*rows)["match"])
        proof = dict(backend="OPTIX_DENOISE", evidence="synthetic API trace", t0=10, t1=11)
        for directory in [*self.hosts, self.guest]:
            (directory / "gpu-proof.json").write_text(json.dumps(proof))
        self.assertTrue(self.parity(*rows)["match"])
        for change in (dict(backend="OPTIX"), dict(t1=13)):
            (self.guest / "gpu-proof.json").write_text(json.dumps({**proof, **change}))
            self.assertIsNone(self.parity(*rows)["match"])


class RunnerTests(Fixtures):
    def test_command_orders_scene_and_script_and_sets_backend(self):
        case = dict(self.case, area="gpu_backend", value="VULKAN", gui=True)
        cmd = runner.command("blender", "/asset/shot.blend", case, self.guest)
        self.assertNotIn("--background", cmd)
        self.assertIn("--disable-autoexec", cmd)
        self.assertLess(cmd.index("--gpu-backend"), cmd.index("/asset/shot.blend"))
        self.assertLess(cmd.index("/asset/shot.blend"), cmd.index("--python"))
        self.assertEqual(cmd[cmd.index("--gpu-backend") + 1], "vulkan")

    def test_wrapper_runs_local_child_and_applies_budget(self):
        asset = self.root / "input.blend"
        asset.write_bytes(b"fixture")
        child = self.root / "fixture-blender"
        child.write_text(f"#!{sys.executable}\nprint({marker()!r})\n")
        child.chmod(0o700)
        config = dict(blender=str(child), seconds=2, asset={"path": str(asset)})
        self.case["image"] = False
        row = runner.run(config, self.case, self.root / "run-ok")
        self.assertEqual(row["status"], "pass")
        self.assertLessEqual(row["t0"], row["t1"])
        self.assertTrue((self.root / "run-ok" / "inputs.json").is_file())
        child.write_text(f"#!{sys.executable}\nimport time\ntime.sleep(20)\n")
        timed = runner.run(dict(config, seconds=0.1), self.case, self.root / "run-timeout")
        self.assertEqual(timed["status"], "timeout")

    def test_asset_pin_mismatch_never_launches_blender(self):
        asset = self.root / "input.blend"
        asset.write_bytes(b"fixture")
        config = dict(blender="does-not-exist", seconds=1,
                      asset={"path": str(asset), "sha256": "0" * 64})
        row = runner.run(config, self.case, self.root / "bad-pin")
        self.assertEqual(row["status"], "not_run")
        self.assertIn("SHA-256", row["reason"])
        self.assertIsNone(row["t0"])

    def test_fetch_refuses_unofficial_redirect_before_request(self):
        with patch.object(runner.subprocess, "run", return_value=NS(
                stdout="302\nhttps://example.com/file.blend")) as launch:
            with self.assertRaisesRegex(ValueError, "official"):
                runner.fetch("https://studio.blender.org/file.blend", self.guest / "asset.blend")
        self.assertEqual(launch.call_count, 1)


if __name__ == "__main__":
    unittest.main()
