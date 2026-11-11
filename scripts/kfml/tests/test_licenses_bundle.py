import hashlib
import json
import os
import tarfile
import tempfile
import time
import unittest
import zipfile
from pathlib import Path

from tests import helpers  # noqa: F401
from kfml import buildwheels, bundle, licenses
from kfml.spec import Upstream

MIT_TEXT = "MIT License\n\nPermission is hereby granted, free of charge, to any person obtaining a copy ...\n"
CUSTOM_TEXT = "Custom research licence. Non-commercial use only. Redistribution forbidden.\n"


def make_wheel(d, name, version, meta_lines, files=None, lic_files=None, tags="py3-none-any"):
    fn = "%s-%s-%s.whl" % (name, version, tags)
    p = Path(d) / fn
    with zipfile.ZipFile(p, "w") as z:
        di = "%s-%s.dist-info" % (name, version)
        z.writestr(di + "/METADATA", "Metadata-Version: 2.1\nName: %s\nVersion: %s\n%s\n" % (name, version, "\n".join(meta_lines)))
        for n, t in (lic_files or {}).items():
            z.writestr(di + "/" + n, t)
        for n, b in (files or {}).items():
            z.writestr(n, b)
    return str(p)


class Licenses(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def test_allowlisted_expression(self):
        w = make_wheel(self.t, "good", "1.0", ["License-Expression: MIT"], lic_files={"licenses/LICENSE": MIT_TEXT})
        s = licenses.scan_wheel(w)
        self.assertEqual((s["license_spdx"], s["redistributable"]), ("MIT", True))

    def test_allowlisted_classifier_and_bsd_sniff(self):
        w = make_wheel(self.t, "cls", "1.0", ["Classifier: License :: OSI Approved :: MIT License"])
        self.assertTrue(licenses.scan_wheel(w)["redistributable"])
        bsd = "Redistribution and use in source and binary forms ... Neither the name of X"
        w = make_wheel(self.t, "bsd", "1.0", ["Classifier: License :: OSI Approved :: BSD License"], lic_files={"LICENSE": bsd})
        s = licenses.scan_wheel(w)
        self.assertEqual(s["license_spdx"], "BSD-3-Clause")
        w = make_wheel(self.t, "bsdx", "1.0", ["Classifier: License :: OSI Approved :: BSD License"])
        self.assertFalse(licenses.scan_wheel(w)["redistributable"])  # 2 vs 3 clause unknown

    def test_missing_licence_is_fetch_only(self):
        w = make_wheel(self.t, "nolic", "1.0", [])
        s = licenses.scan_wheel(w)
        self.assertFalse(s["redistributable"])
        self.assertIn("UNKNOWN", s["fetch_only_reason"])
        w = make_wheel(self.t, "unk", "1.0", ["License: UNKNOWN"])
        self.assertFalse(licenses.scan_wheel(w)["redistributable"])

    def test_custom_licence_is_fetch_only(self):
        w = make_wheel(self.t, "tabby", "1.0", ["License: TabPFN License v1.0 (custom)"], lic_files={"LICENSE": CUSTOM_TEXT})
        s = licenses.scan_wheel(w)
        self.assertFalse(s["redistributable"])
        self.assertIsNone(s["license_spdx"])
        w = make_wheel(self.t, "gpl", "1.0", ["License-Expression: GPL-3.0-only"])
        self.assertFalse(licenses.scan_wheel(w)["redistributable"])
        w = make_wheel(self.t, "conflict", "1.0", ["License-Expression: MIT"], lic_files={"LICENSE": "Apache License Version 2.0"})
        self.assertFalse(licenses.scan_wheel(w)["redistributable"])

    def test_or_and_expressions(self):
        self.assertIsNotNone(licenses._norm_expr("MIT OR GPL-3.0-only"))
        self.assertIsNone(licenses._norm_expr("MIT AND GPL-3.0-only"))
        self.assertIsNotNone(licenses._norm_expr("Apache-2.0 AND MIT"))

    def test_nvidia_binary_wheel(self):
        w = make_wheel(self.t, "okname", "1.0", ["License-Expression: MIT"],
                       files={"okname/libcudnn_engines.so.9": b"x", "okname/other.so": b"y"})
        s = licenses.scan_wheel(w)
        self.assertFalse(s["redistributable"])
        self.assertIn("libcudnn_engines.so.9", s["nvidia_origin_libs"])
        self.assertIn("other.so", s["bundled_libs"])
        w = make_wheel(self.t, "triton", "3.3.1", ["License-Expression: MIT"], files={"triton/backends/nvidia/bin/ptxas": b"x"})
        self.assertFalse(licenses.scan_wheel(w)["redistributable"])
        w = make_wheel(self.t, "nvidia_cublas_cu12", "12.8.4.1", ["License: Proprietary"])
        self.assertFalse(licenses.scan_wheel(w)["redistributable"])
        w = make_wheel(self.t, "torch", "2.7.1+cu128", ["License-Expression: BSD-3-Clause"], files={"torch/lib/libtorch_cuda.so": b"x"})
        s = licenses.scan_wheel(w)
        self.assertFalse(s["redistributable"])
        self.assertIn("CUDA build", s["fetch_only_reason"])

    def test_tags_recorded(self):
        w = make_wheel(self.t, "ext", "0.1", ["License-Expression: MIT"], tags="cp312-cp312-manylinux_2_28_x86_64")
        self.assertEqual(licenses.scan_wheel(w)["tags"], "cp312-cp312-manylinux_2_28_x86_64")

    def test_build_record(self):
        w = make_wheel(self.t, "flash_attn", "2.8.0", ["License-Expression: BSD-3-Clause"],
                       files={"flash_attn/_C.so": b"x"}, tags="cp312-cp312-linux_x86_64")
        up = Upstream("flash-attn", "https://github.com/Dao-AILab/flash-attention", "d" * 40, "BSD-3-Clause", "fetched")
        r = buildwheels.record_for(licenses.scan_wheel(w), up, {"python": "3.12", "torch": "2.7.1+cu128", "cuda": "12.8"}, "glibc-2.28", "e" * 64)
        self.assertEqual((r["tags"], r["cuda_flavor"], r["glibc"], r["upstream_commit"]), ("cp312-cp312-linux_x86_64", "12.8", "glibc-2.28", "d" * 40))
        self.assertFalse(r["refused"])
        w2 = make_wheel(self.t, "bad_ext", "1", ["License-Expression: MIT"], files={"bad_ext/libcublas.so.12": b"x"})
        self.assertTrue(buildwheels.record_for(licenses.scan_wheel(w2), up, {}, "g", None)["refused"])

    def test_arch_list(self):
        self.assertTrue(buildwheels.arch_list("12.8").endswith("12.0+PTX"))
        self.assertNotIn("12.0", buildwheels.arch_list("12.6"))
        self.assertTrue(buildwheels.arch_list("12.6").endswith("9.0+PTX"))


class Bundle(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = Path(self.tmp.name)
        self.wheels = self.t / "wheels"
        self.wheels.mkdir()
        make_wheel(self.wheels, "good", "1.0", ["License-Expression: MIT"])
        make_wheel(self.wheels, "tabby", "1.0", ["License: custom"])
        make_wheel(self.wheels, "triton", "3.3.1", ["License-Expression: MIT"], files={"triton/ptxas": b"x"})
        self.pins = {"datasets": {"d": {"url": "https://e/d", "filename": "d", "sha256": "0" * 64, "license": "MIT"},
                                  "e": {"url": "https://e/e", "filename": "e", "sha256": None, "license": "MIT", "redistribution_ok": True}}}
        self.tree = self.t / "tree"
        (self.tree / "sub").mkdir(parents=True)
        (self.tree / "a.py").write_text("x=1\n")
        (self.tree / "sub" / "b.py").write_text("y=2\n")

    def tearDown(self):
        self.tmp.cleanup()

    def build(self, out):
        return bundle.build(out, str(self.wheels), self.pins, [], root=self.tree)

    def test_selection(self):
        r = self.build(self.t / "o1")
        comps = {c["name"]: c for c in r["manifest"]["components"]}
        self.assertTrue(comps["good"]["redistributable"])
        for n in ("tabby", "triton"):
            self.assertFalse(comps[n]["redistributable"], n)
            self.assertTrue(comps[n]["fetch_only_reason"])
        self.assertFalse(comps["d"]["redistributable"])  # redistribution_ok not set -> fetch-only
        self.assertTrue(comps["e"]["redistributable"])
        with tarfile.open(r["path"]) as tf:
            names = tf.getnames()
        self.assertIn("wheels/good-1.0-py3-none-any.whl", names)
        self.assertFalse([n for n in names if "tabby" in n or "triton" in n])
        self.assertEqual(names, sorted(names))

    def test_reproducible(self):
        a = self.build(self.t / "o1")
        for p in self.tree.rglob("*"):
            os.utime(p, (time.time() + 5000, time.time() + 5000))
        b = self.build(self.t / "o2")
        self.assertEqual(a["sha256"], b["sha256"])
        self.assertEqual(hashlib.sha256(Path(a["path"]).read_bytes()).hexdigest(), a["sha256"])
        with tarfile.open(a["path"]) as tf:
            for m in tf.getmembers():
                self.assertEqual((m.mtime, m.uid, m.gid), (0, 0, 0))
        json.loads((self.t / "o1" / "MANIFEST.json").read_text())

    def test_real_tree_bundle_has_vendored_notice(self):
        r = bundle.build(self.t / "o3", None, {"datasets": {}}, [])
        with tarfile.open(r["path"]) as tf:
            names = tf.getnames()
        self.assertIn("kfml/vendor/nanogpt/NOTICE", names)
        self.assertFalse([n for n in names if "__pycache__" in n])


if __name__ == "__main__":
    unittest.main()
