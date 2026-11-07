#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Owner's 2026-11-07 layer boundary. No network, VM, GPU or real installer."""

import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

IMAGE = Path(__file__).resolve().parents[1] / "apps/image"
sys.path.insert(0, str(IMAGE))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import common
import official
import scan_image


def pin():
    return {
        "id": "driver",
        "kind": "run",
        "version": "580.159.04",
        "url": "https://example.org/driver.run",
        "sha256": hashlib.sha256(b"official").hexdigest(),
        "size": 8,
        "filename": "driver.run",
        "licence": "NVIDIA Software License Agreement",
        "install_argv": ["{file}", "--silent"],
    }


class OfficialFetch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def test_404_is_media_drift(self):
        error = urllib.error.HTTPError(
            "https://example.org/x", 404, "missing", {}, None
        )
        with self.assertRaises(official.MediaDrift):
            official.check_url(
                pin(), opener=lambda *a, **k: (_ for _ in ()).throw(error)
            )

    def test_wrong_length_is_media_drift(self):
        response = io.BytesIO(b"")
        response.status = 200
        response.headers = {"Content-Length": "9"}
        with self.assertRaises(official.MediaDrift):
            official.check_url(pin(), opener=lambda *a, **k: response)

    def test_hash_mismatch_never_passes(self):
        target = self.root / pin()["sha256"]
        target.write_bytes(b"corrupt!")
        with self.assertRaises(official.MediaDrift):
            official.fetch(pin(), self.root, fetch_factory=lambda *a, **k: None)

    def test_corrupt_new_download_is_media_drift(self):
        class Fake:
            def __init__(_, url, out, sha256, **kw):
                _.out = out

            def run(_):
                Path(_.out).write_bytes(b"corrupt!")

        with self.assertRaises(official.MediaDrift):
            official.fetch(pin(), self.root, fetch_factory=Fake)

    def test_pinned_fetcher_receives_official_url_hash_and_size(self):
        seen = []

        class Fake:
            def __init__(_, url, out, sha256, **kw):
                seen.append((url, sha256, kw))
                _.out = out

            def run(_):
                Path(_.out).write_bytes(b"official")

        p = official.fetch(pin(), self.root, fetch_factory=Fake)
        self.assertEqual(p.read_bytes(), b"official")
        self.assertEqual(seen[0][0], pin()["url"])
        self.assertEqual(seen[0][1], pin()["sha256"])
        self.assertEqual(seen[0][2]["expected_size"], 8)
        self.assertFalse(seen[0][2].get("origin_headers"))

    def test_unsuccessful_head_status_cannot_be_healthy(self):
        response = io.BytesIO()
        response.status = 503
        response.headers = {}
        with self.assertRaises(official.MediaDrift):
            official.check_url(pin(), opener=lambda *a, **k: response)

    def test_monthly_size_drift_is_not_success(self):
        response = io.BytesIO()
        response.status = 200
        response.headers = {"Content-Length": "9"}
        with self.assertRaises(official.MediaDrift):
            official.verify_urls([pin()], opener=lambda *a, **k: response)

    def test_monthly_failure_cannot_be_a_success_report(self):
        response = io.BytesIO()
        response.status = 404
        response.headers = {}
        with self.assertRaises(official.MediaDrift):
            official.verify_urls([pin()], opener=lambda *a, **k: response)


