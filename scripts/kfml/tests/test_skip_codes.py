"""The reason codes of a NOT_RUN row: the one fixed vocabulary the payload grades (kfml/errors.py SKIP_CODES).

FALSIFIER (written before the code): a NOT_RUN row carries free text where a code belongs, a row that is not a hardware or host limit carries a
code, an unknown code is accepted by NotRun or by make_row, managed_oversub's host-RAM refusal has no code, or a code reaches a PASS or FAIL row.
"""
import os
import sys
import tempfile
import unittest

from tests.helpers import fake_env
from kfml import errors, executor, gating, rows, runner
from kfml.spec import Spec
from kfml.workloads import managed_oversub as mo


def spec(**kw):
    return Spec("t", "fake.mod", "d", "mid", 100, **kw)


def not_run_exec(*_a, **_k):
    raise AssertionError("a gated row never executes")


class Vocabulary(unittest.TestCase):
    def test_the_vocabulary_is_exactly_these_four_codes(self):
        self.assertEqual(errors.SKIP_CODES, ("HOST_RAM_TOO_SMALL", "VRAM_TOO_SMALL", "DIE_LACKS_FEATURE", "NOT_RUNNABLE_ON_BARE_METAL"))

    def test_not_run_takes_an_optional_code_from_the_vocabulary(self):
        self.assertEqual(errors.NotRun("why").code, "")
        for code in errors.SKIP_CODES:
            self.assertEqual(errors.NotRun("why", code=code).code, code)
            self.assertEqual(str(errors.NotRun("why", code=code)), "why")

    def test_not_run_refuses_a_code_outside_the_vocabulary(self):
        for bad in ("host ram too small", "host_ram_too_small", "OTHER", " HOST_RAM_TOO_SMALL", None, 5, ["HOST_RAM_TOO_SMALL"]):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    errors.NotRun("why", code=bad)

    def test_make_row_carries_a_code_only_from_the_vocabulary(self):
        env = fake_env()
        r = rows.make_row("t", "NOT_RUN", "why", 1.0, env, skip_code="VRAM_TOO_SMALL")
        self.assertEqual(r["skip_code"], "VRAM_TOO_SMALL")
        self.assertNotIn("skip_code", rows.make_row("t", "NOT_RUN", "why", 1.0, env))
        with self.assertRaises(ValueError):
            rows.make_row("t", "NOT_RUN", "why", 1.0, env, skip_code="free text")
        for status in ("PASS", "FAIL"):
            with self.assertRaises(ValueError):
                rows.make_row("t", status, "why", 1.0, env, skip_code="VRAM_TOO_SMALL")


class Gating(unittest.TestCase):
    def verdict(self, s, **envkw):
        return gating.evaluate(s, fake_env(**envkw), {})

    def test_a_card_below_the_compute_capability_floor_is_die_lacks_feature(self):
        v = self.verdict(spec(min_cc=(8, 0)), cc=(7, 5))
        self.assertEqual((v.kind, v.code), ("not_run", "DIE_LACKS_FEATURE"))

    def test_too_little_vram_is_vram_too_small(self):
        v = self.verdict(spec(min_vram_gb=16), vram_total_gb=11.0)
        self.assertEqual((v.kind, v.code), ("not_run", "VRAM_TOO_SMALL"))

    def test_every_other_not_run_has_no_code(self):
        cases = (
            self.verdict(spec(max_cc=(8, 9)), cc=(12, 0)),
            self.verdict(spec(), cuda_available=False, cuda_error="The NVIDIA driver on your system is too old (found version 12040)"),
            self.verdict(spec(), torch=None, cuda_available=False, flavor_blocked=True, flavor_reason="no pinned flavour"),
            gating.evaluate(mo.SPEC, fake_env(), {"KF_ML_GUEST": "1"}),
        )
        for v in cases:
            with self.subTest(reason=v.reason):
                self.assertEqual(v.kind, "not_run")
                self.assertEqual(v.code, "")

    def test_an_ok_or_env_missing_verdict_has_no_code(self):
        self.assertEqual(self.verdict(spec()).code, "")
        self.assertEqual(self.verdict(spec(), torch=None, cuda_available=False).code, "")


