# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline CI planning / S3 protocol falsifiers. No credentials or network."""
import copy
import datetime
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET
import urllib.error

import helpers as H
import package_bundle as B
import bundle_ci as C
import publish_bundle as P


class Inputs(unittest.TestCase):
    def setUp(self):
        self.man = B.load(Path(H.AM) / "manifest.json")
        self.prof = B.load(Path(H.AM) / "gate_profiles.json")

    def test_stable_hash(self):
        self.assertEqual(B.input_hash(self.man, self.prof), B.input_hash(copy.deepcopy(self.man), copy.deepcopy(self.prof)))

    def test_pin_change_invalidates(self):
        before = B.input_hash(self.man, self.prof)
        next(p for p in self.man["packages"] if p["id"] == "python_embed")["files"][0]["sha256"] = "0" * 64
        self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_toolchain_change_invalidates(self):
        before = B.input_hash(self.man, self.prof)
        self.man["bundle_toolchain"]["hosts"]["x86_64-linux"]["sha256"] = "0" * 64
        self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_ci_input_hash_has_exact_output(self):
        with patch.dict(os.environ, {"CI": "true"}, clear=True), patch("sys.stdout", new_callable=io.StringIO) as out:
            self.assertEqual(B.main(["input-hash"]), 0)
        self.assertRegex(out.getvalue(), r"\Ainputs=[0-9a-f]{64}\n\Z")

    def test_explicit_ci_build_allowed_without_credentials(self):
        B.guard({"CI": "true"}, allow_ci=True)

    def test_ci_build_rejects_r2_secret(self):
        with self.assertRaises(ValueError):
            B.guard({"CI": "true", "AWS_SECRET_ACCESS_KEY": "synthetic"}, allow_ci=True)

    def test_ci_build_rejects_r2_key_id(self):
        with self.assertRaises(ValueError):
            B.guard({"CI": "true", "AWS_ACCESS_KEY_ID": "synthetic"}, allow_ci=True)

    def test_run_token_remains_refused(self):
        with self.assertRaises(ValueError):
            B.guard({"CI": "true", "KF_RUN_TOKEN": ""}, allow_ci=True)

    def test_builder_source_change_invalidates(self):
        before = B.input_hash(self.man, self.prof)
        with patch.object(B, "BUILDER_VERSION", B.BUILDER_VERSION + 1):
            self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_open_set_change_invalidates(self):
        before = B.input_hash(self.man, self.prof)
        self.prof["open_packages"].remove("hf_model")
        self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_guest_bytes_change_invalidates(self):
        before = B.input_hash(self.man, self.prof); original = B.repo_files
        def altered(*args):
            data = original(*args); data["guest/kf_stage.ps1"] += b"# changed\n"; return data
        with patch.object(B, "repo_files", side_effect=altered):
            self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_probe_bytes_change_invalidates(self):
        before = B.input_hash(self.man, self.prof); original = Path.read_bytes
        def altered(path):
            data = original(path)
            return data + b"/* changed */" if path.name == "cup8.c" else data
        with patch.object(Path, "read_bytes", altered):
            self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_dx_source_change_invalidates(self):
        before = B.input_hash(self.man, self.prof); original = Path.read_bytes
        def altered(path):
            data = original(path)
            return data + b"/* changed */" if path.name == "kf_dxprobe.cpp" else data
        with patch.object(Path, "read_bytes", altered): self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_iso_writer_change_invalidates(self):
        before = B.input_hash(self.man, self.prof); original = Path.read_bytes
        def altered(path):
            data = original(path)
            return data + b"# changed\n" if path.name == "bundle_iso.py" else data
        with patch.object(Path, "read_bytes", altered): self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_builder_file_change_invalidates(self):
        before = B.input_hash(self.man, self.prof); original = Path.read_bytes
        def altered(path):
            data = original(path)
            return data + b"# changed\n" if path.name == "package_bundle.py" else data
        with patch.object(Path, "read_bytes", altered): self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_license_change_invalidates(self):
        before = B.input_hash(self.man, self.prof); original = B.repo_files
        def altered(*args):
            data = original(*args); data["LICENSES/kayfabe/LICENSE"] += b"changed"; return data
        with patch.object(B, "repo_files", side_effect=altered): self.assertNotEqual(before, B.input_hash(self.man, self.prof))

    def test_host_receipt_metadata_does_not_invalidate(self):
        before = B.input_hash(self.man, self.prof)
        self.man["image"].update(built_utc="2099-01-01", sha256="0" * 64)
        self.assertEqual(before, B.input_hash(self.man, self.prof))

    def test_publisher_is_not_an_iso_input(self):
        before = B.input_hash(self.man, self.prof); original = Path.read_bytes
        def altered(path): return original(path) + (b"changed" if path.name == "publish_bundle.py" else b"")
        with patch.object(Path, "read_bytes", altered): self.assertEqual(before, B.input_hash(self.man, self.prof))


