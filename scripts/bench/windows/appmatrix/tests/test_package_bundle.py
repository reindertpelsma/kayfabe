# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline WB1 contracts. No vendor downloads, compilers, guest or upload."""
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch
import zipfile

import helpers as H
import package_bundle as B
import bundle_iso as I
import bundle_tools as T
import bundle_repro as R
import warnings


def pin(data=b"installer stays intact", name="7zr.exe"):
    return dict(name=name, url="https://vendor.invalid/" + name,
                sha256=hashlib.sha256(data).hexdigest(), size=len(data))


def manifest():
    return dict(schema=1, packages=[dict(id="sevenzip", title="7-Zip", version="1",
        license="public domain", redistribution="Redistributable",
        stage=dict(mode="copy", dest="tools"), files=[pin()],
        compliance=dict(license_id="MIT", upstream="https://vendor.invalid/project", distribution="public-ok",
            conditions=[], sources=[], license_inputs=[pin(b"fixture licence", "LICENSE.txt")],
            license_rules={"7zr.exe":dict(kind="upstream", input="LICENSE.txt")}))])


class Policy(unittest.TestCase):
    def test_ci_presence_refused_even_empty(self):
        with self.assertRaisesRegex(ValueError, "CI"):
            B.guard({"CI": ""})

    def test_run_token_presence_refused_even_empty(self):
        with self.assertRaisesRegex(ValueError, "KF_RUN_TOKEN"):
            B.guard({"KF_RUN_TOKEN": ""})

    def test_person_allowed(self):
        B.guard({})

    def test_cli_ci_refused(self):
        with patch.dict(os.environ, {"CI": "false"}, clear=True), patch("sys.stderr", new_callable=io.StringIO):
            self.assertEqual(B.main(["check"]), 1)

    def test_cli_run_token_refused(self):
        with patch.dict(os.environ, {"KF_RUN_TOKEN": ""}, clear=True), patch("sys.stderr", new_callable=io.StringIO):
            self.assertEqual(B.main(["layout"]), 1)

    def test_committed_selection(self):
        m = H.load_manifest()
        p = json.loads((Path(H.AM) / "gate_profiles.json").read_text())
        chosen = B.select_packages(m, p)
        self.assertEqual({x["id"] for x in chosen}, set(p["open_packages"]))
        self.assertNotIn("ffmpeg", {x["id"] for x in chosen})

    def test_pin_itself_refuses_parent_path_before_coverage(self):
        fl = pin(); fl["name"] = "../escape"
        with self.assertRaises(ValueError): B.validate_pin("sevenzip", fl)

    def test_pin_itself_refuses_windows_path_before_coverage(self):
        fl = pin(); fl["name"] = "C:\\escape"
        with self.assertRaises(ValueError): B.validate_pin("sevenzip", fl)

    def test_unselected_ffmpeg_cannot_enter_profile(self):
        with self.assertRaises(ValueError): B.select_packages(manifest(), {"open_packages":["sevenzip","ffmpeg"]}, ['sevenzip'])

    def test_unselected_gravitymark_cannot_enter_profile(self):
        with self.assertRaises(ValueError): B.select_packages(manifest(), {"open_packages":["sevenzip","gravitymark"]}, ['sevenzip'])

    def test_forbidden_requested(self):
        with self.assertRaisesRegex(ValueError, "ffmpeg"):
            B.select_packages(manifest(), {"open_packages": ["sevenzip"]}, ["ffmpeg"])

    def test_profile_cannot_admit_licensed_package(self):
        m = manifest(); m["packages"][0]["id"] = "gravitymark"
        with self.assertRaisesRegex(ValueError, "gravitymark"):
            B.select_packages(m, {"open_packages": ["gravitymark"]})

    def test_profile_cannot_admit_ffmpeg(self):
        m = manifest(); m["packages"][0]["id"] = "ffmpeg"
        with self.assertRaisesRegex(ValueError, "ffmpeg"):
            B.select_packages(m, {"open_packages": ["ffmpeg"]})

    def test_defined_package_outside_profile_refused(self):
        m = manifest(); other = copy.deepcopy(m["packages"][0]); other["id"] = "python_embed"; m["packages"].append(other)
        with self.assertRaisesRegex(ValueError, "python_embed"):
            B.select_packages(m, {"open_packages": ["sevenzip"]}, ["python_embed"])

    def test_defined_dependency_outside_profile_refused(self):
        m = manifest(); other = copy.deepcopy(m["packages"][0]); other["id"] = "python_embed"; m["packages"].append(other)
        m["packages"][0]["needs"] = ["python_embed"]
        with self.assertRaisesRegex(ValueError, "python_embed"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_dependency_outside_profile_refused(self):
        m = manifest(); m["packages"][0]["needs"] = ["python_embed"]
        with self.assertRaisesRegex(ValueError, "python_embed"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_missing_pin_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["sha256"] = None
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_nonhex_pin_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["sha256"] = "z" * 64
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_wrong_size_pin_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["size"] = -1
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_zero_size_pin_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["size"] = 0
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_traversal_filename_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["name"] = "../escape"
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_absolute_filename_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["name"] = "C:\\bad"
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_plain_http_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["url"] = "http://vendor.invalid/7zr.exe"
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_credentials_url_refused(self):
        m = manifest(); m["packages"][0]["files"][0]["url"] = "https://user:password@vendor.invalid/7zr.exe"
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_dependency_cycle_refused(self):
        m = manifest(); m["packages"][0]["needs"] = ["sevenzip"]
        with self.assertRaisesRegex(ValueError, "cycle"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_two_node_dependency_cycle_refused(self):
        m = manifest(); other = copy.deepcopy(m["packages"][0]); other["id"] = "python_embed"
        m["packages"].append(other); m["packages"][0]["needs"] = ["python_embed"]; other["needs"] = ["sevenzip"]
        with self.assertRaisesRegex(ValueError, "cycle"):
            B.select_packages(m, {"open_packages": ["sevenzip", "python_embed"]})

    def test_case_collision_refused(self):
        m = manifest(); m["packages"][0]["files"].append(pin(name="7ZR.EXE"))
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.select_packages(m, {"open_packages": ["sevenzip"]})

    def test_empty_selection_refused(self):
        with self.assertRaises(ValueError):
            B.select_packages(manifest(), {"open_packages": []})


class Download(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.dest = Path(self.tmp.name) / "7zr.exe"
        self.data = b"installer stays intact"

    def test_cache_hash_checked(self):
        self.dest.write_bytes(b"bad")
        with patch.object(B.urllib.request, "urlopen", side_effect=AssertionError("network")):
            with self.assertRaisesRegex(ValueError, "sevenzip"):
                B.fetch_file("sevenzip", pin(), self.dest)

    def test_cache_same_size_corruption_checked(self):
        self.dest.write_bytes(b"x" * len(self.data))
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.fetch_file("sevenzip", pin(), self.dest)

    def test_valid_cache_no_network(self):
        self.dest.write_bytes(self.data)
        with patch.object(B.urllib.request, "urlopen", side_effect=AssertionError("network")):
            B.fetch_file("sevenzip", pin(), self.dest)

    def response(self, data, status=200, headers=None):
        r = io.BytesIO(data); r.status = status; r.headers = headers or {}
        return r

    def test_resume_validated(self):
        self.dest.with_suffix(".exe.part").write_bytes(self.data[:5])
        r = self.response(self.data[5:], 206, {"Content-Range": f"bytes 5-{len(self.data)-1}/{len(self.data)}"})
        with patch.object(B.urllib.request, "urlopen", return_value=r) as get:
            B.fetch_file("sevenzip", pin(), self.dest)
        self.assertEqual(get.call_args.args[0].get_header("Range"), "bytes=5-")
        self.assertEqual(self.dest.read_bytes(), self.data)

    def test_ignored_range_restarts(self):
        self.dest.with_suffix(".exe.part").write_bytes(b"old")
        with patch.object(B.urllib.request, "urlopen", return_value=self.response(self.data)):
            B.fetch_file("sevenzip", pin(), self.dest)
        self.assertEqual(self.dest.read_bytes(), self.data)

    def test_bad_range_refused(self):
        self.dest.with_suffix(".exe.part").write_bytes(self.data[:5])
        r = self.response(self.data[5:], 206, {"Content-Range": f"bytes 0-{len(self.data)-1}/{len(self.data)}"})
        with patch.object(B.urllib.request, "urlopen", return_value=r):
            with self.assertRaisesRegex(ValueError, "sevenzip"):
                B.fetch_file("sevenzip", pin(), self.dest)
        self.assertFalse(self.dest.exists())

    def test_network_hash_mismatch_never_promoted(self):
        with patch.object(B.urllib.request, "urlopen", return_value=self.response(b"x" * len(self.data))):
            with self.assertRaisesRegex(ValueError, "sevenzip"):
                B.fetch_file("sevenzip", pin(), self.dest)
        self.assertFalse(self.dest.exists())

    def test_network_size_mismatch_never_promoted(self):
        with patch.object(B.urllib.request, "urlopen", return_value=self.response(b"short")):
            with self.assertRaisesRegex(ValueError, "sevenzip"):
                B.fetch_file("sevenzip", pin(), self.dest, retries=1)
        self.assertFalse(self.dest.exists())

    def test_retries_transport_failure(self):
        with patch.object(B.urllib.request, "urlopen", side_effect=[OSError("down"), self.response(self.data)]), patch.object(B.time, "sleep"):
            B.fetch_file("sevenzip", pin(), self.dest)
        self.assertEqual(self.dest.read_bytes(), self.data)

    def test_complete_partial_promoted_without_network(self):
        self.dest.with_suffix(".exe.part").write_bytes(self.data)
        with patch.object(B.urllib.request, "urlopen", side_effect=AssertionError("network")):
            B.fetch_file("sevenzip", pin(), self.dest)
        self.assertEqual(self.dest.read_bytes(), self.data)

    def test_corrupt_complete_partial_not_promoted(self):
        self.dest.with_suffix(".exe.part").write_bytes(b"x" * len(self.data))
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.fetch_file("sevenzip", pin(), self.dest)
        self.assertFalse(self.dest.exists())

    def test_range_total_must_equal_pin(self):
        self.dest.with_suffix(".exe.part").write_bytes(self.data[:5])
        r = self.response(self.data[5:], 206, {"Content-Range": "bytes 5-20/999"})
        with patch.object(B.urllib.request, "urlopen", return_value=r):
            with self.assertRaisesRegex(ValueError, "sevenzip"):
                B.fetch_file("sevenzip", pin(), self.dest)

    def test_response_cannot_exceed_pinned_size(self):
        with patch.object(B.urllib.request, "urlopen", return_value=self.response(self.data + b"x")):
            with self.assertRaisesRegex(ValueError, "sevenzip"):
                B.fetch_file("sevenzip", pin(), self.dest)
        self.assertFalse(self.dest.exists())


class Image(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.iso = Path(self.tmp.name) / "test.iso"
        self.files = {"pkg/sevenzip/7zr.exe": b"MZ\x00unchanged", "KFAPPS.ID": b"identity\n",
                      "guest/deep/Unicode-é.txt": b"utf8", "empty": b""}

    def test_roundtrip_all_files(self):
        I.write_image(self.files, self.iso)
        with I.Reader(self.iso) as r:
            self.assertEqual(r.label, "KFAPPS")
            self.assertEqual(set(r.files), set(self.files))
            for path, data in self.files.items():
                self.assertEqual(b"".join(r.chunks(path)), data)

    def test_multisector_directory(self):
        files = {f"pkg/file-{i:04}.long": str(i).encode() for i in range(150)}
        I.write_image(files, self.iso)
        with I.Reader(self.iso) as r:
            self.assertEqual(set(r.files), set(files))
            for name in files:
                self.assertEqual(b"".join(r.chunks(name)), files[name])

    def test_repeat_build_environment(self):
        old = os.umask(0o077)
        try:
            with patch.dict(os.environ, {"TZ": "Pacific/Honolulu", "LC_ALL": "C", "SOURCE_DATE_EPOCH": "1"}):
                I.write_image(self.files, self.iso)
            a = self.iso.read_bytes()
            os.umask(0o022)
            with patch.dict(os.environ, {"TZ": "Asia/Tokyo", "LC_ALL": "C.UTF-8", "SOURCE_DATE_EPOCH": "999999999"}):
                I.write_image(dict(reversed(list(self.files.items()))), self.iso)
            self.assertEqual(a, self.iso.read_bytes())
        finally:
            os.umask(old)

    def test_path_source_metadata_ignored(self):
        source = Path(self.tmp.name) / "input"; source.write_bytes(b"original")
        I.write_image({"input": source}, self.iso); a = self.iso.read_bytes()
        os.chmod(source, 0o600); os.utime(source, (1, 1))
        I.write_image({"input": source}, self.iso)
        self.assertEqual(a, self.iso.read_bytes())

    def test_input_order_does_not_change_image(self):
        I.write_image(self.files, self.iso); first = self.iso.read_bytes()
        other = Path(self.tmp.name) / "other.iso"
        I.write_image(dict(reversed(list(self.files.items()))), other)
        self.assertEqual(first, other.read_bytes())

    def test_file_directory_collision_refused(self):
        with self.assertRaises(ValueError):
            I.write_image({"a": b"x", "a/b": b"y"}, self.iso)

    def test_joliet_overlong_filename_refused(self):
        with self.assertRaises(ValueError):
            I.write_image({"x" * 65: b"x"}, self.iso)

    def test_non_bmp_name_refused(self):
        with self.assertRaises(ValueError):
            I.write_image({"\U0001f600": b"x"}, self.iso)

    def test_individual_file_size_limit(self):
        with patch.object(I, "MAX_FILE", 1):
            with self.assertRaisesRegex(ValueError, "single-file"):
                I.write_image({"big.zip": b"xx"}, self.iso)

    def test_torch_size_is_unsigned_in_both_namespaces(self):
        size = 2915418348
        for name in (b"F0000001.BIN;1", "torch.whl".encode("utf-16-be")):
            rec = I.record(name, 123, size)
            self.assertEqual(int.from_bytes(rec[10:14], "little"), size)
            self.assertEqual(int.from_bytes(rec[14:18], "big"), size)

    def test_maximum_single_extent_length_is_unsigned(self):
        rec = I.record(b"MAX.BIN;1", 1000, 0xFFFFFFFF)
        self.assertEqual(I.Reader._number(rec, 10, 4), 0xFFFFFFFF)

    def test_traversal_refused(self):
        with self.assertRaises(ValueError):
            I.write_image({"../bad": b"x"}, self.iso)

    def test_case_alias_refused(self):
        with self.assertRaises(ValueError):
            I.write_image({"a": b"x", "A": b"y"}, self.iso)

    def test_truncated_descriptor_refused(self):
        I.write_image(self.files, self.iso)
        self.iso.write_bytes(self.iso.read_bytes()[:34000])
        with self.assertRaises(ValueError):
            I.Reader(self.iso)

    def test_corrupt_descriptor_refused(self):
        I.write_image(self.files, self.iso)
        with self.iso.open("r+b") as f:
            f.seek(16 * 2048 + 1); f.write(b"WRONG")
        with self.assertRaises(ValueError):
            I.Reader(self.iso)

    def test_endian_twin_corruption_refused(self):
        I.write_image(self.files, self.iso)
        with self.iso.open("r+b") as f:
            f.seek(17 * 2048 + 84); f.write(b"\0\0\0\0")
        with self.assertRaises(ValueError):
            I.Reader(self.iso)

    def test_root_extent_endian_twin_corruption_refused(self):
        I.write_image(self.files, self.iso)
        with self.iso.open("r+b") as f:
            f.seek(17 * 2048 + 162); f.write(b"\0\0\0\0")
        with self.assertRaises(ValueError):
            I.Reader(self.iso)

    def test_volume_size_corruption_refused(self):
        I.write_image(self.files, self.iso)
        with self.iso.open("r+b") as f:
            f.seek(17 * 2048 + 80); f.write(I.both32(1))
        with self.assertRaises(ValueError):
            I.Reader(self.iso)

    def test_joliet_label_corruption_refused(self):
        I.write_image(self.files, self.iso)
        with self.iso.open("r+b") as f:
            f.seek(17 * 2048 + 40); f.write("OTHER ".encode("utf-16-be"))
        with self.assertRaises(ValueError):
            I.Reader(self.iso)

    def test_primary_label_corruption_refused(self):
        I.write_image(self.files, self.iso)
        with self.iso.open("r+b") as f:
            f.seek(16 * 2048 + 40); f.write(b"OTHER ")
        with self.assertRaises(ValueError):
            I.Reader(self.iso)

    def corrupt_file_extent(self, extent):
        I.write_image({"file": b"payload"}, self.iso)
        data = self.iso.read_bytes()
        root = int.from_bytes(data[17 * 2048 + 158:17 * 2048 + 162], "little") * 2048
        pos = root
        for _ in range(2):
            pos += data[pos]
        with self.iso.open("r+b") as f:
            f.seek(pos + 2); f.write(I.both32(extent))

    def test_file_extent_in_system_area_refused(self):
        self.corrupt_file_extent(0)
        with self.assertRaisesRegex(ValueError, "outside"):
            I.Reader(self.iso)

    def test_file_extent_past_image_refused(self):
        self.corrupt_file_extent(0xFFFFFFF0)
        with self.assertRaisesRegex(ValueError, "outside"):
            I.Reader(self.iso)

    def invalid_empty_extents(self, extent):
        I.write_image({'empty':b''}, self.iso)
        raw = bytearray(self.iso.read_bytes())
        for sector in (16, 17):
            at = int.from_bytes(raw[sector*I.BLOCK+158:sector*I.BLOCK+162], 'little')*I.BLOCK
            for _ in range(2): at += raw[at]
            raw[at+2:at+10] = I.both32(extent)
        self.iso.write_bytes(raw)
        with self.assertRaisesRegex(ValueError, 'outside'):
            I.Reader(self.iso)

    def test_empty_payload_in_system_area_refused(self):
        self.invalid_empty_extents(0)

    def test_empty_payload_past_image_refused(self):
        self.invalid_empty_extents(0xFFFFFFF0)

    def test_fixed_recording_date(self):
        I.write_image(self.files, self.iso)
        data = self.iso.read_bytes()
        for n in (16, 17):
            self.assertEqual(data[n * 2048 + 813:n * 2048 + 830], b"2000010100000000\0")
            self.assertEqual(data[n * 2048 + 174:n * 2048 + 181], bytes((100, 1, 1, 0, 0, 0, 0)))

    def test_every_directory_record_has_fixed_date(self):
        I.write_image(self.files, self.iso)
        data = self.iso.read_bytes()
        with I.Reader(self.iso) as r:
            directories = r._visited.copy()
        for sector in directories:
            pos = sector * 2048
            while data[pos]:
                self.assertEqual(data[pos + 18:pos + 25], bytes((100, 1, 1, 0, 0, 0, 0)))
                pos += data[pos]

    def test_primary_and_joliet_share_file_extent(self):
        I.write_image({"installer.zip": b"zip bytes"}, self.iso)
        data = self.iso.read_bytes()
        extents = []
        for sector in (16, 17):
            directory = int.from_bytes(data[sector * 2048 + 158:sector * 2048 + 162], "little") * 2048
            at = directory
            for _ in range(2):
                at += data[at]
            extents.append(int.from_bytes(data[at + 2:at + 6], "little"))
        self.assertEqual(extents[0], extents[1])


    @unittest.skipUnless(shutil.which("isoinfo"), "optional independent ISO reader")
    def test_isoinfo_reads_payload_and_label(self):
        I.write_image(self.files, self.iso)
        data = subprocess.check_output(["isoinfo", "-J", "-i", str(self.iso), "-x", "/pkg/sevenzip/7zr.exe"])
        self.assertEqual(data, self.files["pkg/sevenzip/7zr.exe"])
        desc = subprocess.check_output(["isoinfo", "-i", str(self.iso), "-d"], text=True)
        self.assertIn("Volume id: KFAPPS", desc)

    @unittest.skipUnless(shutil.which("xorriso"), "optional independent ISO reader")
    def test_xorriso_reads_payload(self):
        I.write_image(self.files, self.iso)
        dest = Path(self.tmp.name) / "readback"
        subprocess.run(["xorriso", "-osirrox", "on", "-indev", str(self.iso), "-extract", "/pkg/sevenzip/7zr.exe", str(dest)], check=True, capture_output=True)
        self.assertEqual(dest.read_bytes(), self.files["pkg/sevenzip/7zr.exe"])


class Bundle(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.downloads = self.root / "dl"
        (self.downloads / "sevenzip").mkdir(parents=True)
        (self.downloads / "sevenzip" / "7zr.exe").write_bytes(b"installer stays intact")
        self.packages = manifest()["packages"]
        lp = self.downloads / "_compliance/sevenzip/LICENSE.txt"
        lp.parent.mkdir(parents=True); lp.write_bytes(b"fixture licence")
        self.out = self.root / "out"

    def build(self):
        return B.build_image(self.packages, self.downloads, {}, self.out, {})

    def test_receipt_and_verification_cover_every_file(self):
        receipt = self.build()
        B.verify_image(self.out / "winapps-open.iso", receipt)
        with I.Reader(self.out / "winapps-open.iso") as reader:
            self.assertEqual(set(reader.files), set(receipt["files"]))
        self.assertEqual((self.out / "bundle.pin.line").read_text(), B.pin_text(receipt["image"]["sha256"], receipt["inputs"], inventory_sha=receipt["inventory_sha256"]))

    def test_archives_stay_identical_on_image(self):
        receipt = self.build()
        with I.Reader(self.out / "winapps-open.iso") as r:
            self.assertEqual(b"".join(r.chunks("pkg/sevenzip/7zr.exe")), b"installer stays intact")
        self.assertIn("packaging_utc", receipt)
        self.assertIn("python_version", receipt)

    def test_repository_licences_are_on_image(self):
        receipt = self.build()
        for name in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):
            self.assertIn("LICENSES/kayfabe/" + name, receipt["files"])

    def test_repository_licence_bytes_read_back(self):
        self.build()
        with I.Reader(self.out / "winapps-open.iso") as r:
            for name in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):
                original = (Path(H.REPO) / name).read_bytes().replace(b"\r\n", b"\n")
                self.assertEqual(b"".join(r.chunks("LICENSES/kayfabe/" + name)), original)

    def test_legacy_tree_hint_does_not_extract(self):
        self.packages[0].update(image_form="tree", tree_dest="legacy")
        self.build()
        with I.Reader(self.out / "winapps-open.iso") as r:
            stage = json.loads(b"".join(r.chunks("stage.json")))
            self.assertEqual(stage["sevenzip"]["mode"], "copy")
            self.assertIn("pkg/sevenzip/7zr.exe", r.files)
            self.assertFalse(any(p.startswith("legacy/") for p in r.files))

    def test_python_legacy_tree_hint_keeps_guest_extraction(self):
        p = copy.deepcopy(self.packages[0]); p.update(id="python_embed", image_form="tree", tree_dest="py")
        p["stage"] = dict(mode="python_embed", dest="py")
        self.assertEqual(B.stage_table([p])["python_embed"]["mode"], "python_embed")

    def test_wheels_legacy_tree_hint_keeps_guest_extraction(self):
        p = copy.deepcopy(self.packages[0]); p.update(id="pywheels", image_form="tree", tree_dest="py/Lib/site-packages")
        p["stage"] = dict(mode="wheels", dest="py/Lib/site-packages")
        self.assertEqual(B.stage_table([p])["pywheels"]["mode"], "wheels")

    def test_two_empty_dirs_changed_process_environment(self):
        script = '''import os,sys,time
from pathlib import Path
sys.path.insert(0,sys.argv[1])
import package_bundle as B
os.umask(int(sys.argv[4],8))
if hasattr(time,'tzset'): time.tzset()
m=B.load(sys.argv[2]); B.build_image(m['packages'],Path(sys.argv[2]).parent/'dl',{},sys.argv[3],{})
'''
        m = self.root / "manifest.json"; m.write_bytes(B.canonical(manifest()))
        outputs = []
        for n, mask, tz, loc in (("first", "077", "Pacific/Honolulu", "C"), ("second", "022", "Asia/Tokyo", "C.UTF-8")):
            out = self.root / n
            env = dict(os.environ, TZ=tz, LC_ALL=loc)
            subprocess.run([sys.executable, "-I", "-c", script, H.AM, str(m), str(out), mask], env=env, check=True, capture_output=True)
            outputs.append(R.read_candidate(out / "winapps-open.iso", B.load(out / "winapps-open.manifest.json")))
        result=R.compare(outputs[0],outputs[1])
        self.assertEqual(result["level1"],"equal")
        if result["level2"]!="equal":warnings.warn("LEVEL2_CONTAINER_DIFF "+json.dumps(result["container_diff"],sort_keys=True))

    def test_bad_download_prevents_image(self):
        (self.downloads / "sevenzip" / "7zr.exe").write_bytes(b"x")
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            self.build()
        self.assertFalse((self.out / "winapps-open.iso").exists())

    def test_size_pin_prevents_image(self):
        self.packages[0]["files"][0]["size"] += 1
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            self.build()

    def test_same_length_corruption_prevents_image(self):
        (self.downloads / "sevenzip" / "7zr.exe").write_bytes(b"x" * len(b"installer stays intact"))
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            self.build()

    def test_image_hash_mismatch_refused(self):
        receipt = self.build(); receipt["image"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "image"):
            B.verify_image(self.out / "winapps-open.iso", receipt)

    def test_image_padding_tamper_refused(self):
        receipt = self.build(); iso = self.out / "winapps-open.iso"
        with iso.open("r+b") as f:
            f.write(b"X")
        with self.assertRaisesRegex(ValueError, "image size/sha256"):
            B.verify_image(iso, receipt)

    def test_image_size_mismatch_refused(self):
        receipt = self.build(); receipt["image"]["size"] += 1
        with self.assertRaisesRegex(ValueError, "image"):
            B.verify_image(self.out / "winapps-open.iso", receipt)

    def test_receipt_identity_mismatch_refused(self):
        receipt = self.build(); receipt["image"]["identity"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "identity"):
            B.verify_image(self.out / "winapps-open.iso", receipt)

    def test_empty_receipt_identity_refused(self):
        receipt = self.build(); receipt["image"]["identity"] = ""
        with self.assertRaisesRegex(ValueError, "identity"):
            B.verify_image(self.out / "winapps-open.iso", receipt)

    def test_missing_receipt_file_refused(self):
        receipt = self.build(); del receipt["files"]["apps.json"]
        receipt["image"]["identity"] = B.identity(receipt["files"])
        with self.assertRaisesRegex(ValueError, "file list"):
            B.verify_image(self.out / "winapps-open.iso", receipt)

    def test_extra_receipt_file_refused(self):
        receipt = self.build(); receipt["files"]["extra"] = dict(sha256="0" * 64, size=1)
        receipt["image"]["identity"] = B.identity(receipt["files"])
        with self.assertRaisesRegex(ValueError, "file list"):
            B.verify_image(self.out / "winapps-open.iso", receipt)

    def test_file_corruption_detected_after_outer_repin(self):
        receipt = self.build(); iso = self.out / "winapps-open.iso"
        with I.Reader(iso) as r:
            offset, _ = r.files["pkg/sevenzip/7zr.exe"]
        with iso.open("r+b") as f:
            f.seek(offset); f.write(b"X")
        receipt["image"]["sha256"] = B.sha256(iso)
        with self.assertRaisesRegex(ValueError, "sevenzip"):
            B.verify_image(iso, receipt)

    def tool_fixture(self, name):
        tool = self.root / name
        tool.write_bytes(b'MZ synthetic owned probe')
        receipt = B.build_image(self.packages, self.downloads, {name:tool}, self.out, {})
        return self.out / 'winapps-open.iso', receipt, 'tools/' + name

    def corrupt_owned_tool(self, name):
        iso, receipt, path = self.tool_fixture(name)
        with I.Reader(iso) as image:
            offset, _ = image.files[path]
        with iso.open('r+b') as out:
            out.seek(offset); out.write(b'XX')
        receipt['image']['sha256'] = B.sha256(iso)
        with self.assertRaisesRegex(ValueError, 'image file size/sha256 mismatch'):
            B.verify_image(iso, receipt)

    def test_owned_dxprobe_bytes_cannot_escape_receipt_check(self):
        self.corrupt_owned_tool('kf_dxprobe.exe')

    def test_owned_cuda_probe_bytes_cannot_escape_receipt_check(self):
        self.corrupt_owned_tool('cup2.exe')

    def omit_owned_tool_from_receipt(self, name):
        iso, receipt, path = self.tool_fixture(name)
        del receipt['files'][path]
        ident = B.identity(receipt['files'])
        marker = (ident + '\n').encode('ascii')
        with I.Reader(iso) as image:
            offset, _ = image.files['KFAPPS.ID']
        with iso.open('r+b') as out:
            out.seek(offset); out.write(marker)
        receipt['files']['KFAPPS.ID']['sha256'] = hashlib.sha256(marker).hexdigest()
        receipt['image'].update(identity=ident, sha256=B.sha256(iso))
        receipt['inventory_sha256'] = R.inventory_hash(receipt)
        with self.assertRaisesRegex(ValueError, 'image file list mismatch'):
            B.verify_image(iso, receipt)

    def test_owned_dxprobe_cannot_be_hidden_from_receipt(self):
        self.omit_owned_tool_from_receipt('kf_dxprobe.exe')

    def test_owned_cuda_probe_cannot_be_hidden_from_receipt(self):
        self.omit_owned_tool_from_receipt('cup2.exe')

    def test_staging_file_corruption_detected_after_outer_repin(self):
        receipt = self.build(); iso = self.out / "winapps-open.iso"
        with I.Reader(iso) as r:
            offset, _ = r.files["stage.json"]
        with iso.open("r+b") as f:
            f.seek(offset); f.write(b"X")
        receipt["image"]["sha256"] = B.sha256(iso)
        with self.assertRaisesRegex(ValueError, "stage.json"):
            B.verify_image(iso, receipt)

    def test_identity_file_corruption_detected_after_outer_repin(self):
        receipt = self.build(); iso = self.out / "winapps-open.iso"
        with I.Reader(iso) as r:
            offset, _ = r.files["KFAPPS.ID"]
        with iso.open("r+b") as f:
            f.seek(offset); f.write(b"X")
        receipt["image"]["sha256"] = B.sha256(iso)
        with self.assertRaises(ValueError):
            B.verify_image(iso, receipt)

    def repin_marker(self, content):
        receipt = self.build(); iso = self.out / "winapps-open.iso"
        with I.Reader(iso) as r:
            offset, size = r.files["KFAPPS.ID"]
        self.assertEqual(len(content), size)
        with iso.open("r+b") as f:
            f.seek(offset); f.write(content)
        receipt["image"]["sha256"] = B.sha256(iso)
        receipt["files"]["KFAPPS.ID"]["sha256"] = hashlib.sha256(content).hexdigest()
        return iso, receipt

    def test_rehashed_wrong_marker_refused(self):
        iso, receipt = self.repin_marker(b"0" * 64 + b"\n")
        with self.assertRaisesRegex(ValueError, "KFAPPS.ID"):
            B.verify_image(iso, receipt)

    def test_rehashed_marker_wrong_newline_refused(self):
        receipt = self.build()
        iso, receipt = self.repin_marker(receipt["image"]["identity"].encode() + b"X")
        with self.assertRaisesRegex(ValueError, "KFAPPS.ID"):
            B.verify_image(iso, receipt)

    def test_guest_uses_bundled_staging_table(self):
        src = (Path(H.AM) / "guest/kf_stage.ps1").read_text()
        self.assertIn("if (Test-Path $bundleTable) { $tab = Read-Json $bundleTable }", src)
        self.assertIn("else { $tab = Read-Json 'C:\\kf\\stage.json' }", src)

    def test_guest_archive_modes_match_manifest(self):
        src = (Path(H.AM) / "guest/kf_stage.ps1").read_text()
        for mode in B.STAGE_MODES:
            self.assertIn("'" + mode + "' {", src)
        self.assertIn("& tar.exe -xf", src)
        self.assertIn("& $z x -y", src)


class Toolchain(unittest.TestCase):
    def test_all_official_hosts_pinned(self):
        tc = T.check_toolchain(H.load_manifest()["bundle_toolchain"])
        for system, machine, host in (("Windows", "AMD64", "x86_64-windows"), ("Darwin", "arm64", "aarch64-macos"), ("Linux", "aarch64", "aarch64-linux")):
            self.assertEqual(T.host_pin(tc, system, machine)[0], host)

    def test_toolchain_missing_hash_refused(self):
        tc = H.load_manifest()["bundle_toolchain"]; tc["hosts"]["x86_64-linux"]["sha256"] = None
        with self.assertRaises(ValueError):
            T.check_toolchain(tc)

    def test_toolchain_bad_size_refused(self):
        tc = H.load_manifest()["bundle_toolchain"]; tc["hosts"]["aarch64-windows"]["size"] = 0
        with self.assertRaises(ValueError):
            T.check_toolchain(tc)

    def test_toolchain_unofficial_host_refused(self):
        tc = H.load_manifest()["bundle_toolchain"]; tc["hosts"]["x86_64-linux"]["url"] = "https://vendor.invalid/zig.tar.xz"
        with self.assertRaises(ValueError):
            T.check_toolchain(tc)

    def test_toolchain_official_different_version_refused(self):
        tc = H.load_manifest()["bundle_toolchain"]; tc["version"] = "0.14.0"
        with self.assertRaises(ValueError):
            T.check_toolchain(tc)

    def test_every_probe_uses_fixed_target_and_strip(self):
        source = Path("/scratch/source")
        cmds = T.build_commands(Path("/compiler/zig"), source)
        names = set()
        for c in cmds[1:]:
            names.add(c[c.index("-o") + 1])
            self.assertEqual(c[c.index("-target") + 1], "x86_64-windows-gnu")
            for flag in ("-g0", "-s", "-fno-ident", "-ffile-prefix-map=" + str(source) + "=/kf-src"):
                self.assertIn(flag, c)
        self.assertEqual(names, set(T.TOOL_NAMES))

    def test_cpp_probes_static_baseline(self):
        cpp = [c for c in T.build_commands(Path("/zig"), Path("/src")) if c[1] == "c++"]
        self.assertEqual(len(cpp), 2)
        for c in cpp:
            self.assertIn("-static", c)
            self.assertIn("-mcpu=baseline", c)

    def test_no_packaging_shell_or_external_extractor(self):
        import ast
        for module in (B, I, T):
            tree = ast.parse(Path(module.__file__).read_text())
            for call in (x for x in ast.walk(tree) if isinstance(x, ast.Call)):
                if isinstance(call.func, ast.Attribute) and (call.func.attr in ("popen", "extractall") or (call.func.attr == "system" and isinstance(call.func.value, ast.Name) and call.func.value.id == "os")):
                    self.fail("shell/unvalidated extraction: " + module.__name__)
                for kw in call.keywords:
                    if kw.arg == "shell":
                        self.assertIsInstance(kw.value, ast.Constant)
                        self.assertIs(kw.value.value, False)


@unittest.skipUnless(H.find_pwsh(), "optional PowerShell execution of relocated guest paths")
class GuestStaging(unittest.TestCase):
    def run_stage(self, mode, bundled=True):
        with tempfile.TemporaryDirectory(prefix="kf-bundle-staging-") as tmp:
            root = Path(tmp)
            kf, apps, iso = (root / n for n in ("kf", "apps", "iso"))
            kf.mkdir(); (apps / ".staged").mkdir(parents=True)
            source = iso / "pkg/sevenzip/original.zip"
            source.parent.mkdir(parents=True); source.write_bytes(b"unchanged official original fixture")
            (kf / "cd.txt").write_text(str(iso))
            table = {"sevenzip": dict(mode=mode, dest="tools", files=["original.zip"], strip_top=False)}
            (kf / "stage.json").write_bytes(B.canonical({"sevenzip": dict(mode="tree" if bundled else mode, dest="tools", files=["original.zip"])}))
            if bundled:
                (iso / "stage.json").write_bytes(B.canonical(table))
            # Execute the real scripts with only C:\kf/C:\kfapps relocated. The
            # fake tar copies a marker; no external extractor or real app runs.
            for name in ("kf_common.ps1", "kf_stage.ps1"):
                script = (Path(H.AM) / "guest" / name).read_text()
                script = script.replace("C:\\kfapps", str(apps)).replace("C:\\kf", str(kf))
                (root / name).write_text(script)
            harness = root / "run.ps1"
            stage = str(root / "kf_stage.ps1").replace("'", "''")
            harness.write_text("function global:tar.exe { Copy-Item -LiteralPath $args[1] -Destination (Join-Path $args[3] 'extracted.mock'); $global:LASTEXITCODE = 0 }\n& '" + stage + "' -Pkg sevenzip\n")
            env = dict(os.environ, POWERSHELL_TELEMETRY_OPTOUT="1", DOTNET_CLI_TELEMETRY_OPTOUT="1",
                       XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"), DOTNET_CLI_HOME=str(root / "dotnet"))
            p = subprocess.run([H.find_pwsh(), "-NoLogo", "-NoProfile", "-NonInteractive", "-File", str(harness)],
                               capture_output=True, text=True, env=env, timeout=30)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            result = apps / "tools" / ("extracted.mock" if mode == "unzip" else "original.zip")
            self.assertEqual(result.read_bytes(), source.read_bytes())
            self.assertTrue((apps / ".staged/sevenzip.json").is_file())

    def test_bundle_table_copy_overrides_legacy_tree(self):
        self.run_stage("copy")

    def test_bundle_table_archive_overrides_legacy_tree(self):
        self.run_stage("unzip")

    def test_old_image_uses_pushed_table(self):
        self.run_stage("copy", bundled=False)


class Bootstrap(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        # build_tools resolves its output root before invoking Zig. macOS /var
        # aliases and Windows short temp names must model that same real path.
        self.root = Path(self.tmp.name).resolve()
        self.tc = H.load_manifest()["bundle_toolchain"]

    def archive(self, members, tar=False):
        name = "zig-fixture.tar.xz" if tar else "zig-fixture.zip"
        path = self.root / name
        if tar:
            with tarfile.open(path, "w:xz") as z:
                for member, data in members:
                    info = tarfile.TarInfo(member); info.size = len(data)
                    z.addfile(info, io.BytesIO(data))
        else:
            with zipfile.ZipFile(path, "w") as z:
                for member, data in members:
                    z.writestr(member, data)
        data = path.read_bytes()
        return path, pin(data, name)

    def test_zip_compiler_unpacked_with_python(self):
        path, p = self.archive([("zig-fixture/zig.exe", b"compiler")])
        out = T.unpack_compiler(path, p, self.root / "unpack")
        self.assertEqual(out.read_bytes(), b"compiler")

    def test_tar_compiler_unpacked_with_python(self):
        path, p = self.archive([("zig-fixture/zig", b"compiler")], tar=True)
        out = T.unpack_compiler(path, p, self.root / "unpack")
        self.assertEqual(out.read_bytes(), b"compiler")

    def test_zip_traversal_refused(self):
        path, p = self.archive([("../escape", b"bad")])
        with self.assertRaisesRegex(ValueError, "unsafe"):
            T.unpack_compiler(path, p, self.root / "unpack")
        self.assertFalse((self.root / "escape").exists())

    def test_tar_absolute_path_refused(self):
        path, p = self.archive([("/escape", b"bad")], tar=True)
        with self.assertRaisesRegex(ValueError, "unsafe"):
            T.unpack_compiler(path, p, self.root / "unpack")

    def test_zip_symlink_refused(self):
        info = zipfile.ZipInfo("zig-fixture/zig.exe"); info.create_system = 3; info.external_attr = 0o120777 << 16
        path, p = self.archive([(info, b"target")])
        with self.assertRaisesRegex(ValueError, "special"):
            T.unpack_compiler(path, p, self.root / "unpack")

    def test_zip_device_refused(self):
        info = zipfile.ZipInfo("zig-fixture/device"); info.create_system = 3; info.external_attr = 0o020666 << 16
        path, p = self.archive([(info, b"device")])
        with self.assertRaisesRegex(ValueError, "special"):
            T.unpack_compiler(path, p, self.root / "unpack")

    def test_tar_symlink_refused(self):
        path = self.root / "zig-fixture.tar.xz"
        with tarfile.open(path, "w:xz") as z:
            info = tarfile.TarInfo("zig-fixture/zig"); info.type = tarfile.SYMTYPE; info.linkname = "../escape"; z.addfile(info)
        with self.assertRaisesRegex(ValueError, "special"):
            T.unpack_compiler(path, pin(path.read_bytes(), path.name), self.root / "unpack")

    def test_tar_hardlink_refused(self):
        path = self.root / "zig-fixture.tar.xz"
        with tarfile.open(path, "w:xz") as z:
            info = tarfile.TarInfo("zig-fixture/zig"); info.type = tarfile.LNKTYPE; info.linkname = "../escape"; z.addfile(info)
        with self.assertRaisesRegex(ValueError, "special"):
            T.unpack_compiler(path, pin(path.read_bytes(), path.name), self.root / "unpack")

    def test_compiler_archive_hash_checked(self):
        path, p = self.archive([("zig-fixture/zig.exe", b"compiler")]); p["sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            T.unpack_compiler(path, p, self.root / "unpack")

    def test_compiler_archive_size_checked(self):
        path, p = self.archive([("zig-fixture/zig.exe", b"compiler")]); p["size"] += 1
        with self.assertRaises(ValueError):
            T.unpack_compiler(path, p, self.root / "unpack")

    def build_fake(self, info=None, payload=None, rc=0):
        out = self.root / "built"
        for rel in T.NOTICE_SOURCES:
            p = self.root / rel; p.parent.mkdir(parents=True, exist_ok=True); p.write_bytes(b"synthetic compiler notice\r\n")
        def fake_run(command, **kwargs):
            self.assertFalse(kwargs["shell"])
            self.assertEqual(kwargs["env"]["SOURCE_DATE_EPOCH"], "0")
            if "-o" in command:
                name = command[command.index("-o") + 1]
                (Path(kwargs["cwd"]) / name).write_bytes(payload if payload is not None else b"MZ fake PE")
            return subprocess.CompletedProcess(command, rc, stdout=b"compiler diagnostic")
        pe = info or dict(machine=0x8664, timestamp=0, imports=["kernel32.dll"])
        with patch.object(T, "unpack_compiler", return_value=self.root / "zig"), patch.object(T.subprocess, "run", side_effect=fake_run), patch.object(T, "pe_info", return_value=pe):
            return T.build_tools(self.tc, self.root / "archive", out)

    def test_build_invokes_only_pinned_zig_argument_lists(self):
        tools = self.build_fake()
        self.assertEqual(set(tools), set(T.TOOL_NAMES) | set(T.NOTICE_NAMES))
        receipt = B.load(self.root / "built/tools.receipt.json")
        self.assertEqual(set(receipt["sources"]), set(T.SOURCES))

    def test_build_records_compiler_output_hashes(self):
        tools = self.build_fake()
        receipt = B.load(self.root / "built/tools.receipt.json")
        for name, path in tools.items():
            if name not in T.TOOL_NAMES:
                continue
            self.assertEqual(receipt["files"][name]["sha256"], B.sha256(path))

    def test_compiler_notices_are_normalized_and_returned(self):
        tools = self.build_fake()
        for name in T.NOTICE_NAMES:
            self.assertEqual(tools[name].read_bytes(), b"synthetic compiler notice\n")

    def test_compiler_notice_missing_refused(self):
        with self.assertRaises(OSError):
            T.copy_notices(self.root / "absent", self.root / "notices")

    def test_gui_launcher_compiles_and_uses_same_entrypoint(self):
        path = Path(H.AM) / "package_bundle_gui.pyw"
        source = path.read_text()
        compile(source, str(path), "exec")
        self.assertIn("package_bundle.main(args)", source)
        self.assertNotIn("subprocess", source)

    def test_nonzero_pe_timestamp_refused(self):
        with self.assertRaisesRegex(ValueError, "timestamp"):
            self.build_fake(info=dict(timestamp=1, imports=[]))

    def test_wallclock_pe_timestamp_refused(self):
        with self.assertRaisesRegex(ValueError, "timestamp"):
            self.build_fake(info=dict(timestamp=1794080000, imports=[]))

    def test_dynamic_libstdcxx_refused(self):
        with self.assertRaisesRegex(ValueError, "runtime"):
            self.build_fake(info=dict(timestamp=0, imports=["libstdc++-6.dll"]))

    def test_dynamic_libgcc_refused(self):
        with self.assertRaisesRegex(ValueError, "runtime"):
            self.build_fake(info=dict(timestamp=0, imports=["libgcc_s_seh-1.dll"]))

    def test_ascii_build_path_leak_refused(self):
        with self.assertRaisesRegex(ValueError, "path"):
            self.build_fake(payload=str(self.root / "built").encode())

    def test_utf16_build_path_leak_refused(self):
        with self.assertRaisesRegex(ValueError, "path"):
            self.build_fake(payload=str(self.root / "built").encode("utf-16-le"))

    def test_compile_failure_refused(self):
        with self.assertRaisesRegex(ValueError, "pinned zig failed"):
            self.build_fake(rc=1)

    def test_import_library_failure_refused(self):
        with self.assertRaisesRegex(ValueError, "nvcuda import library"):
            self.build_fake(rc=2)



if __name__ == "__main__":
    unittest.main()
