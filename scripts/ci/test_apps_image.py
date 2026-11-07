#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free LB1 contracts. No network and no real QEMU is invoked."""

import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

IMAGE = Path(__file__).resolve().parents[1] / "apps" / "image"
sys.path.insert(0, str(IMAGE))
import common
import iso9660
import build_image
import guest


def digest(data):
    return hashlib.sha256(data).hexdigest()


def fixture():
    return {
        "schema_version": 2,
        "recipe_version": "fixture-2",
        "base": {
            "id": "base",
            "url": "https://example.org/base.img",
            "sha256": digest(b"base"),
            "size": 4,
        },
        "apt": {
            "snapshot": "20261101T000000Z",
            "packages": ["python3", "cmake"],
            "suites": ["noble", "noble-updates", "noble-security"],
            "components": ["main", "universe"],
            "metadata": [],
            "download_budget_bytes": 1024**3,
        },
        "guest": {
            "user": "ubuntu",
            "authorized_keys": ["${SSH_AUTHORIZED_KEY}"],
            "kernel": "6.8.0-147-generic",
            "nvidia_version": "580.159.04",
            "disk_gib": 74,
            "memory_mib": 12288,
            "cpus": 2,
        },
        "artifacts": [
            {
                "id": "model",
                "url": "https://example.org/rev/model",
                "sha256": digest(b"model"),
                "size": 5,
                "kind": "file",
                "target": "/opt/apps/data/model",
            },
            {
                "id": "wheel",
                "url": "https://example.org/demo-1.0-py3-none-any.whl",
                "sha256": digest(b"wheel"),
                "size": 5,
                "kind": "wheel",
                "filename": "demo-1.0-py3-none-any.whl",
                "package": "demo",
                "version": "1.0",
            },
        ],
        "local_files": [],
        "write_files": [],
        "official_fetch": [],
        "image_policy": {"nvidia_notice_policy": "documented-oss-only"},
        "runtime_inventory": {
            "required_paths": ["/opt/apps/data/model", "/opt/apps/bundle/bin/demo"],
            "trees": ["/opt/apps/bundle"],
            "dpkg_binaries": ["ffmpeg"],
        },
        "builds": [
            {
                "name": "demo",
                "requires": "open-layer",
                "source": "local",
                "cwd": "/opt/apps/source",
                "commands": [["cmake", "--version"]],
                "outputs": [
                    {"path": "/opt/apps/bundle/bin/demo", "sha256": "inventory-only"}
                ],
            }
        ],
        "inventory": {
            "required_paths": ["/opt/apps/data/model", "/opt/apps/bundle/bin/demo"],
            "trees": ["/opt/apps/bundle"],
            "dpkg_binaries": ["ffmpeg"],
        },
        "environment": {},
        "links": [],
        "copies": [],
    }


class Pins(unittest.TestCase):
    def test_recipe_accepts_complete_fixture(self):
        common.validate_recipe(fixture())

    def test_missing_base_hash_refused(self):
        r = fixture()
        del r["base"]["sha256"]
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_missing_model_hash_refused(self):
        r = fixture()
        r["artifacts"][0]["sha256"] = ""
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_shell_absolute_path_refused(self):
        r = fixture()
        r["builds"][0]["commands"] = [["/bin/sh", "-c", "make"]]
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_malformed_snapshot_refused(self):
        r = fixture()
        r["apt"]["snapshot"] = "2026-11-01"
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_wrong_schema_refused(self):
        r = fixture()
        r["schema_version"] = 99
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_moving_snapshot_refused(self):
        r = fixture()
        r["apt"]["snapshot"] = "latest"
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_no_http_base(self):
        r = fixture()
        r["base"]["url"] = "http://example.org/base"
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_no_url_credentials(self):
        r = fixture()
        r["artifacts"][0]["url"] = "https://name:password@example.org/model"
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_shell_command_refused(self):
        r = fixture()
        r["builds"][0]["commands"] = [["bash", "-c", "make"]]
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_string_command_refused(self):
        r = fixture()
        r["builds"][0]["commands"] = ["cmake --build build"]
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_shell_operator_refused(self):
        r = fixture()
        r["builds"][0]["commands"] = [["make", "&&", "cp"]]
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_unpinned_output_refused(self):
        r = fixture()
        r["builds"][0]["outputs"][0]["sha256"] = "later"
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_duplicate_input_refused(self):
        r = fixture()
        r["artifacts"].append(copy.deepcopy(r["artifacts"][0]))
        with self.assertRaises(ValueError):
            common.validate_recipe(r)

    def test_local_path_escape_refused(self):
        r = fixture()
        r["local_files"] = [
            {
                "path": "../../../secret",
                "sha256": digest(b"x"),
                "target": "/opt/apps/source/x",
            }
        ]
        with self.assertRaises(ValueError):
            common.validate_recipe(r)


