# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Small Windows PyTorch CUDA known-answer check (GPU execution required, CPU is the oracle)."""
import torch

torch.manual_seed(1)
assert torch.cuda.is_available(), "CUDA unavailable"
name = torch.cuda.get_device_name(0)
assert "NVIDIA" in name.upper() or any(x in name for x in ("GeForce", "RTX", "Tesla")), name
torch.backends.cuda.matmul.allow_tf32 = False
a = torch.arange(64 * 64, dtype=torch.float32).reshape(64, 64) / 4096
b = a.flip(0).contiguous()
expected = a @ b
actual = (a.cuda() @ b.cuda()).cpu()
assert actual.numel() == 4096 and torch.isfinite(actual).all()
torch.testing.assert_close(actual, expected, rtol=1e-5, atol=1e-5)
print("CUDA_RELEASE_PASS device=%s elements=4096 seed=1" % name)