class Planning(unittest.TestCase):
    def test_same_inputs_skip(self):
        self.assertFalse(C.plan("a" * 64, B.pin_text("b" * 64, "a" * 64))["needs_build"])

    def test_same_inputs_other_image_pin_skip(self):
        self.assertFalse(C.plan("a" * 64, B.pin_text("c" * 64, "a" * 64))["needs_build"])

    def test_changed_inputs_build(self):
        self.assertTrue(C.plan("c" * 64, B.pin_text("b" * 64, "a" * 64))["needs_build"])

    def test_empty_pin_build(self):
        self.assertTrue(C.plan("a" * 64, "# EMPTY\n")["needs_build"])

    def test_verify_forces_rebuild(self):
        self.assertTrue(C.plan("a" * 64, B.pin_text("b" * 64, "a" * 64), verify=True)["needs_build"])

    def test_content_addressed_keys(self):
        sha = "a" * 64
        self.assertEqual(P.keys(sha), (f"winapps-open/kfapps-{sha}.iso", f"winapps-open/manifest-{sha}.json"))

    def test_non_digest_key_refused(self):
        with self.assertRaises(ValueError):
            P.keys("../override")

    def test_uppercase_digest_key_refused(self):
        with self.assertRaises(ValueError): P.keys("A" * 64)

    def test_more_than_five_gib_uses_multiple_parts(self):
        ranges = P.part_ranges(6 * 1024**3, 64 * 1024**2)
        self.assertGreater(len(ranges), 1)
        self.assertEqual(sum(size for _, _, size in ranges), 6 * 1024**3)
        self.assertTrue(all(size >= P.MIN_PART for _, _, size in ranges[:-1]))

    def test_parts_cover_tail(self):
        ranges = P.part_ranges(P.MIN_PART * 2 + 7, P.MIN_PART)
        self.assertEqual(ranges[-1][2], 7)
        self.assertEqual([n for n, _, _ in ranges], [1, 2, 3])

    def test_no_image_means_no_input_skip(self):
        with self.assertRaises(ValueError): C.read_pin("# inputs=" + "a" * 64)

    def test_duplicate_inputs_refused(self):
        with self.assertRaises(ValueError): C.read_pin(B.pin_text("b" * 64, "a" * 64) + "# inputs=" + "c" * 64)

    def test_insufficient_disk_refused_before_download(self):
        usage = type("Disk", (), {"free": 14 * 1024**3})()
        with patch.object(C.shutil, "disk_usage", return_value=usage):
            with self.assertRaises(ValueError): C.check_disk(".", 22_378_011_791)

    def test_no_disk_refused(self):
        usage = type("Disk", (), {"free": 0})()
        with patch.object(C.shutil, "disk_usage", return_value=usage):
            with self.assertRaises(ValueError): C.check_disk(".", 1)

    def test_too_many_parts_refused(self):
        with self.assertRaises(ValueError): P.part_ranges(P.MIN_PART * 10001, P.MIN_PART)

    def test_service_part_count_limit_refused(self):
        with self.assertRaises(ValueError): P.part_ranges(P.MIN_PART * 20000, P.MIN_PART)

    def test_part_below_service_minimum_refused(self):
        with self.assertRaises(ValueError): P.part_ranges(100, P.MIN_PART - 1)

    def test_zero_part_size_refused(self):
        with self.assertRaises(ValueError): P.part_ranges(100, 0)

    def test_verification_without_pin_refused(self):
        with self.assertRaises(ValueError): C.plan("a" * 64, "", verify=True)

    def test_verification_comment_only_pin_refused(self):
        with self.assertRaises(ValueError): C.plan("a" * 64, "# EMPTY\n", verify=True)