class Downloads(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.cache = Path(self.tmp.name)
        self.pin = fixture()["artifacts"][0]

    def fetch(self, data):
        response = io.BytesIO(data)
        response.headers = {}
        return common.download(self.pin, self.cache, opener=lambda *a, **k: response)

    def test_download_verified_before_publish(self):
        p = self.fetch(b"model")
        self.assertEqual(p.read_bytes(), b"model")
        self.assertEqual(p.name, self.pin["sha256"])

    def test_corrupt_download_refused(self):
        with self.assertRaises(ValueError):
            self.fetch(b"wrong")
        self.assertFalse((self.cache / self.pin["sha256"]).exists())

    def test_truncated_download_refused(self):
        with self.assertRaises(ValueError):
            self.fetch(b"mod")
        self.assertFalse(list(self.cache.glob("*.part")))

    def test_poisoned_cache_refused(self):
        (self.cache / self.pin["sha256"]).write_bytes(b"wrong")
        with self.assertRaises(ValueError):
            self.fetch(b"model")

    def test_oversize_cache_refused(self):
        (self.cache / self.pin["sha256"]).write_bytes(b"model extra")
        with self.assertRaises(ValueError):
            self.fetch(b"model")

    def test_valid_cache_uses_no_network(self):
        p = self.cache / self.pin["sha256"]
        p.write_bytes(b"model")
        with patch.object(
            common.urllib.request, "urlopen", side_effect=AssertionError("network")
        ):
            self.assertEqual(common.download(self.pin, self.cache), p)

    def test_declared_length_must_match_even_with_correct_hash(self):
        self.pin["size"] = 9
        with self.assertRaises(ValueError):
            self.fetch(b"model")

    def test_cached_length_must_match_even_with_correct_hash(self):
        self.pin["size"] = 9
        (self.cache / self.pin["sha256"]).write_bytes(b"model")
        with self.assertRaises(ValueError):
            self.fetch(b"model")

    def test_oversize_download_refused(self):
        with self.assertRaises(ValueError):
            self.fetch(b"model" * 100)


class Archives(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def tar(self, name, link=None):
        p = self.root / "source.tar"
        with tarfile.open(p, "w") as t:
            i = tarfile.TarInfo(name)
            if link is None:
                i.size = 2
                t.addfile(i, io.BytesIO(b"ok"))
            else:
                i.type = tarfile.SYMTYPE
                i.linkname = link
                t.addfile(i)
        return p

    def test_parent_component_rejected_before_extraction(self):
        with self.assertRaises(ValueError):
            common.relative_path("../file")

    def test_absolute_member_rejected_before_extraction(self):
        with self.assertRaises(ValueError):
            common.relative_path("/file")

    def test_tar_path_escape_refused(self):
        with self.assertRaises(ValueError):
            common.extract(self.tar("../outside"), self.root / "out", 0)
        self.assertFalse((self.root / "outside").exists())

    def test_absolute_tar_path_refused(self):
        with self.assertRaises(ValueError):
            common.extract(self.tar("/tmp/escape"), self.root / "out", 0)

    def test_tar_symlink_escape_refused(self):
        with self.assertRaises(ValueError):
            common.extract(self.tar("a", "../../outside"), self.root / "out", 0)

    def test_zip_path_escape_refused(self):
        p = self.root / "source.zip"
        with zipfile.ZipFile(p, "w") as z:
            z.writestr("../outside", b"bad")
        with self.assertRaises(ValueError):
            common.extract(p, self.root / "out", 0)

    def test_tar_normal_file(self):
        out = self.root / "out"
        common.extract(self.tar("project/file"), out, 1)
        self.assertEqual((out / "file").read_bytes(), b"ok")

    def test_strip_cannot_hide_traversal(self):
        with self.assertRaises(ValueError):
            common.extract(self.tar("../file"), self.root / "out", 1)


class Seed(unittest.TestCase):
    def files(self):
        return {
            "user-data": b"#cloud-config\n{}\n",
            "meta-data": b"instance-id: test\n",
            "network-config": b"version: 2\n",
        }

    def test_seed_deterministic(self):
        self.assertEqual(
            iso9660.make_iso(self.files()),
            iso9660.make_iso(dict(reversed(list(self.files().items())))),
        )

    def test_seed_primary_volume_descriptor(self):
        b = iso9660.make_iso(self.files())
        pvd = b[16 * 2048 : 17 * 2048]
        self.assertEqual(pvd[:7], b"\x01CD001\x01")
        self.assertEqual(pvd[40:72].rstrip(), b"cidata")
        self.assertEqual(len(b) % 2048, 0)
        self.assertEqual(struct.unpack_from("<I", pvd, 80)[0] * 2048, len(b))

    def test_seed_joliet_names_and_data(self):
        b = iso9660.make_iso(self.files())
        svd = b[17 * 2048 : 18 * 2048]
        self.assertEqual(svd[88:91], b"%/E")
        extent = struct.unpack_from("<I", svd, 158)[0]
        directory = b[extent * 2048 : (extent + 1) * 2048]
        names = {}
        offset = 0
        while offset < len(directory) and directory[offset]:
            record = directory[offset : offset + directory[offset]]
            n = record[33 : 33 + record[32]]
            if n not in (b"\x00", b"\x01"):
                name = n.decode("utf-16-be")
                start = struct.unpack_from("<I", record, 2)[0] * 2048
                size = struct.unpack_from("<I", record, 10)[0]
                names[name] = b[start : start + size]
            offset += directory[offset]
        self.assertEqual(names, self.files())

    def test_seed_rejects_incomplete_nocloud_data(self):
        with self.assertRaises(ValueError):
            iso9660.make_iso({"meta-data": b"instance-id: test"})

    def test_both_iso_namespaces_advertise_cidata(self):
        b = iso9660.make_iso(self.files())
        self.assertEqual(b[16 * 2048 + 40 : 16 * 2048 + 72].rstrip(), b"cidata")
        self.assertEqual(
            b[17 * 2048 + 40 : 17 * 2048 + 72].decode("utf-16-be").rstrip(), "cidata"
        )

    def test_seed_rejects_extra_files(self):
        with self.assertRaises(ValueError):
            iso9660.make_iso({"../../escape": b"bad"})


class CloudConfig(unittest.TestCase):
    def config(self, mode="assemble"):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for n in build_image.RUNTIME:
                (root / n).write_text("# fixture\n")
            return build_image.cloud_config(
                fixture(), root, "a" * 32, mode, "http://10.0.2.100:4321", []
            )

    def test_no_shell_runcmd_or_bootcmd(self):
        d = self.config()
        self.assertNotIn("runcmd", d)
        self.assertNotIn("bootcmd", d)
        scripts = [x for x in d["write_files"] if "scripts/per-instance/" in x["path"]]
        self.assertEqual(len(scripts), 1)
        self.assertTrue(scripts[0]["content"].startswith("#!/usr/bin/python3"))

    def test_snapshot_is_only_apt_origin(self):
        d = self.config()
        apt = json.dumps(d["apt"])
        self.assertIn("http://10.0.2.100:4321/ubuntu/", apt)
        self.assertIn("Components: main universe", apt)
        self.assertNotIn("archive.ubuntu.com", apt)
        self.assertNotIn("security.ubuntu.com", apt)
        self.assertFalse(d["apt"]["preserve_sources_list"])

    def test_verify_does_not_install_packages(self):
        d = self.config("verify-open")
        self.assertEqual(d["packages"], [])
        self.assertFalse(d["package_update"])
        self.assertFalse(d["package_upgrade"])

    def test_key_placeholder_not_written(self):
        d = self.config()
        self.assertEqual(d["users"][0]["ssh_authorized_keys"], [])
        self.assertNotIn("${SSH_AUTHORIZED_KEY}", json.dumps(d["users"]))


class Inventory(unittest.TestCase):
    def setUp(self):
        self.recipe = fixture()
        self.inv = {
            "schema_version": 1,
            "layer": "open",
            "nvidia_scan": {"clean": True, "findings": []},
            "no_nvidia_proprietary_files": True,
            "guest": {"kernel": "6.8.0-147-generic", "nvidia_module_version": None},
            "dpkg": ["ii\tcmake\tamd64\t1.0", "ii\tpython3\tamd64\t3.12"],
            "pip_freeze": ["demo==1.0"],
            "pip_files": {"demo": "e" * 64},
            "wheels": {"demo==1.0": digest(b"wheel")},
            "files": {
                "/opt/apps/data/model": digest(b"model"),
                "/opt/apps/bundle/bin/demo": digest(b"built"),
                "/usr/bin/ffmpeg": "f" * 64,
            },
        }

    def test_missing_apt_package_refused(self):
        self.inv["dpkg"] = ["ii\tcmake\tamd64\t1.0"]
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_missing_other_apt_package_refused(self):
        self.inv["dpkg"] = ["ii\tpython3\tamd64\t3.12"]
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_pip_distribution_digest_missing_refused(self):
        self.inv["pip_files"] = {"other": "a" * 64}
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_pip_distribution_digest_extra_refused(self):
        self.inv["pip_files"]["extra"] = "a" * 64
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_wrong_guest_kernel_refused(self):
        self.inv["guest"]["kernel"] = "wrong"
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_wrong_guest_module_version_refused(self):
        self.inv["guest"]["nvidia_module_version"] = "wrong"
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_complete_inventory(self):
        common.validate_inventory(self.inv, self.recipe)

    def test_empty_inventory_refused(self):
        with self.assertRaises(ValueError):
            common.validate_inventory({}, self.recipe)

    def test_missing_model_refused(self):
        del self.inv["files"]["/opt/apps/data/model"]
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_corrupt_model_refused(self):
        self.inv["files"]["/opt/apps/data/model"] = "0" * 64
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_model_config_digest_must_match_its_own_pin(self):
        config = {
            "id": "model-config",
            "kind": "file",
            "url": "https://example.org/config.json",
            "sha256": digest(b"{}"),
            "size": 2,
            "target": "/opt/apps/data/config.json",
        }
        self.recipe["artifacts"].append(config)
        self.recipe["inventory"]["required_paths"].append(config["target"])
        self.inv["files"][config["target"]] = digest(b"{ }")
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_missing_binary_refused(self):
        del self.inv["files"]["/opt/apps/bundle/bin/demo"]
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_pinned_binary_cannot_change(self):
        self.recipe["builds"][0]["outputs"][0]["sha256"] = "d" * 64
        self.inv["files"]["/opt/apps/bundle/bin/demo"] = "e" * 64
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_pip_freeze_normalizes_pep503_names(self):
        self.recipe["artifacts"][1]["package"] = "demo-package"
        self.inv["wheels"] = {"demo-package==1.0": digest(b"wheel")}
        self.inv["pip_files"] = {"demo-package": "e" * 64}
        self.inv["pip_freeze"] = ["Demo_Package==1.0"]
        common.validate_inventory(self.inv, self.recipe)

    def test_expected_binary_hash_enforced(self):
        self.recipe["builds"][0]["outputs"][0]["sha256"] = "a" * 64
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_wheel_hash_refused(self):
        self.inv["wheels"]["demo==1.0"] = "0" * 64
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_pip_drift_refused(self):
        self.inv["pip_freeze"] = ["demo==2.0"]
        with self.assertRaises(ValueError):
            common.validate_inventory(self.inv, self.recipe)

    def test_verify_compares_dpkg_versions(self):
        observed = copy.deepcopy(self.inv)
        observed["dpkg"] = ["ii\tcmake\tamd64\t2.0"]
        with self.assertRaises(ValueError):
            build_image.compare_inventory(self.inv, observed)

    def test_verify_compares_binary_bytes(self):
        observed = copy.deepcopy(self.inv)
        observed["files"]["/usr/bin/ffmpeg"] = "0" * 64
        with self.assertRaises(ValueError):
            build_image.compare_inventory(self.inv, observed)

    def test_verify_compares_installed_pip_bytes(self):
        observed = copy.deepcopy(self.inv)
        observed["pip_files"]["demo"] = "a" * 64
        with self.assertRaises(ValueError):
            build_image.compare_inventory(self.inv, observed)

    def test_equal_inventories_pass(self):
        build_image.compare_inventory(self.inv, copy.deepcopy(self.inv))


class FakeQemu(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fakebin = self.root / "bin"
        self.fakebin.mkdir()
        for name in ("qemu-img", "qemu-system-x86_64"):
            p = self.fakebin / name
            p.write_bytes((IMAGE / "test_fake_qemu.py").read_bytes())
            p.chmod(0o755)
        self.scenario = self.root / "scenario.json"
        self.commands = self.root / "commands.jsonl"
        case = Inventory()
        case.setUp()
        self.inv = case.inv
        self.recipe = case.recipe
        self.case = {"inventory": self.inv}
        self.save()
        self.environment = patch.dict(
            os.environ,
            {
                "PATH": str(self.fakebin) + os.pathsep + os.environ["PATH"],
                "APPS_IMAGE_TEST_SCENARIO": str(self.scenario),
                "APPS_IMAGE_TEST_COMMANDS": str(self.commands),
            },
        )
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.preflight = patch.object(build_image, "preflight")
        self.preflight.start()
        self.addCleanup(self.preflight.stop)
        self.base = self.root / "base"
        self.base.write_bytes(b"base")
        paths = {self.recipe["base"]["sha256"]: self.base}
        for name in ("model", "wheel"):
            path = self.root / name
            path.write_bytes(name.encode())
            paths[digest(name.encode())] = path
        self.fetch = patch.object(build_image, "fetch_inputs", return_value=paths)
        self.fetch.start()
        self.addCleanup(self.fetch.stop)

        def fetch_official(recipe, directory, **kwargs):
            Path(directory).mkdir(parents=True, exist_ok=True)
            return {}

        self.official_patch = patch.object(
            build_image, "fetch_official", side_effect=fetch_official
        )
        self.official_patch.start()
        self.addCleanup(self.official_patch.stop)
        mirror = type("Mirror", (), {"names": {"cmake", "python3"}, "used": {}})()
        self.mirror_patch = patch.object(build_image, "AptMirror", return_value=mirror)
        self.mirror_patch.start()
        self.addCleanup(self.mirror_patch.stop)
        self.out = self.root / "out"

    def save(self):
        self.scenario.write_text(json.dumps(self.case))

    def build(self):
        self.save()
        return build_image.build(self.recipe, self.out, self.root / "cache", timeout=15)

    def records(self):
        return [json.loads(x) for x in self.commands.read_text().splitlines()]

    def test_build_writes_manifest_only_after_verified_completion(self):
        manifest = self.build()
        self.assertEqual(manifest["inventory"], self.inv)
        self.assertEqual(
            manifest["image"]["sha256"],
            digest((self.out / "apps-image.qcow2").read_bytes()),
        )
        self.assertEqual(
            json.loads((self.out / "apps-image.manifest.json").read_text()), manifest
        )
        commands = self.records()
        operations = [r["argv"][0] for r in commands if r["program"] == "qemu-img"]
        self.assertEqual(operations, ["create", "create", "convert", "check"])
        self.assertIn(
            "-c", next(r["argv"] for r in commands if r["argv"][0] == "convert")
        )
        self.assertFalse(list(self.out.glob(".apps-build-*")))

    def test_build_publishes_only_the_audited_open_bundle(self):
        manifest = self.build()
        bundle = self.out / "apps-open-bundle.tar.gz"
        self.assertEqual(common.sha256_file(bundle), manifest["open_bundle"]["sha256"])
        with tarfile.open(bundle) as archive:
            self.assertEqual(archive.getnames(), ["opt/apps/bundle/bin/demo"])

    def test_failed_scan_does_not_publish_an_open_bundle(self):
        self.inv["nvidia_scan"]["clean"] = False
        with self.assertRaises(ValueError):
            self.build()
        self.assertFalse((self.out / "apps-open-bundle.tar.gz").exists())

    def test_assembled_app_must_match_the_audited_builder_bytes(self):
        self.inv["files"]["/opt/apps/bundle/bin/demo"] = "0" * 64
        with self.assertRaises(ValueError):
            self.build()

    def test_assembled_image_cannot_drop_upstream_licence_files(self):
        self.case["extra_builder_file"] = "/opt/apps/bundle/share/licenses/LICENSE"
        with self.assertRaises(ValueError):
            self.build()

    def test_zero_qemu_exit_without_marker_refused(self):
        self.case["no_marker"] = True
        with self.assertRaises(ValueError):
            self.build()
        self.assertFalse((self.out / "apps-image.manifest.json").exists())

    def test_duplicate_completion_marker_refused(self):
        self.case["duplicate"] = True
        with self.assertRaises(ValueError):
            self.build()

    def test_marker_with_wrong_inventory_digest_refused(self):
        self.case["bad_digest"] = True
        with self.assertRaises(ValueError):
            self.build()

    def test_zero_qemu_exit_without_inventory_refused(self):
        self.case["no_inventory"] = True
        with self.assertRaises(ValueError):
            self.build()

    def test_stale_completion_refused(self):
        self.case["stale"] = True
        with self.assertRaises(ValueError):
            self.build()

    def test_error_report_refused(self):
        self.case["error"] = True
        with self.assertRaises(ValueError):
            self.build()

    def test_nonzero_qemu_exit_refused_despite_complete_report(self):
        self.case["qemu_rc"] = 3
        with self.assertRaises(ValueError):
            self.build()
        self.assertFalse((self.out / "apps-image.qcow2").exists())

    def test_corrupt_guest_model_not_certified(self):
        self.inv["files"]["/opt/apps/data/model"] = "0" * 64
        with self.assertRaises(ValueError):
            self.build()

    def test_missing_guest_binary_not_certified(self):
        del self.inv["files"]["/opt/apps/bundle/bin/demo"]
        with self.assertRaises(ValueError):
            self.build()

    def test_qemu_img_failure_does_not_publish_manifest(self):
        self.case["qemu_img_rc"] = 2
        with self.assertRaises(build_image.subprocess.CalledProcessError):
            self.build()
        self.assertFalse((self.out / "apps-image.manifest.json").exists())

    def test_qemu_img_check_failure_does_not_publish_manifest(self):
        self.case["qemu_img_rc"] = 2
        self.case["qemu_img_fail_on"] = "check"
        with self.assertRaises(build_image.subprocess.CalledProcessError):
            self.build()
        self.assertFalse((self.out / "apps-image.manifest.json").exists())

    def test_hung_fake_qemu_times_out_and_is_reaped(self):
        self.case["hang"] = True
        self.save()
        # Direct boot keeps this test under a second while exercising real process ownership.
        work = self.root / "hang"
        work.mkdir()
        seed = build_image.seed(self.recipe, work, "a" * 32, "verify-open")
        with self.assertRaises(TimeoutError):
            build_image.boot(
                self.recipe, self.base, seed, work, "a" * 32, "verify-open", timeout=0.1
            )
        self.assertFalse((self.out / "apps-image.manifest.json").exists())

    def test_verify_boots_overlay_and_preserves_image(self):
        self.build()
        image = self.out / "apps-image.qcow2"
        before = image.read_bytes()
        report = build_image.verify(
            image, self.out / "apps-image.manifest.json", self.root / "verified"
        )
        self.assertEqual(report["status"], "inventory-identical")
        self.assertEqual(image.read_bytes(), before)
        records = self.records()
        vm = [r for r in records if r["program"] == "qemu-system-x86_64"][-1]["argv"]
        self.assertIn("user,id=n0,restrict=on", vm)
        disk = next(
            vm[i + 1]
            for i, x in enumerate(vm)
            if x == "-drive" and "if=virtio" in vm[i + 1]
        )
        self.assertNotIn(str(image) + ",", disk)
        self.assertFalse(list((self.root / "verified").glob(".apps-verify-*")))

    def test_verify_rejects_package_drift(self):
        self.build()
        self.inv["dpkg"] = ["ii\tcmake\tamd64\t2.0"]
        self.save()
        with self.assertRaises(ValueError):
            build_image.verify(
                self.out / "apps-image.qcow2",
                self.out / "apps-image.manifest.json",
                self.root / "v",
            )
        self.assertFalse((self.root / "v" / "apps-image.verify.json").exists())

    def test_build_refuses_pip_version_drift(self):
        self.inv["pip_freeze"] = ["demo==2.0"]
        with self.assertRaises(ValueError):
            self.build()

    def test_verify_rejects_modified_pip_files(self):
        self.build()
        self.inv["pip_files"]["demo"] = "a" * 64
        self.save()
        with self.assertRaises(ValueError):
            build_image.verify(
                self.out / "apps-image.qcow2",
                self.out / "apps-image.manifest.json",
                self.root / "v",
            )

    def test_verify_rejects_nonzero_qemu_exit(self):
        self.build()
        self.case["qemu_rc"] = 2
        self.save()
        with self.assertRaises(ValueError):
            build_image.verify(
                self.out / "apps-image.qcow2",
                self.out / "apps-image.manifest.json",
                self.root / "v",
            )

    def test_verify_rejects_stale_guest_session(self):
        self.build()
        self.case["stale"] = True
        self.save()
        with self.assertRaises(ValueError):
            build_image.verify(
                self.out / "apps-image.qcow2",
                self.out / "apps-image.manifest.json",
                self.root / "v",
            )

    def test_verify_rejects_modified_app_binary(self):
        self.build()
        self.inv["files"]["/usr/bin/ffmpeg"] = "0" * 64
        self.save()
        with self.assertRaises(ValueError):
            build_image.verify(
                self.out / "apps-image.qcow2",
                self.out / "apps-image.manifest.json",
                self.root / "v",
            )

    def test_verify_allows_different_image_bytes_with_equal_inventory(self):
        self.build()
        image = self.out / "apps-image.qcow2"
        image.write_bytes(b"different ext4 timestamps")
        report = build_image.verify(
            image, self.out / "apps-image.manifest.json", self.root / "v"
        )
        self.assertFalse(report["image_sha256_matches_manifest"])
        self.assertEqual(report["status"], "inventory-identical")

    def test_build_refuses_overwrite(self):
        self.build()
        with self.assertRaises(ValueError):
            self.build()

    def test_paths_with_spaces_and_shell_characters_are_literal_arguments(self):
        self.out = self.root / "out ; touch BAD"
        self.build()
        self.assertFalse((self.root / "BAD").exists())
        self.assertTrue((self.out / "apps-image.qcow2").exists())


class GuestInstall(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.recipe = fixture()
        self.calls = []
        self.session = {"server_url": "http://10.0.2.100:1234"}
        for layer, file in [
            ("open", "requirements.lock"),
            ("runtime", "requirements-runtime.lock"),
        ]:
            (self.root / file).write_text(common.requirements(self.recipe, layer))
        self.root_patch = patch.object(guest, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

        def get(pin, session):
            p = self.root / pin["sha256"]
            p.write_bytes(pin["id"].encode())
            return p

        self.get_patch = patch.object(guest, "get_input", side_effect=get)
        self.get_mock = self.get_patch.start()
        self.addCleanup(self.get_patch.stop)
        self.command = patch.object(
            guest, "run", side_effect=lambda argv, **kw: self.calls.append(argv)
        )
        self.command.start()
        self.addCleanup(self.command.stop)

    def install(self):
        guest.install_wheels(self.recipe, self.session, "open")

    def test_guest_pip_installs_only_hashed_offline_wheels(self):
        self.install()
        argv = next(x for x in self.calls if "pip" in x)
        for flag in [
            "--require-hashes",
            "--no-index",
            "--only-binary=:all:",
            "--no-cache-dir",
        ]:
            self.assertIn(flag, argv)

    def test_guest_wheel_resolution_enforces_hashes_without_public_index(self):
        self.install()
        argv = next(x for x in self.calls if "pip" in x)
        self.assertIn("--require-hashes", argv)
        self.assertIn("--no-index", argv)
        self.assertNotIn("--extra-index-url", argv)

    def test_guest_actual_nonzero_command_refused(self):
        self.command.stop()
        with self.assertRaises(guest.subprocess.CalledProcessError):
            guest.run([sys.executable, "-c", "import sys;sys.exit(7)"])

    def test_guest_subprocess_is_argument_list_without_shell(self):
        self.command.stop()
        with patch.object(guest.subprocess, "run") as run:
            guest.run(["cmake", "--build", "build"], capture=True)
        args, kwargs = run.call_args
        self.assertEqual(args[0], ["cmake", "--build", "build"])
        self.assertTrue(kwargs["check"])
        self.assertNotIn("shell", kwargs)

    def test_guest_pip_failure_is_not_completion(self):
        self.command.stop()
        with patch.object(
            guest, "run", side_effect=guest.subprocess.CalledProcessError(1, ["pip"])
        ):
            with self.assertRaises(guest.subprocess.CalledProcessError):
                self.install()

    def test_guest_lock_cannot_drift(self):
        (self.root / "requirements.lock").write_text("demo==9.0")
        with self.assertRaises(ValueError):
            self.install()

    def test_open_wheels_do_not_fetch_official_inputs(self):
        self.recipe["official_fetch"] = [
            {
                "kind": "wheel",
                "id": "nvidia",
                "package": "nvidia-demo",
                "version": "1.0",
                "sha256": "0" * 64,
            }
        ]
        self.install()
        self.assertEqual(
            [c.args[0]["id"] for c in self.get_mock.call_args_list], ["wheel"]
        )


class PipBytes(unittest.TestCase):
    def setUp(self):
        from types import SimpleNamespace

        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "data.py").write_bytes(b"original")
        dist = SimpleNamespace(
            metadata={"Name": "demo"},
            files=[Path("data.py")],
            locate_file=lambda x: self.root / x,
        )
        self.metadata = patch.object(
            guest.importlib.metadata, "distributions", return_value=[dist]
        )
        self.metadata.start()
        self.addCleanup(self.metadata.stop)

    def test_installed_pip_bytes_are_actually_hashed(self):
        before = guest.pip_files()
        (self.root / "data.py").write_bytes(b"changed")
        self.assertNotEqual(guest.pip_files(), before)

    def test_installed_pip_file_removal_is_not_invisible(self):
        (self.root / "data.py").unlink()
        with self.assertRaises(ValueError):
            guest.pip_files()


class HostAllowlist(unittest.TestCase):
    def test_local_server_only_serves_pinned_paths(self):
        import urllib.request

        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "pin"
            p.write_bytes(b"pinned")
            with build_image.serve_inputs({"a" * 64: p}) as url:
                local = url.replace("10.0.2.100", "127.0.0.1")
                self.assertEqual(
                    urllib.request.urlopen(local + "/" + "a" * 64).read(), b"pinned"
                )
                with self.assertRaises(urllib.error.HTTPError) as ctx:
                    urllib.request.urlopen(local + "/../pin")
                ctx.exception.close()

    def test_local_server_does_not_accept_uploads(self):
        import urllib.request

        with build_image.serve_inputs({}) as url:
            request = urllib.request.Request(
                url.replace("10.0.2.100", "127.0.0.1") + "/new",
                data=b"data",
                method="POST",
            )
            with self.assertRaises(urllib.error.HTTPError) as ctx:
                urllib.request.urlopen(request)
            ctx.exception.close()


class ForwardedInputs(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def argv(self, mode, url):
        path = build_image.seed(fixture(), self.root, "b" * 32, mode, url)
        with build_image.Channels(self.root, {"serial": 100, "inventory": 100}) as ch:
            return build_image.qemu_argv(
                fixture(), self.root / "disk", path, ch, "fake-qemu"
            )

    def test_builder_has_explicit_forward_to_pinned_input_server(self):
        argv = self.argv("builder", "http://10.0.2.100:4321")
        net = argv[argv.index("-netdev") + 1]
        self.assertIn("restrict=on", net)
        self.assertIn("guestfwd=tcp:10.0.2.100:4321-chardev:pin-http", net)
        self.assertIn(
            "socket,id=pin-http,host=127.0.0.1,port=4321,reconnect-ms=500", argv
        )

    def test_provisioning_can_only_forward_to_local_official_cache(self):
        argv = self.argv("provision", "http://10.0.2.100:12345")
        net = argv[argv.index("-netdev") + 1]
        self.assertIn("guestfwd=tcp:10.0.2.100:12345-chardev:pin-http", net)
        self.assertNotIn("cmd:", net)
        self.assertIn(
            "socket,id=pin-http,host=127.0.0.1,port=12345,reconnect-ms=500", argv
        )

    def test_input_forward_refuses_an_external_host(self):
        with self.assertRaises(ValueError):
            self.argv("assemble", "http://example.org:4321")

    def test_input_forward_refuses_credentials(self):
        with self.assertRaises(ValueError):
            self.argv("assemble", "http://user:password@10.0.2.100:4321")

    def test_verification_has_no_input_forward(self):
        argv = self.argv("verify-open", "")
        self.assertEqual(argv[argv.index("-netdev") + 1], "user,id=n0,restrict=on")
        self.assertFalse(any("pin-http" in x for x in argv))

    def test_input_server_keeps_forwarded_stream_for_multiple_requests(self):
        import http.client
        import urllib.parse

        p = self.root / "pin"
        p.write_bytes(b"pinned")
        with build_image.serve_inputs({"a" * 64: p}) as url:
            port = urllib.parse.urlsplit(url).port
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
            self.addCleanup(connection.close)
            connection.request("GET", "/" + "a" * 64, headers={"Connection": "close"})
            response = connection.getresponse()
            self.assertEqual(response.read(), b"pinned")
            self.assertFalse(response.will_close)
            first_socket = connection.sock
            connection.request("GET", "/" + "a" * 64, headers={"Connection": "close"})
            self.assertEqual(connection.getresponse().read(), b"pinned")
            self.assertIs(connection.sock, first_socket)
            connection.close()


class CommittedRecipe(unittest.TestCase):
    def setUp(self):
        self.recipe = common.read_recipe(IMAGE / "recipe.json")

    def test_requirements_lock_matches_every_wheel_pin(self):
        self.assertEqual(
            (IMAGE / "requirements.lock").read_text(), common.requirements(self.recipe)
        )
        self.assertTrue(
            all(not p.startswith("nvidia-") for p in common.locked_wheels(self.recipe))
        )
        self.assertEqual(
            (IMAGE / "requirements-runtime.lock").read_text(),
            common.requirements(self.recipe, "runtime"),
        )

    def test_every_local_input_matches_its_content_pin(self):
        files = list(build_image.local_files(self.recipe, IMAGE))
        self.assertGreater(len(files), 15)
        for row, path in files:
            self.assertEqual(row["sha256"], common.sha256_file(path))

    def test_every_legacy_cuda_source_is_built(self):
        sources = {p.stem for p in (IMAGE.parent / "src").glob("*.cu")}
        builds = {b["name"] for b in self.recipe["builds"] if b["source"] == "local"}
        self.assertTrue(sources <= builds)

    def test_every_legacy_sample_and_driver_fatbin_is_in_inventory(self):
        text = (IMAGE.parent / "build_bundle.sh").read_text()
        samples = (
            text.split('SAMPLES="', 1)[1].split('"', 1)[0].replace("\\\n", " ").split()
        )
        required = set(self.recipe["runtime_inventory"]["required_paths"])
        for path in samples:
            if path == "\\":
                continue
            self.assertIn(
                "/opt/apps/bundle/samples/" + path.rsplit("/", 1)[-1], required
            )
        for name in ("vectorAdd_kernel64.fatbin", "matrixMul_kernel64.fatbin"):
            self.assertIn("/opt/apps/bundle/samples/" + name, required)

    def test_only_the_two_gate_models_are_included(self):
        weights = [
            a
            for a in self.recipe["artifacts"]
            if a["kind"] == "file" and a["target"].endswith((".safetensors", ".gguf"))
        ]
        self.assertEqual(
            {a["id"] for a in weights},
            {"qwen2-half-billion-weights", "qwen25-1.5b-q4km"},
        )
        for a in weights:
            self.assertNotIn("/resolve/main/", a["url"])
            self.assertNotIn("/resolve/master/", a["url"])

    def test_gate_cuda_and_native_binaries_have_declared_outputs(self):
        required = set(self.recipe["runtime_inventory"]["required_paths"])
        gate = [
            s.strip()
            for s in (IMAGE.parent / "gate_list.txt").read_text().splitlines()
            if s and not s.startswith("#")
        ]
        rows = (
            (IMAGE.parent / "run_apps.sh")
            .read_text()
            .split("<<'EOF'", 1)[1]
            .split("\nEOF", 1)[0]
            .splitlines()
        )
        commands = {
            row.split("|", 3)[0]: row.split("|", 3)[3]
            for row in rows
            if row.count("|") >= 3
        }
        import re

        for name in gate:
            cmd = commands[name]
            for relative in re.findall(r"\$S/([A-Za-z0-9_]+)", cmd):
                self.assertIn("/opt/apps/bundle/samples/" + relative, required)
            for relative in re.findall(r"\$B/bin/([A-Za-z0-9_]+)", cmd):
                self.assertIn("/opt/apps/bundle/bin/" + relative, required)

    def test_hashcat_and_nvidia_versions_have_official_deb_hashes(self):
        debs = {a["package"]: a for a in self.recipe["artifacts"] if a["kind"] == "deb"}
        self.assertEqual(debs["hashcat"]["version"], "6.2.6+ds1-1build2")
        self.assertFalse(any(p.startswith("libnvidia") for p in debs))
        driver = next(a for a in self.recipe["official_fetch"] if a["kind"] == "run")
        self.assertEqual(driver["version"], "580.159.04")
        for a in debs.values():
            common.validate_pin(a)

    def test_all_authored_builds_use_argv(self):
        for b in self.recipe["builds"]:
            for argv in b["commands"]:
                common.check_argv(argv)
                self.assertFalse(argv[0].endswith(".sh"))

    def test_portable_llama_build_avoids_host_cpu_tuning(self):
        argv = next(b for b in self.recipe["builds"] if b["name"] == "llama")[
            "commands"
        ][0]
        self.assertIn("-DGGML_NATIVE=OFF", argv)
        self.assertIn("-DCMAKE_CUDA_ARCHITECTURES=86", argv)

    def test_seed_preserves_legacy_tap_address_and_slirp_dhcp(self):
        network = self.recipe["guest"]["network"]["ethernets"]["apps"]
        self.assertEqual(network["addresses"], ["192.168.77.2/24"])
        self.assertTrue(network["dhcp4"])
        self.assertEqual(network["routes"][0]["metric"], 500)


if __name__ == "__main__":
    unittest.main()
