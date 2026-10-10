# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import contextlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import unittest

import helpers as H
import appdisk


class Manifest(unittest.TestCase):
    def test_committed_manifest_is_valid_and_fully_pinned(self):
        man = H.load_manifest()
        self.assertEqual(appdisk.check_manifest(man, H.load_apps()), [])
        for p in man["packages"]:
            for f in p["files"]:
                self.assertRegex(f["sha256"] or "", r"^[0-9a-f]{64}$", f"{p['id']}/{f['name']} is not pinned")
                self.assertGreater(f["size"], 0)
                self.assertEqual(f.get("http"), 200, f"{p['id']}/{f['name']}: url was not verified")

    def test_every_package_has_licence_and_redistribution_note(self):
        for p in H.load_manifest()["packages"]:
            self.assertTrue(p["license"] and p["redistribution"], p["id"])

    def test_no_secrets_or_binaries_in_the_manifest_dir(self):
        banned = (".exe", ".dll", ".zip", ".whl", ".iso", ".gguf", ".7z", ".msi", ".pem", ".key")
        for root, _, files in os.walk(H.AM):
            for f in files:
                self.assertFalse(f.lower().endswith(banned), f"binary-looking file committed: {os.path.join(root, f)}")

    def test_check_manifest_catches_problems(self):
        bad = {"schema": 1, "packages": [
            {"id": "a", "title": "t", "version": "1", "license": "l", "redistribution": "r", "stage": {"mode": "bogus"}, "files": [{"name": "x/y", "url": "http://x", "sha256": "zz"}], "needs": ["nope"]},
            {"id": "a", "title": "t", "version": "1", "license": "l", "redistribution": "r", "stage": {"mode": "installer"}, "files": []}], "dropped": [{"id": "d"}]}
        probs = "\n".join(appdisk.check_manifest(bad))
        for w in ("duplicate package ids", "unknown stage mode", "bad file name", "must be https", "not 64 hex", "needs unknown package", "installer without a run command", "no files", "dropped d: no reason"):
            self.assertIn(w, probs)

    def test_closure_orders_dependencies_first(self):
        man = H.load_manifest()
        self.assertEqual(appdisk.closure(man, ["hashcat"]), ["sevenzip", "hashcat"])
        c = appdisk.closure(man, ["pywheels"])
        self.assertEqual(c, ["python_embed", "pywheels"])

    def test_layout_and_stage_table(self):
        man = H.load_manifest()
        lay = dict(appdisk.plan_layout(man, None, 9, None))
        self.assertIn("py/", lay)                                     # the python tree is extracted into the image
        self.assertNotIn("pkg/pywheels/torch-2.8.0+cu126-cp312-cp312-win_amd64.whl", lay)
        self.assertIn("pkg/ffmpeg/ffmpeg-n8.1.3-14-g330caae0c1-win64-gpl-8.1.zip", lay)
        self.assertIn("guest/kf_run_app.ps1", lay)
        t = appdisk.stage_table(man)
        self.assertEqual(t["pywheels"]["mode"], "tree")
        self.assertEqual(t["heaven"]["mode"], "installer")
        self.assertTrue(t["ffmpeg"]["strip_top"])
        self.assertEqual(t["hashcat"]["dest"], "hashcat")