class Scan(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def write(self, path, data):
        p = self.root / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        return p

    def test_proprietary_license_header_fails(self):
        self.write("usr/share/license", b"NVIDIA SOFTWARE LICENSE AGREEMENT\nterms\n")
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_proprietary_license_in_binary_fails(self):
        self.write(
            "opt/apps/bundle/bin/renamed",
            b"\x7fELF\0\nNVIDIA END USER LICENSE AGREEMENT\n",
        )
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_nvidia_deb_origin_fails_even_without_license_text(self):
        self.assertFalse(
            scan_image.scan(
                self.root, ["ii\tlibnvidia-compute-580\tamd64\t580.159.04"]
            )["clean"]
        )

    def test_nvidia_python_distribution_fails(self):
        self.write(
            "opt/apps/venv/lib/python3.12/site-packages/nvidia_cudnn_cu12-9.1.dist-info/METADATA",
            b"Name: nvidia-cudnn-cu12\n",
        )
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_cuda_runtime_file_fails_without_header(self):
        self.write("usr/local/cuda/lib64/libcudart.so.12", b"\x7fELF")
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_nvidia_user_library_fails(self):
        self.write("usr/lib/x86_64-linux-gnu/libcuda.so.580.159.04", b"\x7fELF")
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_installed_scanner_source_does_not_trigger_its_own_rules(self):
        self.write(
            "opt/apps/image/scan_image.py", (IMAGE / "scan_image.py").read_bytes()
        )
        report = scan_image.scan(self.root, [])
        self.assertTrue(report["clean"], report["findings"])

    def test_installed_scanner_bytecode_does_not_trigger_its_own_rules(self):
        import marshal

        code = compile((IMAGE / "scan_image.py").read_text(), "scan_image.py", "exec")
        self.write(
            "opt/apps/image/__pycache__/scan_image.cpython-312.pyc", marshal.dumps(code)
        )
        report = scan_image.scan(self.root, [])
        self.assertTrue(report["clean"], report["findings"])

    def test_vendor_library_symlink_is_a_payload_finding(self):
        self.write("opt/open-library", b"x")
        (self.root / "libcudart.so.12").symlink_to("opt/open-library")
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_vendor_distribution_symlink_is_a_payload_finding(self):
        self.write("opt/open-library", b"x")
        (self.root / "nvidia_cublas-12.dist-info").symlink_to("opt/open-library")
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_plain_open_app_is_clean(self):
        self.write("opt/apps/bundle/bin/vectorAdd", b"\x7fELF\0libcudart.so.12\0")
        self.assertTrue(
            scan_image.scan(self.root, ["ii\tpython3\tamd64\t3.12"])["clean"]
        )

    def test_symlink_escape_is_reported_not_read(self):
        (self.root / "escape").symlink_to("../../definitely-outside-image")
        self.assertFalse(scan_image.scan(self.root, [])["clean"])

    def test_report_lists_finding_paths(self):
        self.write("usr/lib/libnvidia-ml.so.1", b"x")
        report = scan_image.scan(self.root, [])
        self.assertIn(
            "/usr/lib/libnvidia-ml.so.1", {x["path"] for x in report["findings"]}
        )


class ScanTraversalErrors(unittest.TestCase):
    def failed_walk(self, error):
        def walk(root, **kwargs):
            if kwargs.get("onerror"):
                kwargs["onerror"](error)
            return iter(())

        with tempfile.TemporaryDirectory() as root, patch.object(
            scan_image.os, "walk", side_effect=walk
        ):
            return scan_image.scan(root, [])

    def test_permission_denied_directory_cannot_certify_a_clean_image(self):
        import errno

        report = self.failed_walk(OSError(errno.EACCES, "denied", "/locked"))
        self.assertFalse(report["clean"])
        self.assertTrue(report["findings"])

    def test_io_error_during_traversal_cannot_certify_a_clean_image(self):
        import errno

        report = self.failed_walk(OSError(errno.EIO, "unreadable", "/damaged"))
        self.assertFalse(report["clean"])
        self.assertTrue(report["findings"])


class ScanIdentity(unittest.TestCase):
    def test_log_byte_growth_is_not_a_change_of_app_inventory(self):
        before = {
            "clean": True,
            "findings": [],
            "notices": [],
            "bytes": 100,
            "files": 3,
        }
        after = {**before, "bytes": 700}
        self.assertEqual(
            scan_image.inventory_result(before), scan_image.inventory_result(after)
        )

    def test_new_cloud_init_instance_files_are_scan_observations(self):
        before = {
            "clean": True,
            "findings": [],
            "notices": [],
            "bytes": 100,
            "files": 3,
        }
        after = {**before, "files": 9}
        self.assertEqual(
            scan_image.inventory_result(before), scan_image.inventory_result(after)
        )

    def test_inventory_scan_identity_retains_every_finding(self):
        report = {
            "clean": False,
            "findings": [{"path": "/libcuda.so", "reason": "payload"}],
            "notices": [],
            "files": 1,
            "bytes": 4,
        }
        result = scan_image.inventory_result(report)
        self.assertFalse(result["clean"])
        self.assertEqual(result["findings"], report["findings"])

    def test_inventory_scan_identity_retains_licence_evidence(self):
        report = {
            "clean": True,
            "findings": [],
            "notices": [{"path": "/source", "licence_evidence": "MIT"}],
            "files": 1,
            "bytes": 4,
        }
        self.assertEqual(
            scan_image.inventory_result(report)["notices"], report["notices"]
        )


class LayerRecipe(unittest.TestCase):
    def setUp(self):
        self.recipe = common.read_recipe(IMAGE / "recipe.json")

    def test_no_nvidia_packages_in_open_artifacts(self):
        for a in self.recipe["artifacts"]:
            self.assertFalse(a.get("package", "").startswith("nvidia-"))
            self.assertNotIn("developer.download.nvidia.com", a["url"])
        self.assertFalse(
            any(a.get("package") == "triton" for a in self.recipe["artifacts"])
        )

    def test_open_torch_is_cpu_and_cuda_torch_is_transient(self):
        open_torch = next(
            a for a in self.recipe["artifacts"] if a.get("package") == "torch"
        )
        runtime_torch = next(
            a for a in self.recipe["official_fetch"] if a.get("package") == "torch"
        )
        self.assertEqual(open_torch["version"], "2.6.0+cpu")
        self.assertEqual(runtime_torch["version"], "2.6.0+cu124")

    def test_open_apt_components_are_main_universe_only(self):
        self.assertEqual(self.recipe["apt"]["components"], ["main", "universe"])
        self.assertTrue(
            all(
                "/restricted/" not in p["path"] and "/multiverse/" not in p["path"]
                for p in self.recipe["apt"]["metadata"]
            )
        )

    def test_restricted_or_multiverse_recipe_is_rejected(self):
        for component in ["restricted", "multiverse"]:
            recipe = copy.deepcopy(self.recipe)
            recipe["apt"]["components"].append(component)
            with self.assertRaises(ValueError):
                common.validate_recipe(recipe)

    def test_restricted_recipe_is_rejected(self):
        recipe = copy.deepcopy(self.recipe)
        recipe["apt"]["components"] = ["main", "restricted"]
        with self.assertRaises(ValueError):
            common.validate_recipe(recipe)

    def test_multiverse_recipe_is_rejected(self):
        recipe = copy.deepcopy(self.recipe)
        recipe["apt"]["components"] = ["main", "multiverse"]
        with self.assertRaises(ValueError):
            common.validate_recipe(recipe)

    def test_nvidia_open_artifact_is_rejected(self):
        recipe = copy.deepcopy(self.recipe)
        recipe["artifacts"].append(recipe["official_fetch"][1])
        recipe["official_fetch"] = recipe["official_fetch"][:1]
        with self.assertRaises(ValueError):
            common.validate_recipe(recipe)

    def test_nvidia_pip_wheel_cannot_be_moved_into_open_layer(self):
        recipe = copy.deepcopy(self.recipe)
        wheel = next(
            a
            for a in recipe["official_fetch"]
            if a.get("package", "").startswith("nvidia-")
        )
        recipe["official_fetch"].remove(wheel)
        recipe["artifacts"].append(wheel)
        with self.assertRaises(ValueError):
            common.validate_recipe(recipe)

    def test_every_official_input_has_pin_licence_and_install_argv(self):
        for a in self.recipe["official_fetch"]:
            common.validate_pin(a)
            self.assertTrue(a["licence"])
            common.check_argv(a["install_argv"])
            self.assertNotIn("github.com", a["url"])
            self.assertNotIn("kayfabe", a["url"])

    def test_no_static_cudart_build_can_enter_open_layer(self):
        for b in self.recipe["builds"]:
            if b["requires"] != "open-layer":
                continue
            for argv in b["commands"]:
                if argv[0].endswith("/nvcc"):
                    self.assertEqual(argv[argv.index("-cudart") + 1], "shared")
                if argv[0] == "make" and b["source"] == "cuda-samples":
                    self.assertTrue(any("--cudart shared" in a for a in argv))
                if b["name"] == "llama" and "-S" in argv:
                    self.assertIn("-DCMAKE_CUDA_RUNTIME_LIBRARY=Shared", argv)
                    self.assertIn("-DGGML_STATIC=OFF", argv)

    def test_cdp_static_device_runtime_is_declared_and_absent_from_open_inventory(self):
        b = next(
            b for b in self.recipe["builds"] if b["name"] == "sample-cdpSimpleQuicksort"
        )
        self.assertEqual(b["requires"], "nvidia-layer-static")
        self.assertNotIn(
            b["outputs"][0]["path"], self.recipe["inventory"]["required_paths"]
        )
        self.assertIn(
            b["outputs"][0]["path"], self.recipe["runtime_inventory"]["required_paths"]
        )

    def test_runtime_lock_contains_hashes_for_nvidia_wheels(self):
        open_lock = (IMAGE / "requirements.lock").read_text()
        runtime = (IMAGE / "requirements-runtime.lock").read_text()
        self.assertNotIn("nvidia-", open_lock)
        self.assertNotIn("triton==", open_lock)
        self.assertIn("nvidia-cudnn-cu12==9.1.0.70 --hash=sha256:", runtime)
        self.assertIn("torch==2.6.0+cu124 --hash=sha256:", runtime)


class LayerInventories(unittest.TestCase):
    def setUp(self):
        import test_apps_image as base

        case = base.Inventory()
        case.setUp()
        self.inv = case.inv
        self.recipe = case.recipe

    def test_dirty_scan_cannot_certify_open_image(self):
        self.inv["nvidia_scan"]["clean"] = False
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_no_nvidia_declaration_cannot_hide_dirty_scan(self):
        self.inv["nvidia_scan"] = {
            "clean": False,
            "findings": [{"path": "/usr/lib/libcuda.so.1"}],
        }
        self.inv["no_nvidia_proprietary_files"] = True
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_clean_flag_cannot_hide_a_reported_vendor_file(self):
        self.inv["nvidia_scan"]["findings"] = [
            {"path": "/usr/lib/libcuda.so.1", "reason": "NVIDIA payload"}
        ]
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_clean_flag_cannot_hide_a_reported_proprietary_header(self):
        self.inv["nvidia_scan"]["findings"] = [
            {"path": "/LICENSE", "reason": "NVIDIA EULA"}
        ]
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_missing_clean_declaration_refused(self):
        self.inv["no_nvidia_proprietary_files"] = False
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_runtime_needs_actual_official_install_receipts(self):
        self.inv["layer"] = "runtime"
        self.inv["guest"]["nvidia_module_version"] = "580.159.04"
        self.recipe["official_fetch"] = [pin()]
        self.inv["official_receipts"] = {}
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_runtime_receipt_hash_cannot_drift(self):
        self.inv["layer"] = "runtime"
        self.inv["guest"]["nvidia_module_version"] = "580.159.04"
        self.recipe["official_fetch"] = [pin()]
        self.inv["official_receipts"] = {"driver": "0" * 64}
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)


