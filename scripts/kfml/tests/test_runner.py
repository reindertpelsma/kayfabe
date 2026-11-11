import json
import os
import tempfile
import unittest

from tests.helpers import fake_env
from kfml import env as envmod, executor, pins, runner
from kfml.spec import Spec, Upstream

UP = Upstream("up", "https://e/up", "b" * 40, "MIT", "fetched")


def spec(name="t", **kw):
    return Spec(name, "fake.mod", "d", "mid", 100, **kw)


class Fake:
    def __init__(self, result):
        self.result, self.calls = result, []

    def __call__(self, ctx, budget, env, log, python=None):
        self.calls.append((ctx, budget, env))
        return dict(self.result, log=log)


class Runner(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def go(self, specs, env=None, ex=None, environ=None, **kw):
        ex = ex or Fake({"status": "PASS", "metrics": {"m": 1}})
        rows = runner.run_all(specs, env or fake_env(), environ=environ or {}, root=self.root, exec_fn=ex,
                              resolve_fn=kw.pop("resolve_fn", lambda up, tmp: None), **kw)
        return rows, ex

    def test_pass_row(self):
        rows, ex = self.go([spec()])
        self.assertEqual(rows[0]["status"], "PASS")
        self.assertEqual(rows[0]["gpu_name"], "Fake RTX")
        self.assertEqual(rows[0]["lock_sha256"], "ab" * 32)
        self.assertEqual(len(ex.calls), 1)

    def test_not_run_never_executes(self):
        rows, ex = self.go([spec(min_cc=(9, 0))])
        self.assertEqual(rows[0]["status"], "NOT_RUN")
        self.assertIn("compute capability", rows[0]["reason"])
        self.assertEqual(ex.calls, [])

    def test_env_missing_is_fail(self):
        rows, ex = self.go([spec(needs_nvcc=True)], env=fake_env(nvcc=None))
        self.assertEqual(rows[0]["status"], "FAIL")
        self.assertTrue(rows[0]["reason"].startswith("env-missing"))
        self.assertEqual(ex.calls, [])

    def test_child_env_inside_root(self):
        _, ex = self.go([spec()])
        env = ex.calls[0][2]
        self.assertEqual(envmod.assert_env_inside(env, self.root), [])
        for k in ("HF_HOME", "TORCH_HOME", "TRITON_CACHE_DIR", "TORCHINDUCTOR_CACHE_DIR", "XDG_CACHE_HOME",
                  "PIP_CACHE_DIR", "UV_CACHE_DIR", "TMPDIR", "HF_HUB_CACHE", "HOME", "KF_ML_DATA"):
            self.assertTrue(env[k].startswith(self.root), k)

    def test_inherited_outside_cache_var_is_overridden(self):
        _, ex = self.go([spec()], environ={"HF_HOME": "/home/u/.cache/hf", "TMPDIR": "/tmp"})
        self.assertEqual(envmod.assert_env_inside(ex.calls[0][2], self.root), [])

    def test_outside_detector_flags_bad(self):
        bad = envmod.assert_env_inside({"HF_HOME": "/elsewhere"}, self.root)
        self.assertEqual(bad, [("HF_HOME", "/elsewhere")])
        self.assertEqual(envmod.assert_env_inside({"TMPDIR": "/tmp"}, self.root, allowed_ro=("/tmp",)), [])

    def test_refuses_without_venv(self):
        with self.assertRaises(runner.RunnerError):
            runner.run_all([spec()], fake_env(), environ={}, root=None, exec_fn=Fake({}))

    def test_pin_refused(self):
        def bad(up, tmp):
            raise pins.PinError("HEAD x != pin y")
        rows, ex = self.go([spec(upstreams=(UP,))], resolve_fn=bad)
        self.assertEqual(rows[0]["status"], "FAIL")
        self.assertIn("pin refused", rows[0]["reason"])
        self.assertEqual(ex.calls, [])

    def test_pins_recorded(self):
        res = pins.Resolved("/p", UP.commit, "c" * 64)
        rows, ex = self.go([spec(upstreams=(UP,))], resolve_fn=lambda u, t: res)
        self.assertEqual(rows[0]["upstream_commits"], {"up": UP.commit})
        self.assertEqual(rows[0]["patch_sha256"], {"up": "c" * 64})
        self.assertEqual(ex.calls[0][0]["upstream_paths"], {"up": "/p"})

    def test_dataset_sha_and_reference_roundtrip(self):
        rec = os.path.join(self.root, "ref")
        ex = Fake({"status": "PASS", "metrics": {}, "dataset_sha256": {"d": "f" * 64}, "reference": {"x": 1}})
        rows, _ = self.go([spec()], ex=ex, record_dir=rec)
        self.assertEqual(rows[0]["dataset_sha256"], {"d": "f" * 64})
        self.assertTrue(rows[0]["metrics"]["reference_recorded"])
        with open(os.path.join(rec, "t.json")) as f:
            self.assertEqual(json.load(f)["data"], {"x": 1})
        ex2 = Fake({"status": "PASS", "metrics": {}})
        self.go([spec()], ex=ex2, reference_dir=rec)
        self.assertEqual(ex2.calls[0][0]["reference"], {"x": 1})

    def test_select(self):
        a, b = spec("a"), spec("b", gate=False)
        self.assertEqual([s.name for s in runner.select([a, b])], ["a"])
        self.assertEqual([s.name for s in runner.select([a, b], run_all=True)], ["a", "b"])
        self.assertEqual([s.name for s in runner.select([a, b], only=["b"])], ["b"])
        with self.assertRaises(runner.RunnerError):
            runner.select([a], only=["zzz"])

    def test_budget_scale(self):
        _, ex = self.go([spec()], budget_scale=3)
        self.assertEqual(ex.calls[0][1], 300)


class Executor(unittest.TestCase):
    """Real subprocesses against fake workload modules (no torch)."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = self.tmp.name
        os.makedirs(os.path.join(self.t, "mods"))

    def tearDown(self):
        self.tmp.cleanup()

    def run_mod(self, body, budget=20):
        with open(os.path.join(self.t, "mods", "fakewl.py"), "w") as f:
            f.write("import os, sys, time\nfrom kfml.errors import *\n" + body)
        ctx = {"name": "x", "module": "fakewl", "data_dir": self.t, "budget_s": budget}
        env = dict(os.environ, TMPDIR=self.t)
        return executor.run_child(ctx, budget, env, os.path.join(self.t, "log"), extra_path=[os.path.join(self.t, "mods")])

    def test_pass(self):
        r = self.run_mod("def run(ctx):\n    return {'metrics': {'k': 2}}\n")
        self.assertEqual((r["status"], r["metrics"]), ("PASS", {"k": 2}))

    def test_fail_assert(self):
        r = self.run_mod("def run(ctx):\n    raise CheckFailed('bad')\n")
        self.assertEqual(r["status"], "FAIL")
        self.assertIn("bad", r["reason"])

    def test_exception(self):
        r = self.run_mod("def run(ctx):\n    1/0\n")
        self.assertEqual(r["status"], "FAIL")
        self.assertIn("ZeroDivisionError", r["reason"])

    def test_notrun_and_envmissing(self):
        r = self.run_mod("def run(ctx):\n    raise NotRun('why')\n")
        self.assertEqual((r["status"], r["reason"]), ("NOT_RUN", "why"))
        r = self.run_mod("def run(ctx):\n    raise EnvMissing('nvcc')\n")
        self.assertEqual((r["status"], r["reason"]), ("FAIL", "env-missing: nvcc"))

    def test_timeout_kills(self):
        r = self.run_mod("def run(ctx):\n    time.sleep(60)\n", budget=1.5)
        self.assertEqual(r["status"], "FAIL")
        self.assertIn("timeout", r["reason"])

    def test_hard_crash(self):
        r = self.run_mod("def run(ctx):\n    os._exit(3)\n")
        self.assertEqual(r["status"], "FAIL")
        self.assertIn("exit code 3", r["reason"])

    def test_signal(self):
        r = self.run_mod("import signal\ndef run(ctx):\n    os.kill(os.getpid(), signal.SIGSEGV)\n")
        self.assertIn("signal", r["reason"])


if __name__ == "__main__":
    unittest.main()