class BuildOrchestration(unittest.TestCase):
    def run_fake(self, hashes, pin_sha="a" * 64, verify=False):
        inputs = B.input_hash(B.load(Path(H.AM) / "manifest.json"), B.load(Path(H.AM) / "gate_profiles.json"))
        sequence = iter(hashes)
        def fake_main(args):
            if args[0] == "build":
                out = Path(args[args.index("--out") + 1]); out.mkdir()
                (out / "winapps-open.manifest.json").write_bytes(B.canonical(dict(inputs=inputs, image=dict(sha256=next(sequence)))))
            return 0
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, {}, clear=True), patch.object(B, "main", side_effect=fake_main), patch.object(B, "verify_image"), patch.object(C, "check_disk"):
            return C.build_twice(Path(root) / "fresh", inputs, B.pin_text(pin_sha, inputs), verify)

    def test_equal_builds_match_pin(self):
        self.assertEqual(self.run_fake(["a" * 64, "a" * 64]), "a" * 64)

    def test_different_second_build_refused(self):
        with self.assertRaises(ValueError): self.run_fake(["a" * 64, "b" * 64])

    def test_other_different_second_build_refused(self):
        with self.assertRaises(ValueError): self.run_fake(["a" * 64, "c" * 64])

    def test_verification_mismatch_refused(self):
        with self.assertRaises(ValueError): self.run_fake(["a" * 64, "a" * 64], "b" * 64, True)

    def test_other_verification_mismatch_refused(self):
        with self.assertRaises(ValueError): self.run_fake(["a" * 64, "a" * 64], "c" * 64, True)


class Signing(unittest.TestCase):
    # Public AWS documentation examples, never real credentials.
    ACCESS = "AKIAIOSFODNN" + "7EXAMPLE"
    SECRET = "wJalrXUtnFEMI/" + "K7MDENG/bPxRfiCYEXAMPLEKEY"

    def sign(self, method, path, headers=None, data=b""):
        return P.sign_headers(method, "https://examplebucket.s3.amazonaws.com" + path, headers or {}, data,
                              self.ACCESS, self.SECRET, "20130524T000000Z", "us-east-1")

    def test_aws_get_reference_vector(self):
        result = self.sign("GET", "/test.txt", {"Range": "bytes=0-9"})
        self.assertTrue(result["authorization"].endswith("f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"))

    def test_aws_lifecycle_reference_vector(self):
        result = self.sign("GET", "/?lifecycle=")
        self.assertTrue(result["authorization"].endswith("fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"))

    def test_aws_put_reference_vector(self):
        result = self.sign("PUT", "/test%24file.text", {"Date": "Fri, 24 May 2013 00:00:00 GMT", "x-amz-storage-class": "REDUCED_REDUNDANCY"}, b"Welcome to Amazon S3.")
        self.assertTrue(result["authorization"].endswith("98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"))

    def test_content_payload_is_signed(self):
        h = self.sign("PUT", "/key", data=b"fixture")
        self.assertEqual(h["x-amz-content-sha256"], hashlib.sha256(b"fixture").hexdigest())
        self.assertNotEqual(h["authorization"], self.sign("PUT", "/key", data=b"other")["authorization"])

    def test_condition_is_signed(self):
        h = self.sign("POST", "/key?uploadId=a%2Bb", {"If-None-Match": "*"})
        self.assertIn("if-none-match", h["authorization"])

    def test_query_order_does_not_change_signature(self):
        self.assertEqual(self.sign("PUT", "/key?uploadId=a%2Bb&partNumber=1"), self.sign("PUT", "/key?partNumber=1&uploadId=a%2Bb"))

    def test_encoded_query_values_keep_order_independent(self):
        self.assertEqual(self.sign("GET", "/?z=%2F%2B&a=two%20words"), self.sign("GET", "/?a=two+words&z=%2F%2B"))

    def test_transport_error_does_not_log_capability(self):
        with patch.object(P.urllib.request, "build_opener") as opener:
            opener.return_value.open.side_effect = urllib.error.URLError("private-capability-value")
            with self.assertRaises(P.PublishError) as error: P.send("PUT", "https://store.invalid/key?private-capability-value", {}, b"x")
        self.assertNotIn("private-capability-value", str(error.exception))

    def test_redirects_refused(self):
        with self.assertRaises(P.PublishError): P.NoRedirect().redirect_request(None, None, None, None, None, None)

    def test_https_redirect_also_refused(self):
        with self.assertRaises(P.PublishError): P.NoRedirect().redirect_request(None, None, 307, "redirect", {}, "https://other.invalid/key")


