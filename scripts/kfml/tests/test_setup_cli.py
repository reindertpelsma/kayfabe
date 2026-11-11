import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

from tests.helpers import fake_env
from kfml import cli, setup_venv, registry


class FakeRun:
    def __init__(self, rc=0):
        self.calls, self.rc = [], rc

    def __call__(self, cmd, **kw):
        self.calls.append((cmd, kw))
        if cmd[1:3] == ["-m", "venv"]:
            os.makedirs(cmd[3], exist_ok=True)
            Path(cmd[3], "pyvenv.cfg").write_text("x")

        class P:
            pass
        p = P()
        p.returncode, p.stderr, p.stdout = self.rc, "", ""
        return p


class Setup(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_refuses_unhashed_lock(self):
        with self.assertRaises(setup_venv.SetupError) as cm:
            setup_venv.setup(self.t / "v", run=FakeRun(), allow_unhashed=False)
        self.assertIn("HASHES TODO", str(cm.exception))
        self.assertFalse((self.t / "v").exists())

    def test_refuses_foreign_dir(self):
        d = self.t / "home"
        d.mkdir()
        (d / "precious.txt").write_text("x")
        with self.assertRaises(setup_venv.SetupError):
            setup_venv.setup(d, run=FakeRun(), allow_unhashed=True)
        self.assertTrue((d / "precious.txt").exists())

    def test_unhashed_dev_install_touches_only_venv(self):
        fr = FakeRun()
        r = setup_venv.setup(self.t / "v", flavor="cu126", run=fr, allow_unhashed=True)
        self.assertFalse(r["hashed"])
        env = fr.calls[-1][1]["env"]
        from kfml import env as envmod
        self.assertEqual(envmod.assert_env_inside(env, self.t / "v"), [])
        self.assertTrue((self.t / "v" / "kfml-lock.sha256").read_text().startswith(r["lock_sha256"]))
        cmd = fr.calls[-1][0]
        self.assertNotIn("--require-hashes", cmd)
        self.assertIn("requirements.cu126.lock.txt", cmd[-1])

    def test_hashed_lock_uses_require_hashes(self):
        lock = self.t / "l.txt"
        lock.write_text("torch==1 \\\n    --hash=sha256:" + "0" * 64 + "\n")
        self.assertTrue(setup_venv.has_hashes(lock.read_text()))
        c = setup_venv.install_cmd(self.t / "v", lock, True, "pip")
        self.assertIn("--require-hashes", c)
        self.assertIn("--require-hashes", setup_venv.install_cmd(self.t / "v", lock, True, "uv"))

    def test_lock_files(self):
        for fl, fn in setup_venv.FLAVORS.items():
            text = (Path(setup_venv.ROOT) / fn).read_text()
            self.assertIn("download.pytorch.org/whl/" + fl, text)
            self.assertIn("torch==2.7.1+" + fl, text)
            self.assertIn("HASHES TODO", text)
        with self.assertRaises(setup_venv.SetupError):
            setup_venv.lock_path("cu999")

    def test_nvcc_install_verifies_sha(self):
        import hashlib
        import tarfile
        arc_src = self.t / "a.tar.xz"
        inner = self.t / "pkg" / "bin"
        inner.mkdir(parents=True)
        (inner / "nvcc").write_text("#!/bin/sh\n")
        with tarfile.open(arc_src, "w:xz") as tf:
            tf.add(self.t / "pkg", arcname="cuda_x-archive")
        sha = hashlib.sha256(arc_src.read_bytes()).hexdigest()
        pins = {"cuda_redist": {"cu128": {"base_url": "https://e/", "components": {"cuda_x": {"path": "x/a.tar.xz", "sha256": sha}}}}}

        def fetch(url, dest):
            Path(dest).write_bytes(arc_src.read_bytes())
        (self.t / "v").mkdir()
        home = setup_venv.install_nvcc(self.t / "v", "cu128", pins, fetch)
        self.assertTrue((home / "bin" / "nvcc").exists())
        pins["cuda_redist"]["cu128"]["components"]["cuda_x"]["sha256"] = "0" * 64
        (self.t / "v" / "kfml-cache" / "tmp" / "nvcc-stage" / "a.tar.xz").unlink()
        with self.assertRaises(setup_venv.SetupError):
            setup_venv.install_nvcc(self.t / "v", "cu128", pins, fetch)


class Cli(unittest.TestCase):
    def test_list_and_tiers(self):
        out = io.StringIO()
        self.assertEqual(cli.main(["list"], out), 0)
        self.assertIn("nanogpt_compile", out.getvalue())
        out = io.StringIO()
        cli.main(["list", "--tier", "large", "--json"], out)
        d = json.loads(out.getvalue())
        self.assertEqual([t["name"] for t in d["tests"]], ["managed_oversub"])
        self.assertIn("estimate_total_s", d["summary"])

    def test_check_does_not_install_and_reports(self):
        out = io.StringIO()
        called = []
        import subprocess
        real = subprocess.run

        def guard(cmd, *a, **k):
            called.append(cmd)
            return real(cmd, *a, **k)
        rc = cli._check(None, out, probe=lambda: fake_env(torch=None, cuda_available=False, nvcc=None))
        text = out.getvalue()
        self.assertEqual(rc, 1)
        self.assertIn("MISSING", text)
        self.assertIn("kfml setup --venv DIR", text)
        self.assertIn("--with-nvcc", text)
        self.assertIn("test nanogpt_compile", text)

    def test_run_unknown_test_is_usage_error(self):
        self.assertEqual(cli.main(["run", "--only", "nope"], io.StringIO()), 2)

    def test_workloads_compile_and_import_torch_free(self):
        import importlib
        import ast
        here = Path(__file__).resolve().parent.parent / "kfml" / "workloads"
        for p in sorted(here.glob("*.py")):
            ast.parse(p.read_text(), str(p))
        sys.modules.pop("torch", None)
        blocked = {}

        class Block:
            def find_spec(self, name, path=None, target=None):
                if name.split(".")[0] in ("torch", "triton"):
                    raise ImportError("blocked: " + name)
        sys.meta_path.insert(0, Block())
        try:
            for n in registry.NAMES:
                m = importlib.import_module("kfml.workloads." + n)
                importlib.reload(m)
        finally:
            sys.meta_path.pop(0)


if __name__ == "__main__":
    unittest.main()
