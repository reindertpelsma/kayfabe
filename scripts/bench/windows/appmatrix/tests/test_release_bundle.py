# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The optional release assets must be source-bound and completely hash-checked before packaging."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import helpers as H
import release_bundle as B


class Bundle(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.cfg = json.loads((Path(H.AM) / "release_software.json").read_text())
        self.files = {}
        for name in self.cfg["directx"]["required_files"]:
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(b"synthetic test-only asset: " + name.encode())
            if name == "directx/GATE_VRS.patch":
                p.write_bytes((Path(H.AM) / "directx_vrs_gate.patch").read_bytes())
            self.files[name] = hashlib.sha256(p.read_bytes()).hexdigest()
        self.receipt = dict(schema=1, source_commit=self.cfg["directx"]["commit"],
                            source_repository=self.cfg["directx"]["repository"],
                            built_projects=self.cfg["directx"]["projects"], patches=self.cfg["directx"]["patches"], files=self.files)
        self.save()

    def save(self):
        (self.root / "receipt.json").write_text(json.dumps(self.receipt))

    def test_source_bound_complete_assets_are_packaged(self):
        plan = B.assets(self.root, self.cfg)
        self.assertEqual(set(plan), set(self.files) | {"receipt.json"})

    def test_asset_paths_reach_the_iso_layout_with_the_release_prefix(self):
        import appdisk
        plan = dict(appdisk.plan_layout(H.load_manifest(), release_assets=self.root))
        for name in self.files:
            self.assertEqual(plan["release/" + name], ("release", name))

    def test_bom_receipt_from_windows_powershell_is_supported(self):
        (self.root / "receipt.json").write_text(json.dumps(self.receipt), encoding="utf-8-sig")
        self.assertIn("receipt.json", B.assets(self.root, self.cfg))

    def test_release_assets_are_hash_recorded_in_a_real_iso(self):
        import contextlib
        import io
        import shutil
        import appdisk
        if not any(shutil.which(t) for t in ("genisoimage", "xorriso", "mkisofs")):
            self.skipTest("no ISO builder")
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(io.StringIO()):
            out = Path(d)
            manifest = out / "manifest.json"
            manifest.write_text(json.dumps(dict(schema=1, packages=[], image=dict(file="kfapps.iso", label="KFAPPS"))))
            self.assertEqual(appdisk.main(["build", "--manifest", str(manifest), "--dl", str(out / "dl"),
                                          "--release-assets", str(self.root), "--out", str(out / "image")]), 0)
            built = json.loads((out / "image/manifest.json").read_text())
            for name, sha in self.files.items():
                self.assertEqual(built["files"]["release/" + name]["sha256"], sha)
            self.assertTrue((out / "image/kfapps.iso").is_file())

    def test_wrong_or_missing_source_pin_is_refused(self):
        for commit in (None, "0" * 40):
            self.receipt["source_commit"] = commit
            self.save()
            with self.assertRaises(ValueError):
                B.assets(self.root, self.cfg)

    def test_wrong_project_set_is_refused(self):
        self.receipt["built_projects"] = {}
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_missing_vrs_patch_pin_is_refused(self):
        self.receipt["patches"] = {}
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_vrs_patch_cannot_be_replaced_even_with_a_consistent_file_hash(self):
        p = self.root / "directx/GATE_VRS.patch"
        p.write_bytes(b"wrong patch")
        self.files["directx/GATE_VRS.patch"] = hashlib.sha256(p.read_bytes()).hexdigest()
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_missing_required_binary_cannot_be_packaged(self):
        (self.root / "directx/dxr/D3D12RaytracingHelloWorld.exe").unlink()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_missing_required_asset_cannot_be_hidden_by_receipt(self):
        name = "directx/Assets/Dragon_LOD0.bin"
        (self.root / name).unlink()
        del self.files[name]
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_modified_asset_bytes_are_refused(self):
        name = next(iter(self.files))
        (self.root / name).write_bytes(b"changed")
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_a_receipt_cannot_relabel_the_hash_of_a_built_shader(self):
        self.files["directx/mesh/MeshletMS.cso"] = "0" * 64
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_an_extra_binary_outside_the_receipt_is_refused(self):
        (self.root / "extra.exe").write_bytes(b"MZ test only")
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_an_empty_tree_and_empty_receipt_are_refused(self):
        import shutil
        for p in list(self.root.iterdir()):
            if p.is_dir():
                shutil.rmtree(p)
            else:
                p.unlink()
        self.receipt["files"] = {}
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_unrecorded_extra_and_empty_receipts_are_refused(self):
        (self.root / "unrecorded.txt").write_text("extra")
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)
        self.receipt["files"] = {}
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)

    def test_path_traversal_and_links_are_refused(self):
        self.files["../escape"] = "0" * 64
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)
        del self.files["../escape"]
        p = self.root / "linked"
        p.symlink_to(self.root / next(iter(self.files)))
        self.save()
        with self.assertRaises(ValueError):
            B.assets(self.root, self.cfg)


if __name__ == "__main__":
    unittest.main()
