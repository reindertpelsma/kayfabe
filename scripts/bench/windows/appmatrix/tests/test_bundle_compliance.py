# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Compliance fixtures contain invented text, never vendor binaries or credentials."""
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

import helpers as H
import package_bundle as B
import bundle_compliance as C
import bundle_ci as CI
import publish_bundle as P
from test_bundle_publish import MemoryStore


def pin(name, data):
    return dict(name=name, url="https://upstream.invalid/" + name,
                size=len(data), sha256=hashlib.sha256(data).hexdigest())


def package():
    return dict(id="sevenzip", version="1.2.3", license="MIT", redistribution="Redistributable",
                stage=dict(mode="copy", dest="tools"), files=[pin("LICENSE.txt", b"licence text")],
                compliance=dict(license_id="MIT", upstream="https://upstream.invalid/project",
                    distribution="public-ok", conditions=[], sources=[], license_inputs=[],
                    license_rules={"LICENSE.txt": dict(kind="file", file="LICENSE.txt")}))


class Rules(unittest.TestCase):
    def validate(self, p, public=False): return C.validate([p], B.validate_pin, public)

    def test_valid_permissive(self): self.validate(package(), True)

    def test_absent_compliance_refused(self):
        p = package(); del p["compliance"]
        with self.assertRaises(ValueError): self.validate(p)

    def test_empty_compliance_refused(self):
        p = package(); p["compliance"] = {}
        with self.assertRaises(ValueError): self.validate(p)

    def test_public_refuses_private(self):
        p = package(); p["compliance"].update(distribution="private-only", conditions=["Review CUDA EULA"])
        with self.assertRaises(ValueError): self.validate(p, True)

    def test_public_inventory_rejects_private_member(self):
        a=package();b=package();b['id']='python_embed';b['compliance'].update(distribution='private-only',conditions=['private'])
        with self.assertRaises(ValueError):C.validate([a,b],B.validate_pin,True)

    def test_removed_conditions_class_refused(self):
        p = package(); p["compliance"].update(distribution="conditions", conditions=["Retain runtime terms"])
        with self.assertRaises(ValueError): self.validate(p, True)

    def test_unknown_distribution_refused(self):
        p = package(); p["compliance"]["distribution"] = "probably-ok"
        with self.assertRaises(ValueError): self.validate(p)

    def test_empty_condition_refused(self):
        p = package(); p["compliance"]["distribution"] = "conditions"
        with self.assertRaises(ValueError): self.validate(p)

    def test_all_binary_files_need_coverage(self):
        p = package(); p["files"].append(pin("other.exe", b"binary"))
        with self.assertRaises(ValueError): self.validate(p)

    def test_empty_coverage_refused(self):
        p = package(); p["compliance"]["license_rules"] = {}
        with self.assertRaises(ValueError): self.validate(p)

    def test_gpl_requires_mirror(self):
        p = package(); p["compliance"]["license_id"] = "GPL-3.0-only"
        with self.assertRaises(ValueError): self.validate(p)

    def test_lgpl_requires_mirror(self):
        p = package(); p["compliance"]["license_id"] = "LGPL-2.1-or-later"
        with self.assertRaises(ValueError): self.validate(p)

    def test_wrong_source_version_refused(self):
        p = package(); p["compliance"]["sources"] = [dict(pin("source.tar.xz", b"src"), version="0.1")]
        with self.assertRaises(ValueError): self.validate(p)

    def test_missing_source_version_refused(self):
        p = package(); p["compliance"]["sources"] = [pin("source.tar.xz", b"src")]
        with self.assertRaises(ValueError): self.validate(p)

    def test_unpinned_license_refused(self):
        p = package(); p["compliance"]["license_inputs"] = [dict(name="NOTICE", url="https://upstream.invalid/LICENSE")]
        with self.assertRaises(ValueError): self.validate(p)

    def test_unpinned_source_refused(self):
        p = package(); p["compliance"]["sources"] = [dict(name="source", version="1.2.3")]
        with self.assertRaises(ValueError): self.validate(p)

    def test_committed_manifest_has_complete_rules(self):
        self.validate_all = B.select_packages(H.load_manifest(), B.load(Path(H.AM) / "gate_profiles.json"))
        C.validate(self.validate_all, B.validate_pin)

    def test_committed_cuda_is_private(self):
        byid = {p["id"]: p for p in H.load_manifest()["packages"]}
        for pid in ("cuda_demo_suite", "cuda_cudart", "cuda_nvrtc", "pywheels"):
            self.assertEqual(byid[pid]["compliance"]["distribution"], "private-only")

    def test_committed_conditional_terms_are_private(self):
        byid = {p["id"]: p for p in H.load_manifest()["packages"]}
        for pid in ("vulkan_rt", "vc_redist"):
            self.assertEqual(byid[pid]["compliance"]["distribution"], "private-only")

    def test_missing_id_refused(self):
        p = package(); p["compliance"]["license_id"] = ""
        with self.assertRaises(ValueError): self.validate(p)

    def test_non_https_upstream_refused(self):
        p = package(); p["compliance"]["upstream"] = "http://upstream.invalid/project"
        with self.assertRaises(ValueError): self.validate(p)

    def test_default_public_cli_selects_only_public_packages(self):
        from unittest.mock import patch
        with patch.dict(os.environ, {}, clear=True), patch("sys.stderr", new_callable=io.StringIO):
            self.assertEqual(B.main(["check", "--class", "public"]), 0)

    def test_public_input_hash_selects_public_packages(self):
        from unittest.mock import patch
        with patch.dict(os.environ, {}, clear=True), patch("sys.stderr", new_callable=io.StringIO):
            self.assertEqual(B.main(["input-hash", "--class", "public"]), 0)

    def test_compliance_recipe_is_hashed(self):
        from unittest.mock import patch
        m, profile = H.load_manifest(), B.load(Path(H.AM) / "gate_profiles.json")
        before, read = B.input_hash(m, profile), Path.read_bytes
        def altered(path): return read(path) + (b"# changed" if path.name == "bundle_compliance.py" else b"")
        with patch.object(Path, "read_bytes", altered): self.assertNotEqual(before, B.input_hash(m, profile))

    def test_source_pin_change_is_hashed(self):
        m, profile = H.load_manifest(), B.load(Path(H.AM) / "gate_profiles.json")
        before = B.input_hash(m, profile)
        next(p for p in m["packages"] if p["id"] == "blender")["compliance"]["sources"][0]["sha256"] = "0" * 64
        self.assertNotEqual(before, B.input_hash(m, profile))

    def test_larger_source_invalidates_disk_admission(self):
        from unittest.mock import patch
        m, profile = H.load_manifest(), B.load(Path(H.AM) / "gate_profiles.json")
        admitted = CI.disk_budget(m, profile)
        next(p for p in m["packages"] if p["id"] == "blender")["compliance"]["sources"][0]["size"] += 1
        with patch.object(CI.shutil, "disk_usage", return_value=type("Disk", (), {"free": admitted})()):
            with self.assertRaises(ValueError): CI.check_disk(".", CI.disk_budget(m, profile))

    def test_larger_licence_input_invalidates_disk_admission(self):
        from unittest.mock import patch
        m, profile = H.load_manifest(), B.load(Path(H.AM) / "gate_profiles.json")
        admitted = CI.disk_budget(m, profile)
        next(p for p in m["packages"] if p["id"] == "qwen_gguf")["compliance"]["license_inputs"][0]["size"] += 1
        with patch.object(CI.shutil, "disk_usage", return_value=type("Disk", (), {"free": admitted})()):
            with self.assertRaises(ValueError): CI.check_disk(".", CI.disk_budget(m, profile))


