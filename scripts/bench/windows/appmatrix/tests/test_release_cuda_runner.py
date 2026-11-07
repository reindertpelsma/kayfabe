# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Exercise the actual runner with a fake PyTorch runtime; no torch import or GPU arithmetic.

Falsifier: a runtime refusal, incomplete/nonfinite readback or failed numerical oracle emits success.
The fake supplies API responses and oracle refusals, not a claimed CUDA result.
"""
import contextlib
import io
import math
from pathlib import Path
import runpy
import sys
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

import helpers as H


class Runtime:
    def __init__(self, available=True, name="NVIDIA synthetic GPU", count=4096, finite=True, error=0):
        self.available, self.name = available, name
        self.count, self.finite, self.error = count, finite, error
        self.uploads, self.comparisons = 0, []
        self.module = NS(manual_seed=lambda seed: None, float32="float32",
                         cuda=NS(is_available=lambda: self.available, get_device_name=lambda i: self.name),
                         backends=NS(cuda=NS(matmul=NS(allow_tf32=True))),
                         arange=lambda *a, **kw: Tensor(self),
                         isfinite=lambda t: NS(all=lambda: self.finite),
                         testing=NS(assert_close=self.compare))

    def compare(self, actual, expected, **tolerance):
        self.comparisons.append(tolerance)
        assert actual.gpu and not expected.gpu, "oracle must compare GPU readback against CPU"
        assert math.isclose(.5 + self.error, .5, rel_tol=tolerance["rtol"], abs_tol=tolerance["atol"]), "synthetic mismatch"


class Tensor:
    def __init__(self, runtime, gpu=False):
        self.runtime, self.gpu = runtime, gpu

    def reshape(self, *args): return self
    def __truediv__(self, other): return self
    def flip(self, *args): return self
    def contiguous(self): return self
    def __matmul__(self, other): return Tensor(self.runtime, self.gpu or other.gpu)
    def cpu(self): return self
    def numel(self): return self.runtime.count

    def cuda(self):
        self.runtime.uploads += 1
        return Tensor(self.runtime, True)


class CudaRunner(unittest.TestCase):
    def run_fake(self, runtime):
        out = io.StringIO()
        error = None
        with patch.dict(sys.modules, {"torch": runtime.module}), contextlib.redirect_stdout(out):
            try:
                runpy.run_path(str(Path(H.AM) / "guest/py/release_cuda_smoke.py"))
            except AssertionError as e:
                error = e
        return error, out.getvalue()

    def test_checked_cuda_readback_emits_one_success(self):
        runtime = Runtime()
        error, output = self.run_fake(runtime)
        self.assertIsNone(error)
        self.assertEqual(output.count("CUDA_RELEASE_PASS"), 1)
        self.assertEqual(runtime.comparisons, [{"rtol": 1e-5, "atol": 1e-5}])
        self.assertEqual(runtime.uploads, 2)
        self.assertFalse(runtime.module.backends.cuda.matmul.allow_tf32)

    def test_cpu_only_runtime_is_refused(self):
        error, output = self.run_fake(Runtime(available=False))
        self.assertIsNotNone(error)
        self.assertNotIn("CUDA_RELEASE_PASS", output)

    def test_unavailable_cuda_never_uploads_a_tensor(self):
        runtime = Runtime(available=False, name="GeForce synthetic GPU")
        error, output = self.run_fake(runtime)
        self.assertIsNotNone(error)
        self.assertEqual(runtime.uploads, 0)

    def test_non_nvidia_device_is_refused(self):
        self.assertIsNotNone(self.run_fake(Runtime(name="Intel synthetic GPU"))[0])

    def test_unknown_adapter_cannot_emit_success(self):
        error, output = self.run_fake(Runtime(name="software adapter"))
        self.assertIsNotNone(error)
        self.assertNotIn("CUDA_RELEASE_PASS", output)

    def test_truncated_readback_is_refused(self):
        self.assertIsNotNone(self.run_fake(Runtime(count=0))[0])

    def test_nonfinite_readback_is_refused(self):
        error, output = self.run_fake(Runtime(finite=False))
        self.assertIsNotNone(error)
        self.assertNotIn("CUDA_RELEASE_PASS", output)

    def test_mismatching_readback_fails(self):
        self.assertIsNotNone(self.run_fake(Runtime(error=.01))[0])

    def test_divergent_cuda_values_cannot_emit_success(self):
        error, output = self.run_fake(Runtime(error=1))
        self.assertIsNotNone(error)
        self.assertNotIn("CUDA_RELEASE_PASS", output)


if __name__ == "__main__":
    unittest.main()