class RuntimeContext(unittest.TestCase):
    def setUp(self):
        import test_apps_image as base

        base.FakeQemu.setUp(self)
        inv = copy.deepcopy(self.inv)
        inv["layer"] = "runtime"
        inv["guest"]["nvidia_module_version"] = "580.159.04"
        inv["official_receipts"] = {}
        self.case["runtime_inventory"] = inv
        self.save()

    def save(self):
        self.scenario.write_text(json.dumps(self.case))

    def build(self):
        import build_image

        self.save()
        return build_image.build(self.recipe, self.out, self.root / "cache", timeout=15)

    def test_runtime_overlay_and_official_payloads_are_deleted_on_context_exit(self):
        import build_image

        self.build()
        with build_image.provisioned(
            self.out / "apps-image.qcow2",
            self.out / "apps-image.manifest.json",
            self.root / "runtime",
            self.root / "cache",
            timeout=15,
        ) as (disk, path, manifest):
            self.assertTrue(disk.exists())
            self.assertEqual(manifest["layer"], "runtime")
            self.assertFalse(manifest["no_nvidia_proprietary_files"])
            temporary = disk.parent
        self.assertFalse(temporary.exists())

    def test_runtime_driver_receipt_uses_measured_module_version(self):
        import build_image

        self.build()
        with build_image.provisioned(
            self.out / "apps-image.qcow2",
            self.out / "apps-image.manifest.json",
            self.root / "runtime",
            self.root / "cache",
            timeout=15,
        ) as (disk, path, manifest):
            receipt = Path(str(disk) + ".STAGED")
            self.assertIn("version=580.159.04", receipt.read_text())
            self.assertIn("image=" + str(disk), receipt.read_text())
        self.assertFalse(receipt.exists())

    def test_runtime_context_exception_also_deletes_nvidia_overlay(self):
        import build_image

        self.build()
        temporary = None
        with self.assertRaisesRegex(RuntimeError, "gate failed"):
            with build_image.provisioned(
                self.out / "apps-image.qcow2",
                self.out / "apps-image.manifest.json",
                self.root / "runtime",
                self.root / "cache",
                timeout=15,
            ) as (disk, path, manifest):
                temporary = disk.parent
                raise RuntimeError("gate failed")
        self.assertFalse(temporary.exists())

    def test_provisioning_refuses_dirty_open_image_before_install(self):
        import build_image

        self.build()
        self.inv["nvidia_scan"]["clean"] = False
        self.save()
        with self.assertRaises(ValueError):
            with build_image.provisioned(
                self.out / "apps-image.qcow2",
                self.out / "apps-image.manifest.json",
                self.root / "runtime",
                self.root / "cache",
                timeout=15,
            ):
                self.fail("dirty image yielded")

    def test_runtime_inventory_can_be_verified_separately(self):
        import build_image

        self.build()
        with build_image.provisioned(
            self.out / "apps-image.qcow2",
            self.out / "apps-image.manifest.json",
            self.root / "runtime",
            self.root / "cache",
            timeout=15,
        ) as (disk, path, manifest):
            report = build_image.verify(
                disk, path, self.root / "runtime-verification", timeout=15
            )
            self.assertEqual(report["layer"], "runtime")
            self.assertEqual(report["status"], "inventory-identical")

    def test_runtime_verification_rejects_nvidia_install_drift(self):
        import build_image

        self.build()
        with build_image.provisioned(
            self.out / "apps-image.qcow2",
            self.out / "apps-image.manifest.json",
            self.root / "runtime",
            self.root / "cache",
            timeout=15,
        ) as (disk, path, manifest):
            self.case["runtime_inventory"]["guest"]["nvidia_module_version"] = "wrong"
            self.save()
            with self.assertRaises(ValueError):
                build_image.verify(
                    disk, path, self.root / "runtime-verification", timeout=15
                )


