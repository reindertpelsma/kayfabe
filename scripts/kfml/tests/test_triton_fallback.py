"""The fp16 tl.dot compile fallback of triton_kernels (found on the second die, sm_75): GPU-free logic test."""
import unittest

from tests import helpers  # noqa: F401
from kfml.workloads import triton_kernels as tk

ERR = RuntimeError("PassManager::run failed")


class Fallback(unittest.TestCase):
    def test_turing_compile_failure_is_recorded_and_falls_back(self):
        note = tk.fp16_dot_fallback(ERR, (7, 5), True)
        self.assertIn("failed on sm_75", note)
        self.assertIn("PassManager", note)

    def test_a_die_at_or_above_the_declared_capability_stays_a_failure(self):
        for cc in ((8, 0), (8, 9), (9, 0), (12, 0)):
            self.assertIsNone(tk.fp16_dot_fallback(ERR, cc, True), cc)

    def test_other_errors_and_fp32_failures_propagate(self):
        self.assertIsNone(tk.fp16_dot_fallback(RuntimeError("CUDA out of memory"), (7, 5), True))
        self.assertIsNone(tk.fp16_dot_fallback(ERR, (7, 5), False))  # already fp32: a failure is a failure
        self.assertIsNone(tk.fp16_dot_fallback(ERR, None, True))  # unknown cc: not a known-limit

    def test_the_threshold_is_not_a_die_literal(self):
        import re
        text = open(tk.__file__).read()
        self.assertIsNone(re.search(r"RTX|GeForce|Tesla|2080|\b[0-9]{3}\.[0-9]{2}\b", text))


if __name__ == "__main__":
    unittest.main()
