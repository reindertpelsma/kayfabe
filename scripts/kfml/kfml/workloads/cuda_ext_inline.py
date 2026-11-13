"""cuda_ext_inline: torch.utils.cpp_extension.load_inline with small CUDA kernels (shared-memory block
reduction, atomicAdd histogram, cooperative-groups warp reduction, grid-stride loop, __nanosleep/clock64),
compiled for the detected arch (TORCH_CUDA_ARCH_LIST = cc + PTX) and checked against torch."""
from __future__ import annotations

from ..spec import Spec

SPEC = Spec(
    name="cuda_ext_inline", module="kfml.workloads.cuda_ext_inline",
    description="load_inline CUDA kernels vs torch", tier="mid", budget_s=900, launch_bound=False,
    min_vram_gb=8.0, min_cc=(7, 5), min_torch="2.7", needs_nvcc=True, needs_compiler=True,
    download_note="needs nvcc matching torch.version.cuda; first run downloads several GB of CUDA wheels "
                  "(and ~100 MB of CUDA toolkit archives with setup --with-nvcc)")

CPP = r"""
torch::Tensor block_sum(torch::Tensor x);
torch::Tensor histogram(torch::Tensor x, int64_t bins);
torch::Tensor warp_sum_int(torch::Tensor x);
torch::Tensor saxpy_grid_stride(torch::Tensor x, torch::Tensor y, double a, int64_t max_blocks);
torch::Tensor atomic_float_sum(torch::Tensor x);
torch::Tensor spin_clock(int64_t iters);
"""