class BoundedTransport(unittest.TestCase):
    def test_serial_sink_rejects_bytes_over_its_limit(self):
        from channels import Channels
        import socket

        with tempfile.TemporaryDirectory() as d:
            with Channels(d, {"serial": 4}) as channels:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as peer:
                    peer.connect(str(channels.path("serial")))
                    peer.sendall(b"12345")
                    channels.pump(0.1)
                    with self.assertRaises(ValueError):
                        channels.pump(0.1)
                self.assertLessEqual((Path(d) / "serial.log").stat().st_size, 4)

    def test_inventory_sink_rejects_bytes_over_its_limit(self):
        from channels import Channels
        import socket

        with tempfile.TemporaryDirectory() as d:
            with Channels(d, {"inventory": 4}) as channels:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as peer:
                    peer.connect(str(channels.path("inventory")))
                    peer.sendall(b"12345")
                    channels.pump(0.1)
                    with self.assertRaises(ValueError):
                        channels.pump(0.1)
                self.assertLessEqual((Path(d) / "inventory.json").stat().st_size, 4)


class FrozenApt(unittest.TestCase):
    def setUp(self):
        import lzma
        from apt_mirror import AptMirror

        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        data = lzma.compress(
            b"Package: hello\nVersion: 1.0\nFilename: pool/universe/h/hello/hello.deb\nSize: 2\nSHA256: "
            + b"a" * 64
            + b"\n\n"
        )
        path = self.root / "packages"
        path.write_bytes(data)
        meta = {
            "id": "meta",
            "url": "https://snapshot.ubuntu.com/packages",
            "sha256": hashlib.sha256(data).hexdigest(),
            "size": len(data),
            "path": "dists/noble/universe/binary-amd64/Packages.xz",
        }
        recipe = {
            "apt": {
                "components": ["main", "universe"],
                "snapshot": "20261101T000000Z",
                "metadata": [meta],
                "download_budget_bytes": 100,
            }
        }
        self.fetches = []

        def fetched(pin, cache):
            self.fetches.append(pin)
            return path

        self.recipe = recipe
        self.factory = lambda r: AptMirror(
            r, self.root, fetcher=fetched, metadata_fetcher=lambda pin, cache: path
        )

    def test_unknown_guest_request_cannot_cause_arbitrary_host_download(self):
        mirror = self.factory(self.recipe)
        self.assertIsNone(mirror.get("../../root/secret"))
        self.assertEqual(self.fetches, [])

    def test_only_package_pin_from_frozen_index_is_fetched(self):
        mirror = self.factory(self.recipe)
        mirror.get("pool/universe/h/hello/hello.deb")
        self.assertEqual(self.fetches[0]["sha256"], "a" * 64)
        self.assertIn("/20261101T000000Z/pool/universe/", self.fetches[0]["url"])

    def test_by_hash_index_request_serves_the_exact_verified_index(self):
        mirror = self.factory(self.recipe)
        pin = self.recipe["apt"]["metadata"][0]
        self.assertIsNotNone(
            mirror.get(
                "dists/noble/universe/binary-amd64/by-hash/SHA256/" + pin["sha256"]
            )
        )
        self.assertEqual(self.fetches, [])

    def test_apt_download_budget_fails_closed(self):
        recipe = copy.deepcopy(self.recipe)
        recipe["apt"]["download_budget_bytes"] = 1
        mirror = self.factory(recipe)
        with self.assertRaises(official.MediaDrift):
            mirror.get("pool/universe/h/hello/hello.deb")

    def test_zero_apt_budget_downloads_nothing(self):
        recipe = copy.deepcopy(self.recipe)
        recipe["apt"]["download_budget_bytes"] = 0
        mirror = self.factory(recipe)
        with self.assertRaises(official.MediaDrift):
            mirror.get("pool/universe/h/hello/hello.deb")
        self.assertEqual(self.fetches, [])

    def wrong_pool(self, component):
        import lzma

        recipe = copy.deepcopy(self.recipe)
        path = self.root / "packages"
        data = lzma.compress(
            lzma.decompress(path.read_bytes()).replace(
                b"pool/universe/", ("pool/" + component + "/").encode()
            )
        )
        path.write_bytes(data)
        recipe["apt"]["metadata"][0]["sha256"] = hashlib.sha256(data).hexdigest()
        recipe["apt"]["metadata"][0]["size"] = len(data)
        return recipe

    def test_main_universe_index_cannot_redirect_to_restricted_pool(self):
        with self.assertRaises(ValueError):
            self.factory(self.wrong_pool("restricted"))

    def test_main_universe_index_cannot_redirect_to_multiverse_pool(self):
        with self.assertRaises(ValueError):
            self.factory(self.wrong_pool("multiverse"))

    def test_multiverse_mirror_is_refused(self):
        recipe = copy.deepcopy(self.recipe)
        recipe["apt"]["components"] = ["main", "multiverse"]
        with self.assertRaises(ValueError):
            self.factory(recipe)

    def test_restricted_mirror_is_refused(self):
        recipe = copy.deepcopy(self.recipe)
        recipe["apt"]["components"] = ["main", "restricted"]
        with self.assertRaises(ValueError):
            self.factory(recipe)


