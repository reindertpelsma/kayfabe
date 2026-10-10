#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""win_cuda_equiv.py TEST -- Windows counterparts of the Linux app matrix's CUDA samples / realapp kernels,
written against PyTorch (and CuPy for managed memory), because a Windows guest has no nvcc/MSVC and NVIDIA
ships no prebuilt cuda-samples binaries. Every test computes on the GPU and checks against a CPU reference,
printing `CHECK <key> ok|FAIL <detail>` lines (the Linux run_apps.sh vocabulary) and a final
`WINEQ_DONE <test>`. The test that maps to each Linux row is listed in apps.json (`linux`).
  win_cuda_equiv.py list
"""
import os
import sys
import time

import torch

# KF_EQ_DEVICE=cpu runs the same checks against the CPU (logic test of this file on a GPU-less machine; the matrix never sets it)
DEV = os.environ.get("KF_EQ_DEVICE", "cuda")
G = torch.Generator().manual_seed(2026)


def chk(k, ok, extra=""):
    print(f"CHECK {k} {'ok' if ok else 'FAIL'} {extra}".rstrip(), flush=True)


def rel(a, b):
    d = (a - b).abs().double(); n = b.abs().double()          # abs(): complex-safe
    return (d.norm() / n.norm().clamp_min(1e-30)).item()


def t_matmul():                      # matrixMul, matrixMulDrv, sgemm_cublas, simpleCUBLAS
    torch.backends.cuda.matmul.allow_tf32 = False
    a = torch.randn(1024, 1024, generator=G); b = torch.randn(1024, 1024, generator=G)
    c = (a.to(DEV) @ b.to(DEV)).cpu(); r = a @ b
    e = (c - r).abs().max().item(); chk("matmul_fp32", e < 1e-2, f"maxerr={e:.2e}")


def t_tc16():                        # cudaTensorCoreGemm
    a = torch.randn(2048, 2048, generator=G); b = torch.randn(2048, 2048, generator=G)
    c = (a.to(DEV).half() @ b.to(DEV).half()).float().cpu(); e = rel(c, a @ b)
    chk("gemm_fp16_tensorcore", e < 5e-3, f"relerr={e:.2e}")


def t_tcbf16():                      # bf16TensorCoreGemm
    a = torch.randn(2048, 2048, generator=G); b = torch.randn(2048, 2048, generator=G)
    c = (a.to(DEV).bfloat16() @ b.to(DEV).bfloat16()).float().cpu(); e = rel(c, a @ b)
    chk("gemm_bf16_tensorcore", e < 3e-2, f"relerr={e:.2e}")


def t_fft():                         # simpleCUFFT, fft_cufft
    x = torch.randn(8, 1 << 16, dtype=torch.complex64, generator=G)
    y = torch.fft.fft(x.to(DEV)).cpu(); e = rel(y, torch.fft.fft(x)); chk("fft_forward", e < 1e-4, f"relerr={e:.2e}")
    z = torch.fft.ifft(torch.fft.fft(x.to(DEV))).cpu(); e = rel(z, x); chk("fft_roundtrip", e < 1e-4, f"relerr={e:.2e}")


def t_rng():                         # MersenneTwisterGP11213 (curand): statistics + reproducibility
    g1 = torch.Generator(device=DEV).manual_seed(7); g2 = torch.Generator(device=DEV).manual_seed(7)
    u = torch.rand(1 << 22, device=DEV, generator=g1); v = torch.rand(1 << 22, device=DEV, generator=g2)
    chk("rng_reproducible", torch.equal(u, v))
    m = u.mean().item(); s = u.std().item()
    chk("rng_uniform_moments", abs(m - 0.5) < 2e-3 and abs(s - 0.28868) < 2e-3, f"mean={m:.5f} std={s:.5f}")
    n = torch.randn(1 << 22, device=DEV, generator=g1)
    chk("rng_normal_moments", abs(n.mean().item()) < 5e-3 and abs(n.std().item() - 1) < 5e-3)


def t_reduce():                      # reduction, reduce
    x = torch.randn(1 << 24, generator=G)
    chk("reduce_sum_f32", abs(x.to(DEV).sum().item() - x.double().sum().item()) < 2.0, "tol=2.0 over 16M")
    i = torch.randint(-1000, 1000, (1 << 24,), generator=G, dtype=torch.int64)
    chk("reduce_sum_i64_exact", i.to(DEV).sum().item() == i.sum().item())
    chk("reduce_minmax", x.to(DEV).max().item() == x.max().item() and x.to(DEV).min().item() == x.min().item())


def t_scan():                        # scan
    i = torch.randint(0, 100, (1 << 22,), generator=G, dtype=torch.int64)
    chk("scan_cumsum_i64_exact", torch.equal(i.to(DEV).cumsum(0).cpu(), i.cumsum(0)))
    f = torch.rand(1 << 22, generator=G, dtype=torch.float64)
    chk("scan_cumsum_f64", torch.allclose(f.to(DEV).cumsum(0).cpu(), f.cumsum(0), rtol=1e-9))


def t_sort():                        # sortingNetworks
    k = torch.randint(0, 1 << 30, (1 << 22,), generator=G, dtype=torch.int32)
    s = k.to(DEV).sort().values.cpu(); chk("sort_i32_exact", torch.equal(s, k.sort().values))
    v, idx = k.to(DEV).sort(stable=True); chk("sort_argsort_valid", torch.equal(k[idx.cpu()], v.cpu()))


def t_hist():                        # histogram
    x = torch.randint(0, 256, (1 << 22,), generator=G, dtype=torch.int64)
    chk("histogram_bincount_exact", torch.equal(torch.bincount(x.to(DEV), minlength=256).cpu(), torch.bincount(x, minlength=256)))


def t_transpose():                   # transpose
    x = torch.randn(4096, 4096, generator=G)
    chk("transpose_exact", torch.equal(x.to(DEV).t().contiguous().cpu(), x.t().contiguous()))


def t_blackscholes():                # BlackScholes, blackscholes
    n = 1 << 22
    s = torch.rand(n, generator=G, dtype=torch.float64) * 90 + 10; k = torch.rand(n, generator=G, dtype=torch.float64) * 90 + 10
    t = torch.rand(n, generator=G, dtype=torch.float64) * 4.9 + 0.1; r, v = 0.02, 0.30

    def call(s, k, t):
        d1 = (torch.log(s / k) + (r + 0.5 * v * v) * t) / (v * torch.sqrt(t)); d2 = d1 - v * torch.sqrt(t)
        cdf = lambda x: 0.5 * (1 + torch.erf(x / 2 ** 0.5))
        return s * cdf(d1) - k * torch.exp(-r * t) * cdf(d2)
    g = call(s.to(DEV).float(), k.to(DEV).float(), t.to(DEV).float()).double().cpu(); ref = call(s, k, t)
    e = rel(g, ref); chk("blackscholes_call", e < 1e-4, f"relerr={e:.2e}")


def t_fwt():                         # fastWalshTransform
    def fwht(x):
        n, h = x.numel(), 1
        while h < n:
            x = x.reshape(n // (2 * h), 2, h); a, b = x[:, 0, :], x[:, 1, :]
            x = torch.stack((a + b, a - b), 1).reshape(n); h *= 2
        return x
    x = torch.randint(-8, 8, (1 << 16,), generator=G).float()
    chk("fwht_exact", torch.equal(fwht(x.to(DEV)).cpu(), fwht(x)))


def t_streams():                     # simpleStreams, concurrentKernels, asyncAPI
    st = [torch.cuda.Stream() for _ in range(4)]; outs = []; a = torch.randn(1024, 1024, generator=G).to(DEV)
    ev0, ev1 = torch.cuda.Event(enable_timing=True), torch.cuda.Event(enable_timing=True)
    ev0.record()
    for i, s in enumerate(st):
        with torch.cuda.stream(s):
            outs.append((a * float(i + 1)).sum())
    ev1.record(); torch.cuda.synchronize()
    ref = a.double().sum().item()
    chk("streams_4_independent", all(abs(o.item() - ref * (i + 1)) < 5.0 for i, o in enumerate(outs)))
    chk("streams_event_timing", ev0.elapsed_time(ev1) >= 0, f"ms={ev0.elapsed_time(ev1):.3f}")
    s1, s2 = torch.cuda.Stream(), torch.cuda.Stream(); e = torch.cuda.Event()
    with torch.cuda.stream(s1):
        x = a @ a; e.record()
    with torch.cuda.stream(s2):
        s2.wait_event(e); y = x + 1
    torch.cuda.synchronize(); chk("streams_cross_dependency", torch.allclose(y.cpu(), (a.cpu() @ a.cpu()) + 1, atol=5e-2, rtol=1e-3))


def t_graphs():                      # simpleCudaGraphs, graphMemoryNodes
    a = torch.randn(512, 512, device=DEV, generator=torch.Generator(device=DEV).manual_seed(3)); x = torch.zeros(512, 512, device=DEV)
    s = torch.cuda.Stream(); s.wait_stream(torch.cuda.current_stream())
    with torch.cuda.stream(s):
        y = a @ x + 1
    torch.cuda.current_stream().wait_stream(s)
    g = torch.cuda.CUDAGraph()
    with torch.cuda.graph(g):
        y = a @ x + 1
    ok = True
    for i in range(10):
        x.copy_(torch.full((512, 512), float(i), device=DEV)); g.replay(); torch.cuda.synchronize()
        ok = ok and torch.allclose(y.cpu(), a.cpu() @ x.cpu() + 1, atol=1e-2, rtol=1e-3)
    chk("cuda_graph_replay_10", ok)


def t_atomics():                     # simpleAtomicIntrinsics (colliding atomicAdd inside index_add_)
    idx = torch.randint(0, 997, (1 << 22,), generator=G)
    out = torch.zeros(997, device=DEV).index_add_(0, idx.to(DEV), torch.ones(1 << 22, device=DEV))
    chk("atomic_add_collisions_exact", torch.equal(out.cpu(), torch.bincount(idx, minlength=997).float()))


def t_zerocopy():                    # simpleZeroCopy: pinned host memory
    h = torch.arange(1 << 24, dtype=torch.float32).pin_memory()
    d = h.to(DEV, non_blocking=True); torch.cuda.synchronize()
    chk("pinned_h2d", torch.equal(d.cpu(), h)); d += 1; r = torch.empty_like(h).pin_memory(); r.copy_(d, non_blocking=True); torch.cuda.synchronize()
    chk("pinned_d2h", torch.equal(r, h + 1))


def t_mandelbrot():                  # mandelbrot
    n = 512; ys, xs = torch.meshgrid(torch.linspace(-1.2, 1.2, n), torch.linspace(-2.0, 0.8, n), indexing="ij")

    def run(c):
        z = torch.zeros_like(c); cnt = torch.zeros(c.shape, dtype=torch.int32, device=c.device)
        for _ in range(100):
            m = (z.real ** 2 + z.imag ** 2) <= 4; z = torch.where(m, z * z + c, z); cnt += m.int()
        return cnt
    c = torch.complex(xs, ys); g = run(c.to(DEV)).cpu(); r = run(c)
    agree = (g == r).float().mean().item(); chk("mandelbrot_vs_cpu", agree > 0.999, f"agree={agree:.5f}")


def t_nbody():                       # nbody
    n = 2048; p = torch.randn(n, 3, generator=G)

    def acc(p):
        d = p[None, :, :] - p[:, None, :]; r2 = (d * d).sum(-1) + 0.01
        return (d * (r2 ** -1.5)[..., None]).sum(1)
    g = acc(p.to(DEV)).cpu(); ref = acc(p.double()).float(); e = rel(g, ref); chk("nbody_accel", e < 1e-3, f"relerr={e:.2e}")


def t_memcpy2d():                    # memcpy2d
    x = torch.randn(2048, 3072, generator=G); v = x[::2, ::3]
    chk("strided_h2d_d2h", torch.equal(v.to(DEV).cpu(), v))
    d = torch.zeros(2048, 3072, device=DEV); d[::2, ::3] = v.to(DEV)
    chk("strided_scatter", torch.equal(d[::2, ::3].cpu(), v))


def t_triad():                       # stream_triad
    n = 1 << 26; b = torch.rand(n, device=DEV); c = torch.rand(n, device=DEV); a = torch.empty(n, device=DEV)
    torch.add(b, c, alpha=3.0, out=a); torch.cuda.synchronize(); t0 = time.time()
    for _ in range(10):
        torch.add(b, c, alpha=3.0, out=a)
    torch.cuda.synchronize(); dt = (time.time() - t0) / 10
    bw = 3 * n * 4 / dt / 1e9
    chk("triad_correct", torch.allclose(a.cpu()[:1000], (b + 3.0 * c).cpu()[:1000]))
    chk("triad_bandwidth_sane", bw > 1.0, f"GB/s={bw:.1f}")


def t_cg():                          # conjugateGradient: CG on a dense SPD system, residual checked on the host
    n = 1024; m = torch.randn(n, n, generator=G, dtype=torch.float64); a = (m @ m.T + n * torch.eye(n, dtype=torch.float64)).to(DEV)
    b = torch.randn(n, generator=G, dtype=torch.float64).to(DEV); x = torch.zeros_like(b); r = b.clone(); p = r.clone(); rs = r @ r
    for i in range(200):
        ap = a @ p; al = rs / (p @ ap); x += al * p; r -= al * ap; rn = r @ r
        if rn.sqrt().item() < 1e-10:
            break
        p = r + (rn / rs) * p; rs = rn
    res = (a.cpu() @ x.cpu() - b.cpu()).norm().item() / b.cpu().norm().item()
    chk("cg_residual", res < 1e-8, f"iters={i + 1} relres={res:.2e}")


def t_managed():                     # UnifiedMemoryStreams/Perf, conjugateGradientUM, attach_verify (CuPy managed memory)
    import ctypes
    import cupy as cp
    import numpy as np
    n = 1 << 26                       # 256 MiB of float32
    mem = cp.cuda.malloc_managed(n * 4)
    arr = cp.ndarray((n,), dtype=cp.float32, memptr=cp.cuda.MemoryPointer(mem, 0))
    arr[:] = cp.arange(n, dtype=cp.float32); cp.cuda.Device().synchronize()
    host = np.frombuffer((ctypes.c_char * (n * 4)).from_address(mem.ptr), dtype=np.float32)
    chk("managed_gpu_write_cpu_read", bool(host[12345] == 12345.0 and host[n - 1] == float(n - 1)))
    host[:1024] = 7.0                 # CPU writes, GPU reads
    chk("managed_cpu_write_gpu_read", float(arr[:1024].sum()) == 7.0 * 1024)


def streams_probe(name):             # stream_probe {default, created, nonblocking, two, created2nd}
    x = torch.arange(1 << 20, dtype=torch.float32, device=DEV)
    if name == "default":
        y = x * 2
    elif name in ("created", "nonblocking"):   # torch streams are created cudaStreamNonBlocking
        s = torch.cuda.Stream()
        with torch.cuda.stream(s):
            y = x * 2
        s.synchronize()
    elif name == "two":
        s1, s2 = torch.cuda.Stream(), torch.cuda.Stream()
        with torch.cuda.stream(s1):
            y1 = x * 2
        with torch.cuda.stream(s2):
            y2 = x * 3
        torch.cuda.synchronize(); y = y1; chk("two_streams_second", torch.equal(y2.cpu(), (x * 3).cpu()))
    elif name == "perthread":                  # CuPy's per-thread default stream (cudaStreamPerThread)
        import cupy as cp
        with cp.cuda.Stream.ptds:
            y = torch.as_tensor(cp.asarray(x) * 2, device=DEV)
    elif name == "created2nd":
        s = torch.cuda.Stream()
        with torch.cuda.stream(s):
            y = x * 2
        s.synchronize(); s2 = torch.cuda.Stream()
        with torch.cuda.stream(s2):
            y = y + 0
        s2.synchronize()
    else:
        raise SystemExit(f"unknown probe {name}")
    torch.cuda.synchronize()
    ok = torch.equal(y.cpu(), (x * 2).cpu()); chk(f"stream_{name}", ok)
    print(f"STREAM_PROBE_DONE {name} rc={0 if ok else 1}", flush=True)


TESTS = {k[2:]: v for k, v in list(globals().items()) if k.startswith("t_") and callable(v)}

if __name__ == "__main__":
    if len(sys.argv) < 2 or sys.argv[1] == "list":
        print(" ".join(sorted(TESTS)) + " stream_default stream_created stream_nonblocking stream_perthread stream_two stream_created2nd")
        sys.exit(0)
    name = sys.argv[1]
    if name.startswith("stream_"):
        assert torch.cuda.is_available(), "no CUDA"
        print("[eq] device", torch.cuda.get_device_name(0), "torch", torch.__version__, file=sys.stderr)
        streams_probe(name[7:]); sys.exit(0)
    if name not in TESTS:
        print(f"unknown test {name}; try: list"); sys.exit(2)
    if DEV == "cuda":
        assert torch.cuda.is_available(), "no CUDA"
        print("[eq] device", torch.cuda.get_device_name(0), "torch", torch.__version__, file=sys.stderr)
    TESTS[name]()
    print(f"WINEQ_DONE {name}", flush=True)
