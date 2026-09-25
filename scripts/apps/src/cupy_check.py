#!/usr/bin/env python3
# cupy_check.py — CuPy (NVRTC-compiled elementwise/reduction kernels, cuBLAS, cuFFT, cuRAND) vs NumPy.
import numpy as np, cupy as cp
def chk(k, ok): print(f"CHECK {k} {'ok' if ok else 'FAIL'}", flush=True)
rs = np.random.default_rng(3)
a = rs.standard_normal((512, 512)).astype(np.float64); b = rs.standard_normal((512, 512))
chk("cupy_matmul", np.allclose(cp.asnumpy(cp.asarray(a) @ cp.asarray(b)), a @ b))
x = rs.standard_normal(1 << 20)
chk("cupy_reduce", abs(float(cp.asarray(x).sum()) - x.sum()) < 1e-6)
k = cp.ElementwiseKernel("float64 x", "float64 y", "y = x * x + 1.0", "sq1")
chk("cupy_nvrtc_kernel", np.allclose(cp.asnumpy(k(cp.asarray(x))), x * x + 1.0))
chk("cupy_fft", np.allclose(cp.asnumpy(cp.fft.fft(cp.asarray(x[:4096]))), np.fft.fft(x[:4096])))
r = cp.random.default_rng(1).standard_normal(1 << 20)
chk("cupy_random", abs(float(r.mean())) < 0.01 and abs(float(r.std()) - 1) < 0.01)
print("CUPY_DONE")