CUDA = r"""
#include <ATen/cuda/CUDAContext.h>
#include <c10/cuda/CUDAGuard.h>
#include <cooperative_groups.h>
#include <cooperative_groups/reduce.h>
namespace cg = cooperative_groups;

#define KF_CHECK(x) TORCH_CHECK(x.is_cuda() && x.is_contiguous(), #x " must be a contiguous CUDA tensor")

// 1. shared-memory tree reduction: one partial per block.
__global__ void block_sum_kernel(const float* __restrict__ x, float* __restrict__ part, int64_t n) {
  extern __shared__ float sh[];
  int tid = threadIdx.x;
  int64_t i = (int64_t)blockIdx.x * blockDim.x * 2 + tid;
  float v = 0.f;
  if (i < n) v += x[i];
  if (i + blockDim.x < n) v += x[i + blockDim.x];
  sh[tid] = v;
  __syncthreads();
  for (int s = blockDim.x / 2; s > 0; s >>= 1) {
    if (tid < s) sh[tid] += sh[tid + s];
    __syncthreads();
  }
  if (tid == 0) part[blockIdx.x] = sh[0];
}

torch::Tensor block_sum(torch::Tensor x) {
  KF_CHECK(x);
  const c10::cuda::CUDAGuard guard(x.device());
  int64_t n = x.numel();
  const int threads = 256;
  int64_t blocks = (n + threads * 2 - 1) / (threads * 2);
  auto part = torch::zeros({blocks}, x.options());
  block_sum_kernel<<<blocks, threads, threads * sizeof(float), at::cuda::getCurrentCUDAStream()>>>(
      x.data_ptr<float>(), part.data_ptr<float>(), n);
  C10_CUDA_KERNEL_LAUNCH_CHECK();
  return part;
}

// 2. histogram: privatized shared-memory bins merged with global atomicAdd.
__global__ void hist_kernel(const int* __restrict__ x, int* __restrict__ out, int64_t n, int bins) {
  extern __shared__ int sbins[];
  for (int b = threadIdx.x; b < bins; b += blockDim.x) sbins[b] = 0;
  __syncthreads();
  for (int64_t i = (int64_t)blockIdx.x * blockDim.x + threadIdx.x; i < n; i += (int64_t)gridDim.x * blockDim.x)
    atomicAdd(&sbins[x[i]], 1);
  __syncthreads();
  for (int b = threadIdx.x; b < bins; b += blockDim.x) atomicAdd(&out[b], sbins[b]);
}

torch::Tensor histogram(torch::Tensor x, int64_t bins) {
  KF_CHECK(x);
  TORCH_CHECK(x.scalar_type() == torch::kInt32, "int32 expected");
  const c10::cuda::CUDAGuard guard(x.device());
  auto out = torch::zeros({bins}, x.options());
  int64_t n = x.numel();
  int blocks = (int)std::min<int64_t>(1024, (n + 255) / 256 + 1);
  hist_kernel<<<blocks, 256, bins * sizeof(int), at::cuda::getCurrentCUDAStream()>>>(
      x.data_ptr<int>(), out.data_ptr<int>(), n, (int)bins);
  C10_CUDA_KERNEL_LAUNCH_CHECK();
  return out;
}

// 3. cooperative groups: tile-of-32 reduction of ints, one result per warp.
__global__ void warp_sum_kernel(const int* __restrict__ x, int* __restrict__ out, int64_t n) {
  auto block = cg::this_thread_block();
  auto tile = cg::tiled_partition<32>(block);
  int64_t i = (int64_t)blockIdx.x * blockDim.x + threadIdx.x;
  int v = (i < n) ? x[i] : 0;
  int s = cg::reduce(tile, v, cg::plus<int>());
  if (tile.thread_rank() == 0) out[i / 32] = s;
}

torch::Tensor warp_sum_int(torch::Tensor x) {
  KF_CHECK(x);
  TORCH_CHECK(x.scalar_type() == torch::kInt32, "int32 expected");
  const c10::cuda::CUDAGuard guard(x.device());
  int64_t n = x.numel();
  int64_t warps = (n + 31) / 32;
  int64_t blocks = (warps * 32 + 255) / 256;
  auto out = torch::zeros({blocks * 8}, x.options());
  warp_sum_kernel<<<blocks, 256, 0, at::cuda::getCurrentCUDAStream()>>>(x.data_ptr<int>(), out.data_ptr<int>(), n);
  C10_CUDA_KERNEL_LAUNCH_CHECK();
  return out.narrow(0, 0, warps);
}

// 4. grid-stride loop with a deliberately small grid so every thread loops many times.
__global__ void saxpy_kernel(const float* __restrict__ x, float* __restrict__ y, float a, int64_t n) {
  for (int64_t i = (int64_t)blockIdx.x * blockDim.x + threadIdx.x; i < n; i += (int64_t)gridDim.x * blockDim.x)
    y[i] = a * x[i] + y[i];
}

torch::Tensor saxpy_grid_stride(torch::Tensor x, torch::Tensor y, double a, int64_t max_blocks) {
  KF_CHECK(x); KF_CHECK(y);
  const c10::cuda::CUDAGuard guard(x.device());
  int64_t n = x.numel();
  int64_t blocks = std::min<int64_t>(max_blocks, (n + 255) / 256);
  auto out = y.clone();
  saxpy_kernel<<<blocks, 256, 0, at::cuda::getCurrentCUDAStream()>>>(x.data_ptr<float>(), out.data_ptr<float>(), (float)a, n);
  C10_CUDA_KERNEL_LAUNCH_CHECK();
  return out;
}

// 5. global float atomicAdd (inputs are small integers, so the sum is exact in any order).
__global__ void atomic_sum_kernel(const float* __restrict__ x, float* __restrict__ out, int64_t n) {
  for (int64_t i = (int64_t)blockIdx.x * blockDim.x + threadIdx.x; i < n; i += (int64_t)gridDim.x * blockDim.x)
    atomicAdd(out, x[i]);
}

torch::Tensor atomic_float_sum(torch::Tensor x) {
  KF_CHECK(x);
  const c10::cuda::CUDAGuard guard(x.device());
  auto out = torch::zeros({1}, x.options());
  atomic_sum_kernel<<<64, 256, 0, at::cuda::getCurrentCUDAStream()>>>(x.data_ptr<float>(), out.data_ptr<float>(), x.numel());
  C10_CUDA_KERNEL_LAUNCH_CHECK();
  return out;
}

// 6. __nanosleep + clock64, used sparingly: one thread, a few short sleeps.
__global__ void spin_kernel(long long* out, int iters) {
  if (threadIdx.x == 0) {
    long long t0 = clock64();
    for (int i = 0; i < iters; ++i) __nanosleep(1000);
    long long t1 = clock64();
    out[0] = t1 - t0;
  }
}

torch::Tensor spin_clock(int64_t iters) {
  auto out = torch::zeros({1}, torch::dtype(torch::kInt64).device(torch::kCUDA));
  spin_kernel<<<1, 32, 0, at::cuda::getCurrentCUDAStream()>>>((long long*)out.data_ptr<int64_t>(), (int)iters);
  C10_CUDA_KERNEL_LAUNCH_CHECK();
  return out;
}
"""

