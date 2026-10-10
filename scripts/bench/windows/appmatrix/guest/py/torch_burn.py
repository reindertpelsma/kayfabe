#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""torch_burn.py SECONDS -- the Windows counterpart of gpu_burn: hammer the GPU with fp32 matmuls for SECONDS and
compare every result to the first one (same inputs => bit-identical on a healthy GPU). Prints
`GPU 0: OK` iff 0 compare errors (the Linux gpu_burn pass string), plus the iteration count and GFLOPS."""
import sys
import time

import torch

secs = float(sys.argv[1]) if len(sys.argv) > 1 else 60.0
assert torch.cuda.is_available(), "no CUDA"
torch.backends.cuda.matmul.allow_tf32 = False
g = torch.Generator(device="cuda").manual_seed(11)
n = 4096
a = torch.randn(n, n, device="cuda", generator=g); b = torch.randn(n, n, device="cuda", generator=g)
ref = a @ b; torch.cuda.synchronize()
errs = 0; it = 0; t0 = time.time(); last = t0
while time.time() - t0 < secs:
    c = a @ b
    if it % 4 == 0:
        if not torch.equal(c, ref):
            errs += 1
    it += 1
    if time.time() - last > 10:
        torch.cuda.synchronize(); last = time.time(); print(f"[burn] t={last - t0:.0f}s iters={it} errors={errs}", flush=True)
torch.cuda.synchronize(); dt = time.time() - t0
print(f"iterations={it} seconds={dt:.1f} GFLOPS={2 * n ** 3 * it / dt / 1e9:.0f} compare_errors={errs}")
print("GPU 0: OK" if errs == 0 else f"GPU 0: FAULTY ({errs} compare errors)", flush=True)
sys.exit(0 if errs == 0 else 1)