class RunnerRows(unittest.TestCase):
    def go(self, specs, env=None, ex=not_run_exec):
        with tempfile.TemporaryDirectory() as root:
            return runner.run_all(specs, env or fake_env(), environ={}, root=root, exec_fn=ex, resolve_fn=lambda up, tmp: None)

    def test_a_gated_row_carries_its_code(self):
        r = self.go([spec(min_vram_gb=16)], env=fake_env(vram_total_gb=11.0))[0]
        self.assertEqual((r["status"], r["skip_code"]), ("NOT_RUN", "VRAM_TOO_SMALL"))
        r = self.go([spec(min_cc=(9, 0))])[0]
        self.assertEqual((r["status"], r["skip_code"]), ("NOT_RUN", "DIE_LACKS_FEATURE"))

    def test_a_gated_row_without_a_code_has_no_code_key(self):
        r = self.go([spec(max_cc=(8, 0))])[0]
        self.assertEqual(r["status"], "NOT_RUN")
        self.assertNotIn("skip_code", r)

    def test_the_code_of_a_child_result_reaches_the_row(self):
        ex = lambda ctx, budget, env, log, python=None: {"status": "NOT_RUN", "reason": "x", "skip_code": "HOST_RAM_TOO_SMALL", "metrics": {}}
        r = self.go([spec()], ex=ex)[0]
        self.assertEqual((r["status"], r["skip_code"]), ("NOT_RUN", "HOST_RAM_TOO_SMALL"))

    def test_a_code_on_a_pass_or_fail_child_result_is_not_copied(self):
        for status in ("PASS", "FAIL"):
            ex = lambda ctx, budget, env, log, python=None, s=status: {"status": s, "reason": "x", "skip_code": "HOST_RAM_TOO_SMALL", "metrics": {}}
            r = self.go([spec()], ex=ex)[0]
            self.assertNotIn("skip_code", r)


class Child(unittest.TestCase):
    """Real subprocess: the code a workload raises with is written into the child's result."""

    def run_mod(self, body):
        with tempfile.TemporaryDirectory() as t:
            os.makedirs(os.path.join(t, "mods"))
            with open(os.path.join(t, "mods", "fakewl.py"), "w") as f:
                f.write("from kfml.errors import *\n" + body)
            ctx = {"name": "x", "module": "fakewl", "data_dir": t, "budget_s": 20}
            return executor.run_child(ctx, 20, dict(os.environ, TMPDIR=t), os.path.join(t, "log"), extra_path=[os.path.join(t, "mods")])

    def test_a_not_run_with_a_code(self):
        r = self.run_mod("def run(ctx):\n    raise NotRun('why', code='HOST_RAM_TOO_SMALL')\n")
        self.assertEqual((r["status"], r["reason"], r["skip_code"]), ("NOT_RUN", "why", "HOST_RAM_TOO_SMALL"))

    def test_a_not_run_without_a_code_has_none(self):
        r = self.run_mod("def run(ctx):\n    raise NotRun('why')\n")
        self.assertEqual(r["status"], "NOT_RUN")
        self.assertEqual(r.get("skip_code", ""), "")

    def test_an_unknown_code_is_a_fail_never_a_not_run(self):
        r = self.run_mod("def run(ctx):\n    raise NotRun('why', code='whatever')\n")
        self.assertEqual(r["status"], "FAIL")
        self.assertNotIn("skip_code", r)


class ManagedOversub(unittest.TestCase):
    GIB = 2**30

    def test_the_host_ram_refusal_carries_host_ram_too_small(self):
        with self.assertRaises(errors.NotRun) as caught:
            mo.require_host_ram(24 * self.GIB, 47.6)   # 30 GiB wanted, 60% of 47.6 GiB is 28.6
        self.assertEqual(caught.exception.code, "HOST_RAM_TOO_SMALL")
        self.assertIn("host RAM too small", str(caught.exception))

    def test_enough_host_ram_does_not_refuse(self):
        self.assertEqual(mo.require_host_ram(24 * self.GIB, 64.0), 30 * self.GIB)
        mo.require_host_ram(24 * self.GIB, 50.0)       # 30 GiB wanted, 30.0 allowed: the boundary fits

    def test_run_uses_the_checked_helper_after_the_memory_is_known(self):
        import inspect
        self.assertIn("want = require_host_ram(total)", inspect.getsource(mo.run))

    def test_the_guest_precheck_reason_has_no_code(self):
        self.assertEqual(mo._precheck(None, {"KF_ML_GUEST": "1"}), mo.REASON)


if __name__ == "__main__":
    unittest.main()