class MemoryStore:
    def __init__(self):
        self.objects = {}; self.uploads = {}; self.calls = []; self.fail_part = None
        self.embedded_error = False; self.on_part = None; self.condition_race = False

    def request(self, method, key, query=None, headers=None, data=b""):
        query = query or {}; headers = headers or {}; self.calls.append((method, key, query.copy(), headers.copy()))
        if method == "HEAD":
            if key not in self.objects: raise P.StoreError(404)
            return self.objects[key][0], b""
        if method == "POST" and "uploads" in query:
            uid = str(len(self.uploads) + 1); self.uploads[uid] = dict(key=key, headers=headers.copy(), parts={})
            return {}, f"<InitiateMultipartUploadResult><UploadId>{uid}</UploadId></InitiateMultipartUploadResult>".encode()
        if "uploadId" in query:
            upload = self.uploads[query["uploadId"]]
            if method == "GET":
                root = ET.Element("ListPartsResult"); ET.SubElement(root, "IsTruncated").text = "false"
                for n, (etag, body) in sorted(upload["parts"].items()):
                    item = ET.SubElement(root, "Part")
                    for tag, value in (("PartNumber", n), ("ETag", etag), ("Size", len(body))): ET.SubElement(item, tag).text = str(value)
                return {}, ET.tostring(root)
            if method == "PUT":
                n = int(query["partNumber"])
                if n == self.fail_part: raise P.StoreError(503)
                etag = '"' + hashlib.md5(data).hexdigest() + '"'; upload["parts"][n] = (etag, data)
                if self.on_part: self.on_part(n)
                return {"etag": etag}, b""
            if method == "DELETE":
                del self.uploads[query["uploadId"]]; return {}, b""
            if method == "POST":
                if headers.get("If-None-Match") != "*": raise AssertionError("unconditional multipart completion")
                if self.embedded_error: return {}, b"<Error><Code>InternalError</Code></Error>"
                root = ET.fromstring(data); body = b""
                for part in root:
                    n = int(part.findtext("PartNumber")); etag = part.findtext("ETag")
                    assert etag == upload["parts"][n][0]; body += upload["parts"][n][1]
                meta = dict(upload["headers"], **{"content-length": str(len(body))})
                if self.condition_race:
                    if self.condition_race != "absent": self.objects[key] = (meta, body)
                    raise P.StoreError(412)
                if key in self.objects: raise P.StoreError(412)
                self.objects[key] = (meta, body)
                return {}, b"<CompleteMultipartUploadResult><ETag>done</ETag></CompleteMultipartUploadResult>"
        if method == "PUT":
            if headers.get("If-None-Match") != "*": raise AssertionError("unconditional object PUT")
            if key in self.objects: raise P.StoreError(412)
            self.objects[key] = (dict(headers, **{"content-length": str(len(data))}), data); return {}, b""
        if method == "GET": return self.objects[key]
        raise AssertionError((method, key, query))