FUNCS = ["block_sum", "histogram", "warp_sum_int", "saxpy_grid_stride", "atomic_float_sum", "spin_clock"]


def run(ctx):
    import os
    import time
    import torch
    from torch.utils import cpp_extension
    from . import _common as C

    cc = "%d.%d" % tuple(ctx.cc)
    os.environ["TORCH_CUDA_ARCH_LIST"] = cc + "+PTX"
    build = os.path.join(os.environ.get("TORCH_EXTENSIONS_DIR", ctx.data_dir), "kfml_inline_%s" % cc.replace(".", "_"))
    os.makedirs(build, exist_ok=True)
    t0 = time.time()
    ext = cpp_extension.load_inline(name="kfml_inline_%s" % cc.replace(".", "_"), cpp_sources=CPP, cuda_sources=CUDA,
                                    functions=FUNCS, build_directory=build, verbose=False,
                                    extra_cuda_cflags=["-O2", "--std=c++17"])
    build_s = time.time() - t0
    dev = "cuda"
    g = torch.Generator().manual_seed(ctx.seed)
    metrics = {"arch_list": cc + "+PTX", "build_s": build_s}

    # block reduction: exact on integer-valued floats, tolerance on random floats
    for n in (1, 255, 256, 513, 1_000_003):
        xi = torch.randint(0, 8, (n,), generator=g).float().to(dev)
        got = ext.block_sum(xi).double().sum()
        C.check(float(got) == float(xi.double().sum()), "block_sum integer n=%d: %s != %s" % (n, float(got), float(xi.double().sum())))
        xf = torch.randn(n, generator=g).to(dev)
        got = float(ext.block_sum(xf).double().sum())
        ref = float(xf.double().sum())
        C.check(abs(got - ref) <= 1e-4 * max(1.0, float(xf.abs().double().sum())), "block_sum float n=%d" % n)

    # histogram
    for n, bins in ((1, 3), (1000, 16), (1_000_003, 257), (200_000, 1024)):
        x = torch.randint(0, bins, (n,), generator=g, dtype=torch.int32).to(dev)
        got = ext.histogram(x, bins)
        ref = torch.bincount(x.long(), minlength=bins).int()
        C.check(torch.equal(got, ref), "histogram n=%d bins=%d mismatch" % (n, bins))

    # cooperative-groups warp reduction (ints: exact); pad partial last warp with zeros
    for n in (32, 100, 8192, 100_003):
        x = torch.randint(-50, 50, (n,), generator=g, dtype=torch.int32).to(dev)
        got = ext.warp_sum_int(x)
        pad = (-n) % 32
        ref = torch.nn.functional.pad(x, (0, pad)).view(-1, 32).sum(1, dtype=torch.int32)
        C.check(torch.equal(got, ref), "warp_sum_int n=%d mismatch" % n)

    # grid-stride loop, forced stride (grid of 3 blocks)
    for n in (1, 777, 1_000_003):
        x = torch.randn(n, generator=g).to(dev)
        y = torch.randn(n, generator=g).to(dev)
        got = ext.saxpy_grid_stride(x, y, 0.37, 3)
        torch.testing.assert_close(got, 0.37 * x + y, rtol=1e-5, atol=1e-5)

    # atomicAdd of floats (exact on small integers)
    x = torch.randint(0, 4, (500_001,), generator=g).float().to(dev)
    C.check(float(ext.atomic_float_sum(x)) == float(x.double().sum()), "atomic float sum mismatch")

    # __nanosleep / clock64: sparing use, only sanity
    cyc = int(ext.spin_clock(20).item())
    C.check(cyc > 0, "clock64 delta not positive: %d" % cyc)
    metrics["spin_clock_cycles"] = cyc
    # the PTX/arch we built for really runs on this device: any launch above already proved it
    return {"metrics": metrics}
