import hashlib
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from tests import helpers  # noqa: F401
from kfml import datasets, pins, registry
from kfml.spec import Upstream

ROOT = Path(__file__).resolve().parent.parent


class Datasets(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = Path(self.tmp.name)
        self.src = self.t / "src.bin"
        self.src.write_bytes(b"hello kfml")
        self.sha = hashlib.sha256(b"hello kfml").hexdigest()

    def tearDown(self):
        self.tmp.cleanup()

    def pins(self, sha):
        return {"datasets": {"d": {"url": self.src.as_uri(), "filename": "d.bin", "sha256": sha}}}

    def test_pinned_ok(self):
        r = datasets.fetch("d", self.t / "data", self.pins(self.sha), environ={})
        self.assertTrue(r["pinned"])
        self.assertEqual(r["sha256"], self.sha)
        self.assertTrue(datasets.fetch("d", self.t / "data", self.pins(self.sha), environ={})["pinned"])  # cached

    def test_wrong_hash_refused_and_deleted(self):
        with self.assertRaises(datasets.HashMismatch):
            datasets.fetch("d", self.t / "data", self.pins("0" * 64), environ={})
        self.assertFalse((self.t / "data" / "d.bin").exists())

    def test_pin_me(self):
        with self.assertRaises(datasets.PinMe) as cm:
            datasets.fetch("d", self.t / "data", self.pins(None), environ={})
        self.assertIn("PIN ME", str(cm.exception))
        self.assertIn(self.sha, str(cm.exception))
        with self.assertRaises(datasets.PinMe):  # second call does not redownload and still refuses
            datasets.fetch("d", self.t / "data", self.pins(None), environ={})
        r = datasets.fetch("d", self.t / "data", self.pins(None), environ={"KF_ML_ALLOW_UNPINNED": "1"})
        self.assertFalse(r["pinned"])

    def test_repo_pins_file(self):
        p = datasets.load_pins()
        self.assertEqual(len(p["datasets"]["tinyshakespeare"]["sha256"]), 64)
        for name, d in p["datasets"].items():
            self.assertTrue(d["url"].startswith("https://"), name)
        for fl, red in p["cuda_redist"].items():
            for c in red["components"].values():
                self.assertEqual(len(c["sha256"]), 64)


def git(cwd, *a):
    subprocess.run(["git", *a], cwd=cwd, check=True, capture_output=True,
                   env=dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@e", GIT_COMMITTER_NAME="t",
                            GIT_COMMITTER_EMAIL="t@e"))
    return subprocess.run(["git", "rev-parse", "HEAD"], cwd=cwd, capture_output=True, text=True).stdout.strip()


class Pins(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = Path(self.tmp.name)
        self.repo = t / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        (self.repo / "a.txt").write_text("one\n")
        git(self.repo, "add", "-A")
        self.sha = git(self.repo, "commit", "-qm", "c1")
        self.work = t / "work"
        self.work.mkdir()
        self.root = t / "root"
        (self.root / "patches").mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def up(self, commit=None, patch=None):
        return Upstream("r", self.repo.as_uri(), commit or self.sha, "MIT", "fetched", patch=patch)

    def test_clone_ok_with_patch(self):
        patch = self.root / "patches" / "p.patch"
        patch.write_text("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1,2 @@\n one\n+two\n")
        r = pins.clone_at_pin(self.up(patch="patches/p.patch"), self.work, root=self.root)
        self.assertEqual(r.commit, self.sha)
        self.assertEqual((Path(r.path) / "a.txt").read_text(), "one\ntwo\n")
        self.assertEqual(r.patch_sha256, hashlib.sha256(patch.read_bytes()).hexdigest())

    def test_wrong_sha_refused(self):
        with self.assertRaises(pins.PinError) as cm:
            pins.clone_at_pin(self.up(commit="1" * 40), self.work, root=self.root)
        self.assertIn("1" * 40, str(cm.exception))

    def test_branch_name_refused_at_declaration(self):
        with self.assertRaises(ValueError):
            Upstream("r", "u", "master", "MIT", "fetched")

    def test_head_mismatch_refused(self):
        def fake_run(cmd, **kw):
            class P:
                returncode, stderr = 0, ""
                stdout = ("f" * 40) if cmd[1:3] == ["rev-parse", "HEAD"] else ""
            return P()
        with self.assertRaises(pins.PinError) as cm:
            pins.clone_at_pin(self.up(), self.work, root=self.root, run=fake_run)
        self.assertIn("!= pin", str(cm.exception))

    def test_patch_failure_refused(self):
        (self.root / "patches" / "bad.patch").write_text("--- a/zzz\n+++ b/zzz\n@@ -1 +1 @@\n-x\n+y\n")
        with self.assertRaises(pins.PinError):
            pins.clone_at_pin(self.up(patch="patches/bad.patch"), self.work, root=self.root)

    def test_vendored_notice_present_for_every_vendor_dir(self):
        vd = ROOT / "vendor"
        dirs = [d for d in vd.iterdir() if d.is_dir()]
        self.assertTrue(dirs)
        for d in dirs:
            notice = (d / "NOTICE").read_text()
            self.assertIn("https://github.com/", notice, d.name)
            self.assertRegex(notice, r"[0-9a-f]{40}", d.name)
            self.assertTrue((d / "LICENSE").is_file(), d.name)

    def test_declared_vendored_upstreams_match_notice(self):
        found = 0
        for s in registry.all_specs():
            for u in s.upstreams:
                if u.vendored:
                    pins.check_vendored(u)  # NOTICE names url + commit, LICENSE present
                    found += 1
        self.assertGreaterEqual(found, 1)

    def test_vendored_mismatch_refused(self):
        u = Upstream("nanoGPT", "https://github.com/karpathy/nanoGPT", "0" * 40, "MIT", "vendored", vendor_dir="nanogpt")
        with self.assertRaises(pins.PinError):
            pins.check_vendored(u)

    def test_vendored_model_is_verbatim_upstream_size(self):
        # no local tweak: a tweak would have to be documented in NOTICE and show in the diff
        self.assertIn("verbatim", (ROOT / "vendor" / "nanogpt" / "NOTICE").read_text())


if __name__ == "__main__":
    unittest.main()