class Artifacts(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.p = package()
        self.cache = self.root / "dl"; (self.cache / "sevenzip").mkdir(parents=True)
        (self.cache / "sevenzip/LICENSE.txt").write_bytes(b"licence text")

    def stage(self): return C.stage([self.p], self.cache)

    def zip(self, names):
        data = io.BytesIO()
        with zipfile.ZipFile(data, "w") as z:
            for name, body in names.items(): z.writestr(name, body)
        raw = data.getvalue(); (self.cache / "sevenzip/app.zip").write_bytes(raw)
        self.p["files"] = [pin("app.zip", raw)]
        self.p["compliance"]["license_rules"] = {"app.zip": dict(kind="zip")}
        return raw

    def test_license_text_inside_iso_layout(self):
        files, meta, sources = self.stage()
        self.assertIn(b"licence text", files.values())
        self.assertTrue(any(k.startswith("LICENSES/sevenzip/") for k in files))

    def test_license_index_records_original(self):
        files, _, _ = self.stage(); index = json.loads(files["LICENSES/index.json"])
        self.assertEqual(index[0]["sha256"], hashlib.sha256(b"licence text").hexdigest())

    def test_direct_license_copy_rechecks_bytes_read(self):
        from unittest.mock import patch
        with patch.object(C, "read_text", return_value=b"changed after verification"):
            with self.assertRaises(ValueError): self.stage()

    def test_upstream_license_copy_rechecks_bytes_read(self):
        from unittest.mock import patch
        self.zip({"deps/NOTICE": b"notice"}); self.upstream_fallback()
        with patch.object(C, "read_text", return_value=b"changed after verification"):
            with self.assertRaises(ValueError): self.stage()

    def test_zip_copies_only_license_text(self):
        raw = self.zip({"app/LICENSE.txt": b"MIT copyright", "app/app.exe": b"binary"})
        files, _, _ = self.stage()
        self.assertIn(b"MIT copyright", files.values()); self.assertNotIn(b"binary", files.values())
        self.assertEqual((self.cache / "sevenzip/app.zip").read_bytes(), raw)

    def test_nested_wheel_notice_copied(self):
        self.zip({"lib.dist-info/licenses/LICENSE": b"BSD text", "lib.dist-info/NOTICE": b"third party"})
        files, _, _ = self.stage()
        self.assertIn(b"BSD text", files.values()); self.assertIn(b"third party", files.values())

    def test_runtime_prefixed_license_copied(self):
        self.zip({"runtime/VulkanRT-License.txt": b"runtime redistribution terms"})
        files, _, _ = self.stage(); self.assertIn(b"runtime redistribution terms", files.values())

    def test_cuda_eula_copied(self):
        self.zip({"torch/NVIDIA-CUDA-EULA.txt": b"NVIDIA terms"})
        files, _, _ = self.stage(); self.assertIn(b"NVIDIA terms", files.values())

    def test_standalone_eula_copied(self):
        self.zip({"EULA.txt": b"runtime terms"})
        files, _, _ = self.stage(); self.assertIn(b"runtime terms", files.values())

    def upstream_fallback(self):
        data = b"main program licence"; lp = pin("LICENSE", data)
        self.p["compliance"]["license_inputs"] = [lp]
        self.p["compliance"]["license_rules"]["app.zip"]["fallback"] = "LICENSE"
        path = self.cache / "_compliance/sevenzip/LICENSE"; path.parent.mkdir(parents=True); path.write_bytes(data)
        return data

    def test_upstream_main_license_also_copied_with_dependency_notice(self):
        self.zip({"deps/LICENSE": b"dependency MIT"}); main = self.upstream_fallback()
        files, _, _ = self.stage(); self.assertIn(main, files.values())

    def test_upstream_main_license_has_its_own_index_provenance(self):
        self.zip({"deps/NOTICE": b"dependency notice"}); self.upstream_fallback()
        files, _, _ = self.stage(); entries = json.loads(files["LICENSES/index.json"])
        self.assertTrue(any(e["origin"] == "https://upstream.invalid/LICENSE" for e in entries))

    def test_archive_without_license_refused(self):
        self.zip({"app.exe": b"binary"})
        with self.assertRaisesRegex(ValueError, "licence"): self.stage()

    def test_wheel_without_notice_refused(self):
        self.zip({"module.py": b"pass"})
        with self.assertRaisesRegex(ValueError, "licence"): self.stage()

    def test_corrupt_zip_names_package_in_failure(self):
        self.zip({"LICENSE": b"legal text"}); path = self.cache / "sevenzip/app.zip"; path.write_bytes(b"not a ZIP")
        self.p["files"] = [pin("app.zip", b"not a ZIP")]
        with self.assertRaisesRegex(ValueError, "package sevenzip/app.zip"): self.stage()

    def test_corrupt_source_names_package_in_failure(self):
        path, src = self.source(); path.write_bytes(b"not a tar")
        src.update(pin("source.tar.xz", b"not a tar"))
        self.p["compliance"]["license_rules"] = {"LICENSE.txt":dict(kind="source", input="source.tar.xz")}
        with self.assertRaisesRegex(ValueError, "package sevenzip/source.tar.xz"): self.stage()

    def test_large_license_refused(self):
        self.zip({"LICENSE": b"x" * (4 * 1024**2 + 1)})
        with self.assertRaises(ValueError): self.stage()

    def test_empty_license_refused(self):
        self.zip({"LICENSE": b""})
        with self.assertRaises(ValueError): self.stage()

    def test_large_upstream_license_refused(self):
        data = b"x" * (4 * 1024**2 + 1); lp = pin("LICENSE", data)
        self.p["compliance"].update(license_inputs=[lp], license_rules={"LICENSE.txt":dict(kind="upstream", input="LICENSE")})
        path = self.cache / "_compliance/sevenzip/LICENSE"; path.parent.mkdir(parents=True); path.write_bytes(data)
        with self.assertRaises(ValueError): self.stage()

    def test_legal_text_reader_does_not_request_unbounded_allocation(self):
        from unittest.mock import patch
        class Trap(io.BytesIO):
            def read(self, size=-1):
                if size < 0 or size > 4 * 1024**2 + 1: raise RuntimeError("unbounded allocation requested")
                return super().read(size)
        with patch.object(Path, "open", return_value=Trap(b"text")):
            self.assertEqual(C.read_text("fixture"), b"text")

    def test_large_legal_file_read_stops_at_cap(self):
        path = self.root / "large-licence"; path.write_bytes(b"x" * (4 * 1024**2 + 128))
        self.assertEqual(len(C.read_text(path)), 4 * 1024**2 + 1)

    def source(self):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w:xz") as t:
            item = tarfile.TarInfo("src/COPYING"); item.size = 7; t.addfile(item, io.BytesIO(b"GPLtext"))
        data = stream.getvalue(); p = dict(pin("source.tar.xz", data), version=self.p["version"])
        self.p["compliance"]["sources"] = [p]
        path = self.cache / "_compliance/sevenzip/source.tar.xz"; path.parent.mkdir(parents=True); path.write_bytes(data)
        return path, p

    def test_exact_source_archive_is_unmodified(self):
        path, pin = self.source(); files, _, sources = self.stage()
        self.assertEqual(files["SOURCE/sevenzip/source.tar.xz"], path)
        self.assertEqual(sources[0]["sha256"], B.sha256(path))

    def test_source_key_content_addressed_same_prefix(self):
        _, pin = self.source(); _, _, sources = self.stage()
        self.assertEqual(sources[0]["key"], "winapps-open/source-" + pin["sha256"] + ".archive")

    def test_source_bad_hash_refused(self):
        path, pin = self.source(); pin["sha256"] = "0" * 64
        with self.assertRaises(ValueError): self.stage()

    def test_source_truncation_refused(self):
        path, _ = self.source(); path.write_bytes(b"bad")
        with self.assertRaises(ValueError): self.stage()

    def test_offer_and_distributor_obligation(self):
        self.source(); files, _, _ = self.stage(); notice = files["THIRD_PARTY_NOTICES.txt"].decode()
        self.assertIn("at least three years", notice); self.assertIn("OUR obligation", notice)
        self.assertIn("https://upstream.invalid/source.tar.xz", notice)

    def test_offer_mirror_is_not_only_upstream_link(self):
        _, pin = self.source(); files, _, _ = self.stage(); notice = files["THIRD_PARTY_NOTICES.txt"].decode()
        self.assertIn("SOURCE/sevenzip/source.tar.xz", notice)
        self.assertIn("winapps-open/source-" + pin["sha256"], notice)

    def test_offer_minimum_term_is_stated_separately(self):
        self.source(); files, _, _ = self.stage()
        self.assertIn("at least three years after our last distribution", files["THIRD_PARTY_NOTICES.txt"].decode())

    def test_notice_has_exact_release_and_license(self):
        files, _, _ = self.stage(); notice = files["THIRD_PARTY_NOTICES.txt"].decode()
        for text in ("sevenzip", "1.2.3", "MIT", "https://upstream.invalid/project"): self.assertIn(text, notice)

    def test_each_package_file_gets_distribution(self):
        files, meta, _ = self.stage()
        self.assertEqual(meta["pkg/sevenzip/LICENSE.txt"][0]["distribution"], "public-ok")

    def test_full_receipt_licenses_bound_and_verified(self):
        receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {})
        self.assertTrue(all(f.get("licenses") for f in receipt["files"].values()))
        B.verify_image(self.root / "out/winapps-open.iso", receipt)

    def test_receipt_license_tamper_refused(self):
        receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {})
        receipt["files"]["pkg/sevenzip/LICENSE.txt"]["licenses"][0]["id"] = "other"
        with self.assertRaises(ValueError): B.verify_image(self.root / "out/winapps-open.iso", receipt)

    def test_receipt_binary_has_actual_package_license(self):
        receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {})
        self.assertEqual(receipt["files"]["pkg/sevenzip/LICENSE.txt"]["licenses"][0]["id"], "MIT")

    def test_receipt_license_retains_upstream(self):
        receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {})
        self.assertEqual(receipt["files"]["pkg/sevenzip/LICENSE.txt"]["licenses"][0]["upstream"], self.p["compliance"]["upstream"])

    def test_receipt_source_tamper_refused(self):
        self.source(); receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {})
        receipt["sources"][0]["sha256"] = "0" * 64
        with self.assertRaises(ValueError): B.verify_image(self.root / "out/winapps-open.iso", receipt)

    def test_sources_and_licenses_reproduce_across_environment_and_path(self):
        self.source(); before = {k: os.environ.get(k) for k in ("TZ", "LC_ALL")}; mask = os.umask(0o022)
        try:
            os.environ.update(TZ="Pacific/Honolulu", LC_ALL="C")
            first = B.build_image([self.p], self.cache, {}, self.root / "first", {})
            os.umask(0o077); os.environ.update(TZ="Asia/Tokyo", LC_ALL="C.UTF-8")
            second = B.build_image([self.p], self.cache, {}, self.root / "second with spaces", {})
        finally:
            os.umask(mask)
            for key, value in before.items():
                if value is None: os.environ.pop(key, None)
                else: os.environ[key] = value
        self.assertEqual(first["image"]["sha256"], second["image"]["sha256"])
        self.assertEqual((self.root / "first/winapps-open.iso").read_bytes(), (self.root / "second with spaces/winapps-open.iso").read_bytes())

    def test_removed_source_receipt_refused(self):
        self.source(); receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {})
        receipt["sources"] = []
        with self.assertRaises(ValueError): B.verify_image(self.root / "out/winapps-open.iso", receipt)

    def test_public_verify_refuses_private_receipt(self):
        self.p["compliance"].update(distribution="private-only", conditions=["Review needed"])
        receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {}, bundle_class="private")
        with self.assertRaises(ValueError): B.verify_image(self.root / "out/winapps-private.iso", receipt, True)

    def build_source(self):
        self.source()
        receipt = B.build_image([self.p], self.cache, {}, self.root / "out", {})
        return receipt, self.root / "out/winapps-open.iso"

    def test_source_is_published_before_iso(self):
        receipt, iso = self.build_source(); store = MemoryStore()
        P.publish(store, iso, receipt, self.root / "state.json")
        self.assertEqual(list(store.objects)[0], receipt["sources"][0]["key"])

    def test_mirrored_source_bytes_are_exact(self):
        receipt, iso = self.build_source(); store = MemoryStore()
        P.publish(store, iso, receipt, self.root / "state.json")
        key = receipt["sources"][0]["key"]
        self.assertEqual(hashlib.sha256(store.objects[key][1]).hexdigest(), receipt["sources"][0]["sha256"])

    def test_missing_source_receipt_prevents_object_code_upload(self):
        receipt, iso = self.build_source(); receipt["sources"] = []; store = MemoryStore()
        with self.assertRaises(ValueError): P.publish(store, iso, receipt, self.root / "state.json")
        self.assertEqual(store.objects, {})

    def test_changed_source_pin_prevents_upload(self):
        receipt, iso = self.build_source(); receipt["sources"][0]["size"] += 1; store = MemoryStore()
        with self.assertRaises(ValueError): P.publish(store, iso, receipt, self.root / "state.json")
        self.assertEqual(store.objects, {})

    def test_failed_source_upload_prevents_iso_publication(self):
        receipt, iso = self.build_source(); store = MemoryStore(); store.fail_part = 1
        from unittest.mock import patch
        with patch.object(P.time, "sleep"), self.assertRaises(P.StoreError):
            P.publish(store, iso, receipt, self.root / "state.json")
        self.assertEqual(store.objects, {})

    def test_failed_source_completion_prevents_iso_publication(self):
        receipt, iso = self.build_source(); store = MemoryStore(); store.embedded_error = True
        with self.assertRaises(P.PublishError): P.publish(store, iso, receipt, self.root / "state.json")
        self.assertEqual(store.objects, {})

    def test_existing_source_reused_without_overwrite(self):
        receipt, iso = self.build_source(); store = MemoryStore()
        P.publish(store, iso, receipt, self.root / "state.json")
        store.calls.clear(); P.publish(store, iso, receipt, self.root / "state.json")
        self.assertFalse(any(method in ("POST", "PUT") for method, _, _, _ in store.calls))

    def test_source_identity_independent_of_iso_recipe(self):
        receipt, iso = self.build_source(); store = MemoryStore()
        P.publish_sources(store, iso, receipt, self.root / "state.json", P.DEFAULT_PART)
        receipt["inputs"] = "0" * 64; store.calls.clear()
        P.publish_sources(store, iso, receipt, self.root / "state.json", P.DEFAULT_PART)
        self.assertFalse(any(method in ("POST", "PUT") for method, _, _, _ in store.calls))