@unittest.skipUnless(shutil.which("curl") and (shutil.which("xorriso") or shutil.which("genisoimage") or shutil.which("mkisofs")), "needs curl and an ISO tool")
class BuildEndToEnd(unittest.TestCase):
    """fetch (file:// URLs) -> hash pinning -> tree packages extracted -> ISO -> read back, on a tiny fake manifest."""

    def test_fetch_build_verify(self):
        with contextlib.redirect_stdout(io.StringIO()):
            self._fetch_build_verify()

    def _fetch_build_verify(self):
        import zipfile
        with tempfile.TemporaryDirectory() as d:
            src = os.path.join(d, "src"); os.makedirs(src)
            embed = os.path.join(src, "pyembed.zip")
            with zipfile.ZipFile(embed, "w") as z:
                z.writestr("python312._pth", "python312.zip\n.\n"); z.writestr("python.exe", "MZ")
            wheel = os.path.join(src, "w-1-py3-none-any.whl")
            with zipfile.ZipFile(wheel, "w") as z:
                z.writestr("pkg/__init__.py", "x=1")
            tool = os.path.join(src, "tool.zip")
            with zipfile.ZipFile(tool, "w") as z:
                z.writestr("top/tool.exe", "MZ")
            man = {"schema": 1, "image": {"file": "t.iso", "label": "KFAPPS"}, "packages": [
                {"id": "python_embed", "title": "p", "version": "1", "license": "l", "redistribution": "r", "image_form": "tree", "tree_dest": "py",
                 "stage": {"mode": "python_embed", "dest": "py"}, "files": [{"name": "pyembed.zip", "url": "file://" + embed, "sha256": None}]},
                {"id": "pywheels", "title": "w", "version": "1", "license": "l", "redistribution": "r", "image_form": "tree", "tree_dest": "py/Lib/site-packages", "needs": ["python_embed"],
                 "stage": {"mode": "wheels", "dest": "py"}, "files": [{"name": "w-1-py3-none-any.whl", "url": "file://" + wheel, "sha256": None}]},
                {"id": "tool", "title": "t", "version": "1", "license": "l", "redistribution": "r", "stage": {"mode": "unzip", "dest": "tool", "strip_top": True},
                 "files": [{"name": "tool.zip", "url": "file://" + tool, "sha256": None}]}], "dropped": []}
            mp = os.path.join(d, "m.json"); json.dump(man, open(mp, "w"))
            self.assertEqual(appdisk.main(["fetch", "--manifest", mp, "--dl", os.path.join(d, "dl")]), 0)
            m2 = json.load(open(mp))
            for p in m2["packages"]:
                for f in p["files"]:
                    self.assertRegex(f["sha256"], r"^[0-9a-f]{64}$")
            self.assertEqual(appdisk.main(["verify", "--manifest", mp, "--dl", os.path.join(d, "dl")]), 0)
            tools = os.path.join(d, "tools"); os.makedirs(tools); open(os.path.join(tools, "kf_x.exe"), "wb").write(b"MZ")
            out = os.path.join(d, "out")
            self.assertEqual(appdisk.main(["build", "--manifest", mp, "--dl", os.path.join(d, "dl"), "--tools", tools, "--out", out]), 0)
            built = json.load(open(os.path.join(out, "manifest.json")))
            self.assertRegex(built["image"]["sha256"], r"^[0-9a-f]{64}$")
            self.assertIn("pkg/tool/tool.zip", built["files"])
            self.assertIn("tools/kf_x.exe", built["files"])
            self.assertIn("guest/kf_run_app.ps1", built["files"])
            iso = os.path.join(out, "t.iso")
            listing = subprocess.run(["bash", "-c", f"7z l '{iso}' 2>/dev/null || isoinfo -R -l -i '{iso}'"], capture_output=True, text=True).stdout
            for w in ("python312._pth", "python.exe", "tool.zip", "kf_x.exe", "KFAPPS.ID", "__init__.py"):
                self.assertIn(w, listing, listing[-600:])
            # a tampered download is refused at build time
            open(os.path.join(d, "dl", "tool", "tool.zip"), "ab").write(b"x")
            self.assertEqual(appdisk.main(["build", "--manifest", mp, "--dl", os.path.join(d, "dl"), "--tools", tools, "--out", os.path.join(d, "out2")]), 1)
            self.assertEqual(appdisk.main(["verify", "--manifest", mp, "--dl", os.path.join(d, "dl")]), 1)


if __name__ == "__main__":
    unittest.main()
