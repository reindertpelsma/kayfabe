import unittest

from tests.helpers import fake_env
from kfml import gating
from kfml.spec import Spec


def spec(**kw):
    return Spec("t", "m", "d", "mid", 100, **kw)


class Gating(unittest.TestCase):
    def kind(self, s, **envkw):
        return gating.evaluate(s, fake_env(**envkw), {})

    def test_ok(self):
        self.assertEqual(self.kind(spec()).kind, "ok")

    def test_no_gpu_is_fail_not_notrun(self):
        v = self.kind(spec(), gpu_visible=False, cuda_available=False)
        self.assertEqual(v.kind, "env_missing")

    def test_no_torch(self):
        self.assertEqual(self.kind(spec(), torch=None, cuda_available=False).kind, "env_missing")

    def test_driver_too_old_is_not_run(self):
        v = self.kind(spec(), cuda_available=False,
                      cuda_error="The NVIDIA driver on your system is too old (found version 12040)")
        self.assertEqual(v.kind, "not_run")
        self.assertIn("driver too old for this CUDA wheel (needs >= 570.26)", v.reason)

    def test_other_cuda_init_error_is_fail(self):
        v = self.kind(spec(), cuda_available=False, cuda_error="CUDA error: out of memory")
        self.assertEqual(v.kind, "env_missing")

    def test_cc_bounds(self):
        v = self.kind(spec(min_cc=(8, 0)), cc=(7, 5))
        self.assertEqual((v.kind, "min 8.0" in v.reason), ("not_run", True))
        v = self.kind(spec(max_cc=(8, 9)), cc=(12, 0))
        self.assertEqual((v.kind, "max 8.9" in v.reason), ("not_run", True))

    def test_vram(self):
        v = self.kind(spec(min_vram_gb=16), vram_total_gb=11.0)
        self.assertEqual(v.kind, "not_run")
        self.assertIn("vram", v.reason)

    def test_old_torch_is_env_missing(self):
        self.assertEqual(self.kind(spec(), torch="2.5.0").kind, "env_missing")

    def test_nvcc(self):
        s = spec(needs_nvcc=True)
        self.assertEqual(self.kind(s, nvcc=None).kind, "env_missing")
        v = self.kind(s, nvcc="12.6")
        self.assertEqual(v.kind, "env_missing")
        self.assertIn("major.minor", v.reason)
        self.assertEqual(gating.evaluate(s, fake_env(nvcc="12.6"), {"KF_ML_NVCC_LOOSE": "1"}).kind, "ok")
        self.assertEqual(self.kind(s, nvcc="12.8").kind, "ok")

    def test_triton_and_compiler(self):
        self.assertEqual(self.kind(spec(needs_triton=True), triton=None).kind, "env_missing")
        self.assertEqual(self.kind(spec(needs_compiler=True), python_h=False).kind, "env_missing")

    def test_precheck_guest(self):
        from kfml.workloads import managed_oversub as mo
        e = fake_env()
        self.assertEqual(gating.evaluate(mo.SPEC, e, {}).kind, "ok")
        v = gating.evaluate(mo.SPEC, e, {"KF_ML_GUEST": "1"})
        self.assertEqual((v.kind, v.reason), ("not_run", "managed-memory paging not supported"))
        v = gating.evaluate(mo.SPEC, e, {"KF_ML_GUEST": "1", "KF_ML_PROBE_TRY": "1"})
        self.assertEqual(v.kind, "ok")


if __name__ == "__main__":
    unittest.main()