class ElfLinking(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def binary(self, names):
        import struct

        data = bytearray(1024)
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<Q", data, 32, 64)
        struct.pack_into("<HH", data, 54, 56, 2)
        struct.pack_into(
            "<IIQQQQQQ", data, 64, 1, 4, 0, 0x400000, 0, len(data), len(data), 4096
        )
        entries = [(5, 0x400000 + 768)]
        strings = b"\0"
        for name in names:
            entries.append((1, len(strings)))
            strings += name.encode() + b"\0"
        entries.extend([(10, len(strings)), (0, 0)])
        struct.pack_into(
            "<IIQQQQQQ",
            data,
            120,
            2,
            4,
            256,
            0x400100,
            0,
            len(entries) * 16,
            len(entries) * 16,
            8,
        )
        for i, (tag, value) in enumerate(entries):
            struct.pack_into("<qQ", data, 256 + i * 16, tag, value)
        data[768 : 768 + len(strings)] = strings
        p = self.root / "app"
        p.write_bytes(data)
        return p

    def test_dynamic_runtime_dependency_is_read_without_readelf(self):
        import elf

        self.assertEqual(
            elf.check_shared(self.binary(["libcudart.so.12"]), ["libcudart.so"]),
            ["libcudart.so.12"],
        )

    def test_missing_shared_cudart_is_rejected(self):
        import elf

        with self.assertRaises(ValueError):
            elf.check_shared(self.binary(["libc.so.6"]), ["libcudart.so"])

    def test_missing_shared_cublas_is_rejected(self):
        import elf

        with self.assertRaises(ValueError):
            elf.check_shared(self.binary(["libcuda.so.1"]), ["libcublas.so"])

    def test_malformed_string_table_cannot_look_like_success(self):
        import elf, struct

        p = self.binary(["libcudart.so.12"])
        data = bytearray(p.read_bytes())
        struct.pack_into("<Q", data, 264, 0xFFFFFFFFFFFFFFFF)
        p.write_bytes(data)
        with self.assertRaises(ValueError):
            elf.needed(p)

    def test_non_elf_output_cannot_pass_link_audit(self):
        import elf

        p = self.root / "app"
        p.write_bytes(b"libcudart.so.12")
        with self.assertRaises(ValueError):
            elf.check_shared(p, ["libcudart.so"])


class AdditionalLayerFailures(unittest.TestCase):
    def test_monthly_checks_continue_after_a_missing_url(self):
        calls = []

        def missing(request, **kwargs):
            calls.append(request.full_url)
            raise urllib.error.HTTPError(request.full_url, 404, "missing", {}, None)

        first = pin()
        second = dict(pin(), id="second", url="https://example.org/second.run")
        with self.assertRaises(official.MediaDrift):
            official.verify_urls([first, second], opener=missing)
        self.assertEqual(len(calls), 2)

    def test_empty_official_list_does_not_certify_monthly_health(self):
        with self.assertRaises(official.MediaDrift):
            official.verify_urls([])

    def test_scan_file_budget_exhaustion_is_failure(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "a").write_bytes(b"a")
            (root / "b").write_bytes(b"b")
            self.assertFalse(scan_image.scan(root, [], max_files=1)["clean"])

    def test_scan_byte_budget_exhaustion_is_failure(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "a").write_bytes(b"oversize")
            self.assertFalse(scan_image.scan(root, [], max_bytes=1)["clean"])


class StrictNoticeScan(unittest.TestCase):
    def test_nvidia_copyright_notice_is_not_silently_waived(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "LICENSE"
            p.write_bytes(b"Copyright (c) 2020 NVIDIA Corporation.\nBSD-3-Clause\n")
            self.assertFalse(scan_image.scan(d, [])["clean"])

    def test_documented_bsd_nvidia_source_notice_is_retained_and_listed(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "LICENSE"
            p.write_bytes(
                b"Copyright (c) NVIDIA Corporation\nSPDX-License-Identifier: BSD-3-Clause\n"
            )
            report = scan_image.scan(d, [])
            self.assertTrue(report["clean"])
            self.assertEqual(
                report["notices"][0]["licence_evidence"], "SPDX: BSD-3-Clause"
            )

    def test_documented_mit_nvidia_source_notice_is_not_proprietary(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "header.h"
            p.write_bytes(
                b"// Copyright NVIDIA Corporation\n// SPDX-License-Identifier: MIT\n"
            )
            self.assertTrue(scan_image.scan(d, [])["clean"])

    def test_proprietary_header_overrides_oss_notice_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "LICENSE"
            p.write_bytes(
                b"Copyright NVIDIA Corporation\nSPDX-License-Identifier: BSD-3-Clause\nNVIDIA SOFTWARE LICENSE AGREEMENT\n"
            )
            self.assertFalse(scan_image.scan(d, [])["clean"])

    def test_second_nvidia_notice_is_listed_as_a_finding(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "header.h"
            p.write_bytes(b"// Copyright 2010 NVIDIA Corporation\n")
            report = scan_image.scan(d, [])
            self.assertFalse(report["clean"])
            self.assertEqual(report["findings"][0]["path"], "/header.h")

    def test_nvidia_cuda_deb_is_not_allowed_without_files(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertFalse(
                scan_image.scan(d, ["ii\tcuda-cudart-12-6\tamd64\t12.6"])["clean"]
            )

    def test_nvidia_smi_executable_is_not_an_open_image_file(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "usr/bin/nvidia-smi"
            p.parent.mkdir(parents=True)
            p.write_bytes(b"\x7fELF")
            self.assertFalse(scan_image.scan(d, [])["clean"])


class BuilderBoundary(unittest.TestCase):
    def setUp(self):
        self.recipe = common.read_recipe(IMAGE / "recipe.json")
        self.recipe["official_fetch"] = []
        self.recipe["official_write_files"] = []

    def test_builder_does_not_export_a_cuda_library_directory_link(self):
        import guest

        with patch.object(guest, "run"), patch.object(guest, "link_files") as links:
            guest.install_official(
                self.recipe, {"server_url": "http://10.0.2.100:1"}, builder=True
            )
        self.assertFalse(
            any(x["path"] == "/opt/apps/bundle/lib" for x in links.call_args.args[0])
        )

    def test_builder_does_not_export_proprietary_cuda_headers_via_symlink(self):
        import guest

        with patch.object(guest, "run"), patch.object(guest, "link_files") as links:
            guest.install_official(
                self.recipe, {"server_url": "http://10.0.2.100:1"}, builder=True
            )
        self.assertFalse(
            any(
                x["path"].startswith("/opt/apps/bundle/cuda/")
                for x in links.call_args.args[0]
            )
        )

    def test_runtime_installs_the_declared_cuda_links(self):
        import guest

        with patch.object(guest, "run"), patch.object(guest, "link_files") as links:
            guest.install_official(self.recipe, {"server_url": "http://10.0.2.100:1"})
        self.assertEqual(links.call_args.args[0], self.recipe["official_links"])


class ExportTransfer(unittest.TestCase):
    def setUp(self):
        import test_apps_image as base

        self.recipe = base.fixture()
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def archive(
        self, name="opt/apps/bundle/bin/demo", data=b"built", type=None, link=""
    ):
        import tarfile

        path = self.root / "export.tar.gz"
        with tarfile.open(path, "w:gz", format=tarfile.USTAR_FORMAT) as t:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755
            if type is not None:
                info.type = type
                info.linkname = link
                info.size = 0
            t.addfile(info, io.BytesIO(data) if info.isfile() else None)
        return path

    def inventory(self):
        return {
            "files": {"/opt/apps/bundle/bin/demo": hashlib.sha256(b"built").hexdigest()}
        }

    def test_export_matches_exact_builder_file_hashes(self):
        import export_check

        self.assertEqual(
            export_check.validate_export(self.archive(), self.recipe, self.inventory()),
            self.inventory()["files"],
        )

    def test_corrupt_export_file_fails_before_final_guest(self):
        import export_check

        with self.assertRaises(ValueError):
            export_check.validate_export(
                self.archive(data=b"wrong"), self.recipe, self.inventory()
            )

    def test_export_reported_hash_must_match_transferred_bytes(self):
        import export_check

        inventory = self.inventory()
        inventory["files"]["/opt/apps/bundle/bin/demo"] = "0" * 64
        with self.assertRaises(ValueError):
            export_check.validate_export(self.archive(), self.recipe, inventory)

    def test_fifo_cannot_be_exported(self):
        import export_check, tarfile

        inventory = {
            "files": {"/opt/apps/bundle/bin/demo": hashlib.sha256(b"").hexdigest()}
        }
        with self.assertRaises(ValueError):
            export_check.validate_export(
                self.archive(type=tarfile.FIFOTYPE), self.recipe, inventory
            )

    def test_static_device_fatbin_cannot_enter_open_export(self):
        import export_check

        path = "opt/apps/bundle/samples/cdp.fatbin"
        recipe = copy.deepcopy(self.recipe)
        recipe["builds"][0]["requires"] = "nvidia-layer-static"
        recipe["builds"][0]["outputs"][0]["path"] = "/" + path
        inventory = {"files": {"/" + path: hashlib.sha256(b"built").hexdigest()}}
        with self.assertRaises(ValueError):
            export_check.validate_export(self.archive(name=path), recipe, inventory)

    def test_extra_file_cannot_hide_outside_builder_inventory(self):
        import export_check

        with self.assertRaises(ValueError):
            export_check.validate_export(
                self.archive(name="opt/apps/bundle/bin/extra"),
                self.recipe,
                self.inventory(),
            )

    def test_device_file_cannot_be_exported(self):
        import export_check, tarfile

        with self.assertRaises(ValueError):
            export_check.validate_export(
                self.archive(type=tarfile.CHRTYPE),
                self.recipe,
                {
                    "files": {
                        "/opt/apps/bundle/bin/demo": hashlib.sha256(b"").hexdigest()
                    }
                },
            )

    def test_path_escape_cannot_be_exported(self):
        import export_check

        with self.assertRaises(ValueError):
            export_check.validate_export(
                self.archive(name="../escape"), self.recipe, self.inventory()
            )

    def test_static_runtime_output_cannot_be_exported(self):
        import export_check

        self.recipe["builds"][0]["requires"] = "nvidia-layer-static"
        with self.assertRaises(ValueError):
            export_check.validate_export(self.archive(), self.recipe, self.inventory())

    def test_cuda_library_tree_cannot_be_exported(self):
        import export_check

        path = "opt/apps/bundle/lib/libcudart.so.12"
        recipe = copy.deepcopy(self.recipe)
        recipe["builds"][0]["outputs"][0]["path"] = "/" + path
        inventory = {"files": {"/" + path: hashlib.sha256(b"built").hexdigest()}}
        with self.assertRaises(ValueError):
            export_check.validate_export(self.archive(name=path), recipe, inventory)

    def test_cuda_header_tree_cannot_be_exported(self):
        import export_check

        path = "opt/apps/bundle/cuda/include/cuda.h"
        recipe = copy.deepcopy(self.recipe)
        recipe["builds"][0]["outputs"][0]["path"] = "/" + path
        inventory = {"files": {"/" + path: hashlib.sha256(b"built").hexdigest()}}
        with self.assertRaises(ValueError):
            export_check.validate_export(self.archive(name=path), recipe, inventory)


class OpenClTarget(unittest.TestCase):
    def test_provisioning_disables_pocl_registration(self):
        import guest

        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "nvidia.icd").write_text("libnvidia-opencl.so.1")
            (root / "pocl.icd").write_text("libpocl.so")
            with patch.object(guest, "Path", return_value=root):
                guest.select_nvidia_opencl()
            self.assertFalse((root / "pocl.icd").exists())
            self.assertTrue((root / "pocl.icd.disabled").exists())
            self.assertTrue((root / "nvidia.icd").exists())

    def test_provisioning_cannot_accept_a_cpu_only_opencl_platform(self):
        import guest

        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "pocl.icd").write_text("libpocl.so")
            with patch.object(guest, "Path", return_value=root):
                with self.assertRaises(ValueError):
                    guest.select_nvidia_opencl()

    def test_empty_icd_directory_cannot_be_a_provisioned_gpu_runtime(self):
        import guest

        with tempfile.TemporaryDirectory() as d:
            with patch.object(guest, "Path", return_value=Path(d)):
                with self.assertRaises(ValueError):
                    guest.select_nvidia_opencl()

    def test_all_other_icds_are_disabled_before_the_gpu_gate(self):
        import guest

        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "nvidia.icd").write_text("libnvidia-opencl.so.1")
            (root / "mesa.icd").write_text("other")
            with patch.object(guest, "Path", return_value=root):
                guest.select_nvidia_opencl()
            self.assertEqual([p.name for p in root.glob("*.icd")], ["nvidia.icd"])


class BundledCudaHeaders(unittest.TestCase):
    def test_wrapped_proprietary_confidential_header_fails(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "cuda_fp16.h"
            p.write_bytes(
                b"/* Copyright NVIDIA Corporation.\n * PROPRIETARY and\n * CONFIDENTIAL to NVIDIA.\n */"
            )
            self.assertFalse(scan_image.scan(d, [])["clean"])

    def test_proprietary_cuda_header_cannot_hide_behind_bsd_marker(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "cuda_fp16.hpp"
            p.write_bytes(
                b"// Copyright NVIDIA Corporation.\n// SPDX-License-Identifier: BSD-3-Clause\n// PROPRIETARY AND CONFIDENTIAL TO NVIDIA\n"
            )
            self.assertFalse(scan_image.scan(d, [])["clean"])

    def test_cupy_wheel_is_runtime_only_due_to_bundled_proprietary_headers(self):
        recipe = common.read_recipe(IMAGE / "recipe.json")
        self.assertFalse(
            any(a.get("package") == "cupy-cuda12x" for a in recipe["artifacts"])
        )
        pin = next(
            a for a in recipe["official_fetch"] if a.get("package") == "cupy-cuda12x"
        )
        self.assertEqual(pin["requires"], "nvidia-layer")
        self.assertIn("headers", pin["reason"])


if __name__ == "__main__":
    unittest.main()