class Multipart(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "fixture.iso"; self.path.write_bytes(b"abcdefghijkl")
        self.state = Path(self.tmp.name) / "state.json"; self.store = MemoryStore()
        self.digest = B.sha256(self.path); self.inputs = "a" * 64; self.key = P.keys(self.digest)[0]; self.part_size = 4
        self.minimum = patch.object(P, "MIN_PART", 4); self.minimum.start(); self.addCleanup(self.minimum.stop)

    def upload(self):
        with patch.object(P.time, "sleep"):
            P.multipart(self.store, self.path, self.key, self.digest, self.inputs, self.state, part_size=self.part_size)

    def test_upload_exact_bytes(self):
        self.upload(); self.assertEqual(self.store.objects[self.key][1], self.path.read_bytes()); self.assertFalse(self.state.exists())

    def test_presigned_oversized_buffer_refused(self):
        self.part_size = P.DEFAULT_PART + 1
        with self.assertRaises(ValueError): self.upload()
        self.assertEqual(self.store.calls, [])

    def test_service_maximum_does_not_override_memory_limit(self):
        self.part_size = P.MAX_PART
        with self.assertRaises(ValueError): self.upload()
        self.assertEqual(self.store.calls, [])

    def test_completion_has_create_only_condition(self):
        self.upload()
        complete = [c for c in self.store.calls if c[0] == "POST" and "uploadId" in c[2]]
        self.assertEqual(complete[0][3]["If-None-Match"], "*")

    def test_existing_key_not_overwritten(self):
        headers = dict(P.metadata(self.digest, self.inputs), **{"content-length": "12"})
        self.store.objects[self.key] = (headers, self.path.read_bytes()); self.upload()
        self.assertEqual([c[0] for c in self.store.calls], ["HEAD"])

    def test_existing_conflicting_key_refused(self):
        self.store.objects[self.key] = ({"content-length": "12"}, b"existing")
        with self.assertRaises(P.PublishError): self.upload()
        self.assertEqual(self.store.objects[self.key][1], b"existing")

    def test_existing_wrong_size_refused(self):
        self.store.objects[self.key] = (dict(P.metadata(self.digest, self.inputs), **{"content-length": "1"}), b"x")
        with self.assertRaises(P.PublishError): self.upload()

    def test_resume_does_not_reupload_verified_part(self):
        self.store.fail_part = 2
        with self.assertRaises(P.StoreError): self.upload()
        self.assertTrue(self.state.exists()); self.store.fail_part = None; self.upload()
        self.assertEqual(sum(c[0] == "PUT" and c[2].get("partNumber") == "1" for c in self.store.calls), 1)

    def test_remote_etag_change_reuploads(self):
        self.store.fail_part = 2
        with self.assertRaises(P.StoreError): self.upload()
        self.store.uploads["1"]["parts"][1] = ('"different"', b"bad!")
        self.store.fail_part = None; self.upload()
        self.assertEqual(sum(c[0] == "PUT" and c[2].get("partNumber") == "1" for c in self.store.calls), 2)
        self.assertEqual(self.store.objects[self.key][1], self.path.read_bytes())

    def test_remote_part_size_change_reuploads(self):
        self.store.fail_part = 2
        with self.assertRaises(P.StoreError): self.upload()
        etag, _ = self.store.uploads["1"]["parts"][1]; self.store.uploads["1"]["parts"][1] = (etag, b"x")
        self.store.fail_part = None; self.upload()
        self.assertEqual(sum(c[0] == "PUT" and c[2].get("partNumber") == "1" for c in self.store.calls), 2)

    def test_resume_binding_refuses_other_input_hash(self):
        self.store.fail_part = 2
        with self.assertRaises(P.StoreError): self.upload()
        self.inputs = "b" * 64; self.store.fail_part = None; prior = len(self.store.calls)
        with self.assertRaises(P.PublishError): self.upload()
        self.assertTrue(all(c[0] == "HEAD" for c in self.store.calls[prior:]))
        self.assertNotIn(self.key, self.store.objects)

    def test_resume_binding_refuses_other_part_size(self):
        self.store.fail_part = 2
        with self.assertRaises(P.StoreError): self.upload()
        state = B.load(self.state); state["part_size"] = 8; self.state.write_bytes(B.canonical(state)); self.store.fail_part = None
        with self.assertRaises(P.PublishError): self.upload()

    def test_embedded_error_is_failure(self):
        self.store.embedded_error = True
        with self.assertRaises(P.PublishError): self.upload()
        self.assertNotIn(self.key, self.store.objects)

    def test_embedded_namespaced_error_is_failure(self):
        with self.assertRaises(P.PublishError): P.xml_root(b'<Error xmlns="urn:s3"><Code>failed</Code></Error>')

    def test_plain_embedded_error_rejected_by_parser(self):
        with self.assertRaises(P.PublishError): P.xml_root(b"<Error><Code>InternalError</Code></Error>")

    def test_concurrent_valid_creator_is_not_overwritten(self):
        self.store.condition_race = True; self.upload()
        self.assertEqual(self.store.objects[self.key][1], self.path.read_bytes())
        self.assertTrue(any(c[0] == "DELETE" for c in self.store.calls))

    def test_conflict_drops_aborted_resume_state(self):
        self.store.condition_race = "absent"
        with self.assertRaises(P.PublishError): self.upload()
        self.assertFalse(self.state.exists())

    def test_conflict_retry_can_start_fresh(self):
        self.store.condition_race = "absent"
        with self.assertRaises(P.PublishError): self.upload()
        self.store.condition_race = False; self.upload()
        self.assertEqual(self.store.objects[self.key][1], self.path.read_bytes())

    def test_changed_bytes_refused_before_complete(self):
        self.path.write_bytes(b"ABCDEFGHIJKL")
        with self.assertRaises(P.PublishError): self.upload()
        self.assertFalse(any(c[0] == "POST" and "uploadId" in c[2] for c in self.store.calls))


    def test_late_byte_change_refused_before_complete(self):
        self.path.write_bytes(b"x" * 24576); self.digest = B.sha256(self.path); self.key = P.keys(self.digest)[0]; self.part_size = 4096
        def change(n):
            if n == 1:
                with self.path.open("r+b") as f:
                    f.seek(20480); f.write(b"Z" * 4096)
        self.store.on_part = change
        with self.assertRaises(P.PublishError): self.upload()


class ReceiptPublishing(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup); self.root = Path(self.tmp.name)
        downloads = self.root / "downloads"; p = downloads / "sevenzip/7zr.exe"; p.parent.mkdir(parents=True); p.write_bytes(b"fixture")
        from test_package_bundle import manifest
        packages = manifest()["packages"]; packages[0]["files"][0].update(sha256=B.sha256(p), size=7)
        lp = downloads / "_compliance/sevenzip/LICENSE.txt"; lp.parent.mkdir(parents=True); lp.write_bytes(b"fixture licence")
        self.receipt = B.build_image(packages, downloads, {}, self.root / "out", {})
        self.iso = self.root / "out/winapps-open.iso"; self.state = self.root / "state.json"; self.store = MemoryStore()

    def publish(self): return P.publish(self.store, self.iso, self.receipt, self.state)

    def test_iso_and_receipt_keys_both_exist(self):
        pair = self.publish(); self.assertEqual(set(pair), set(self.store.objects))

    def test_json_create_only_condition(self):
        self.publish(); puts = [c for c in self.store.calls if c[0] == "PUT" and not c[2]]
        self.assertEqual(len(puts), 1); self.assertEqual(puts[0][3]["If-None-Match"], "*")

    def test_second_publish_does_not_replace_receipt(self):
        _, key = self.publish(); original = self.store.objects[key][1]
        self.receipt["packaging_utc"] = "2099-01-01"; self.publish()
        self.assertEqual(self.store.objects[key][1], original)
        self.assertEqual(sum(c[0] == "PUT" and not c[2] for c in self.store.calls), 1)

    def test_local_corrupt_image_stops_before_network(self):
        with self.iso.open("r+b") as f: f.write(b"X")
        with self.assertRaises(P.PublishError): self.publish()
        self.assertEqual(self.store.calls, [])

    def test_local_size_mismatch_stops_before_network(self):
        self.receipt["image"]["size"] += 1
        with self.assertRaises(P.PublishError): self.publish()
        self.assertEqual(self.store.calls, [])

    def test_existing_bad_receipt_is_not_overwritten(self):
        key = P.keys(self.receipt["image"]["sha256"])[1]; self.store.objects[key] = ({}, b'{}')
        with self.assertRaises(P.PublishError): self.publish()
        self.assertEqual(self.store.objects[key][1], b'{}')

    def test_receipt_inputs_bound_to_image(self):
        self.receipt["inputs"] = "0" * 64
        with self.assertRaises(ValueError): B.verify_image(self.iso, self.receipt)

    def test_receipt_empty_inputs_bound_to_image(self):
        self.receipt["inputs"] = ""
        with self.assertRaises(ValueError): B.verify_image(self.iso, self.receipt)


class Presigned(unittest.TestCase):
    def setUp(self):
        self.key = P.keys("a" * 64)[0]
        self.url = "https://store.invalid/bucket/" + self.key
        self.plans = {self.key: dict(upload_id="u", head=self.url, parts={"1": self.url + "?uploadId=u&partNumber=1"},
                                    complete=self.url + "?uploadId=u&X-Amz-SignedHeaders=host%3Bif-none-match")}
        self.sender = lambda *args: ({}, b"done")

    def test_part_url_and_method_from_caller(self):
        with patch.object(P, "send") as send:
            send.return_value = ({}, b""); store = P.PresignedStore(self.plans, sender=send)
            store.request("PUT", self.key, {"uploadId": "u", "partNumber": "1"}, data=b"data")
            self.assertEqual(send.call_args.args[0], "PUT"); self.assertEqual(send.call_args.args[3], b"data")

    def test_insecure_capability_refused(self):
        self.plans[self.key]["head"] = self.url.replace("https:", "http:")
        with self.assertRaises(P.PublishError): P.PresignedStore(self.plans, self.sender).request("HEAD", self.key)

    def test_wrong_key_capability_refused(self):
        self.plans[self.key]["head"] = "https://store.invalid/other"
        with self.assertRaises(P.PublishError): P.PresignedStore(self.plans, self.sender).request("HEAD", self.key)

    def test_unsigned_completion_condition_refused(self):
        self.plans[self.key]["complete"] = self.url + "?uploadId=u&X-Amz-SignedHeaders=host"
        with self.assertRaises(P.PublishError): P.PresignedStore(self.plans, self.sender).request("POST", self.key, {"uploadId": "u"}, {"If-None-Match": "*"})

    def test_missing_completion_signed_headers_refused(self):
        self.plans[self.key]["complete"] = self.url + "?uploadId=u"
        with self.assertRaises(P.PublishError): P.PresignedStore(self.plans, self.sender).request("POST", self.key, {"uploadId": "u"}, {"If-None-Match": "*"})



class Workflow(unittest.TestCase):
    def setUp(self):
        self.text = (Path(H.REPO) / ".github/workflows/bundle-publish.yml").read_text()

    def test_dispatch_only_disabled_default(self):
        triggers = self.text.split("on:\n", 1)[1].split("permissions:", 1)[0]
        self.assertIn("workflow_dispatch:", triggers)
        for forbidden in ("pull_request:", "push:", "schedule:"):
            self.assertNotIn(forbidden, triggers)

    def test_no_artifact_or_private_release_store(self):
        for token in ("upload-artifact", "download-artifact", "gh release", "actions/cache", "pull_request_target"):
            self.assertNotIn(token, self.text)

    def test_build_has_no_environment_or_secret_reference(self):
        build = self.text.split("\n  build:\n", 1)[1].split("\n  build-and-publish:", 1)[0]
        self.assertNotIn("secrets.", build)
        self.assertNotIn("environment:", build)

    def test_secret_names_occur_only_in_protected_read_and_matching_publish_steps(self):
        public = self.text.split("- name: Publish immutable objects to public bucket",1)[1].split("- name: Publish immutable objects to private bucket",1)[0]
        private = self.text.split("- name: Publish immutable objects to private bucket",1)[1].split("- name: Remove ephemeral",1)[0]
        read = self.text.split("- id: private\n        name: Read selected private manifest",1)[1].split("- id: matrix",1)[0]
        self.assertEqual(self.text.count("${{ secrets."), 6)
        for block, allowed, forbidden in ((public,'BUNDLE_PUBLIC_R2','BUNDLE_PRIVATE_R2'),(private,'BUNDLE_PRIVATE_R2','BUNDLE_PUBLIC_R2'),(read,'BUNDLE_PRIVATE_R2','BUNDLE_PUBLIC_R2')):
            self.assertEqual(block.count("${{ secrets."),2)
            self.assertIn("secrets."+allowed,block);self.assertNotIn("secrets."+forbidden,block)

    def test_publisher_protected_master_and_flag(self):
        job = self.text.split("\n  build-and-publish:\n", 1)[1].split("    steps:", 1)[0]
        for phrase in ("vars.BUNDLE_PUBLISH_ENABLED == 'true'", "refs/heads/master", "github.event_name == 'push'", "github.event_name == 'workflow_dispatch'", "environment: bundle-publish-master"):
            self.assertIn(phrase, job)

    def test_actions_pinned_and_minimal_permissions(self):
        import re
        self.assertEqual(len(re.findall(r"uses: actions/checkout@[0-9a-f]{40} # v", self.text)), 4)
        self.assertNotIn(": write", self.text.split("permissions:",1)[1].split("concurrency:",1)[0])


class PublishGuard(unittest.TestCase):
    def context(self):
        return dict(GITHUB_REPOSITORY="reindertpelsma/kayfabe", GITHUB_REF="refs/heads/master",
                    GITHUB_EVENT_NAME="workflow_dispatch", GITHUB_ACTIONS="true", BUNDLE_PUBLISH_APPROVED="true")

    def test_master_dispatch_allowed(self):
        P.guard_publish(self.context())

    def test_pull_request_refused(self):
        e = self.context(); e["GITHUB_EVENT_NAME"] = "pull_request"
        with self.assertRaises(ValueError): P.guard_publish(e)

    def test_pull_request_target_refused(self):
        e = self.context(); e["GITHUB_EVENT_NAME"] = "pull_request_target"
        with self.assertRaises(ValueError): P.guard_publish(e)

    def test_fork_repository_refused(self):
        e = self.context(); e["GITHUB_REPOSITORY"] = "fork/kayfabe"
        with self.assertRaises(ValueError): P.guard_publish(e)

    def test_other_owner_repository_refused(self):
        e = self.context(); e["GITHUB_REPOSITORY"] = "other/kayfabe"
        with self.assertRaises(ValueError): P.guard_publish(e)

    def test_other_branch_refused(self):
        e = self.context(); e["GITHUB_REF"] = "refs/heads/next"
        with self.assertRaises(ValueError): P.guard_publish(e)

    def test_tag_refused(self):
        e = self.context(); e["GITHUB_REF"] = "refs/tags/master"
        with self.assertRaises(ValueError): P.guard_publish(e)

    def test_unapproved_environment_refused(self):
        e = self.context(); del e["BUNDLE_PUBLISH_APPROVED"]
        with self.assertRaises(ValueError): P.guard_publish(e)

    def test_local_invocation_refused(self):
        e = self.context(); del e["GITHUB_ACTIONS"]
        with self.assertRaises(ValueError): P.guard_publish(e)


if __name__ == "__main__":
    unittest.main()
