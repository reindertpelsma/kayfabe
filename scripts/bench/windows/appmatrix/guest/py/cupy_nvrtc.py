#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""cupy_nvrtc.py TEST -- kernels compiled at run time with NVRTC through CuPy (no nvcc, no MSVC): the Windows
counterparts of simpleCooperativeGroups (cg), globalToShmemAsyncCopy (asynccopy) and the nvkvm-pv sha256 kernel
(sha256). Headers come from the staged cuda_cudart/cuda_cccl redistributables (env KF_CUDA_INCLUDE, ';'-separated).
Prints `CHECK <key> ok|FAIL` and `NVRTC_DONE <test>`.  Tests: cg asynccopy sha256 list"""
import hashlib
import os
import sys

import numpy as np
import torch   # imported first: it pre-loads the CUDA runtime/NVRTC DLLs that CuPy then finds by name
import cupy as cp

INC = [p for p in os.environ.get("KF_CUDA_INCLUDE", "").split(";") if p]
OPT = tuple("-I" + p for p in INC) + ("--std=c++17",)


def chk(k, ok, extra=""):
    print(f"CHECK {k} {'ok' if ok else 'FAIL'} {extra}".rstrip(), flush=True)


def t_cg():
    src = r'''
    #include <cooperative_groups.h>
    namespace cg = cooperative_groups;
    extern "C" __global__ void blocksum(const float* in, float* out, int n) {
        cg::thread_block blk = cg::this_thread_block();
        cg::thread_block_tile<32> tile = cg::tiled_partition<32>(blk);
        int i = blockIdx.x * blockDim.x + threadIdx.x;
        float v = i < n ? in[i] : 0.f;
        for (int o = tile.size() / 2; o > 0; o /= 2) v += tile.shfl_down(v, o);
        __shared__ float part[32];
        if (tile.thread_rank() == 0) part[threadIdx.x / 32] = v;
        blk.sync();
        if (threadIdx.x == 0) { float s = 0; for (int w = 0; w < blockDim.x / 32; w++) s += part[w]; out[blockIdx.x] = s; }
    }'''
    k = cp.RawKernel(src, "blocksum", options=OPT)
    n = 1 << 20; x = np.random.default_rng(1).random(n, dtype=np.float32); d = cp.asarray(x); out = cp.zeros(n // 256, dtype=cp.float32)
    k((n // 256,), (256,), (d, out, np.int32(n))); cp.cuda.Device().synchronize()
    ref = x.reshape(-1, 256).sum(1, dtype=np.float64)
    chk("cooperative_groups_block_sum", bool(np.allclose(cp.asnumpy(out), ref, rtol=1e-4)))


def t_asynccopy():
    src = r'''
    #include <cuda_pipeline.h>
    extern "C" __global__ void copy_sum(const float* in, float* out) {
        extern __shared__ float tile[];
        int i = blockIdx.x * blockDim.x + threadIdx.x;
        __pipeline_memcpy_async(&tile[threadIdx.x], &in[i], sizeof(float));
        __pipeline_commit(); __pipeline_wait_prior(0); __syncthreads();
        out[i] = tile[blockDim.x - 1 - threadIdx.x] * 2.f;
    }'''
    k = cp.RawKernel(src, "copy_sum", options=OPT)
    n = 1 << 20; x = np.arange(n, dtype=np.float32); d = cp.asarray(x); out = cp.zeros_like(d)
    k((n // 256,), (256,), (d, out), shared_mem=256 * 4); cp.cuda.Device().synchronize()
    ref = (x.reshape(-1, 256)[:, ::-1] * 2).reshape(-1)
    chk("async_copy_pipeline", bool(np.array_equal(cp.asnumpy(out), ref)))


SHA = r'''
typedef unsigned int u32;
__device__ __constant__ u32 K[64] = {0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2};
#define ROR(x,n) (((x) >> (n)) | ((x) << (32 - (n))))
extern "C" __global__ void sha256_one_block(const unsigned char* msgs, unsigned char* digests, int n) {
    int t = blockIdx.x * blockDim.x + threadIdx.x; if (t >= n) return;
    u32 w[64]; const unsigned char* m = msgs + (size_t)t * 64;      // each message is already padded to one 64-byte block
    for (int i = 0; i < 16; i++) w[i] = (u32)m[4*i] << 24 | (u32)m[4*i+1] << 16 | (u32)m[4*i+2] << 8 | m[4*i+3];
    for (int i = 16; i < 64; i++) { u32 s0 = ROR(w[i-15],7) ^ ROR(w[i-15],18) ^ (w[i-15] >> 3), s1 = ROR(w[i-2],17) ^ ROR(w[i-2],19) ^ (w[i-2] >> 10); w[i] = w[i-16] + s0 + w[i-7] + s1; }
    u32 a=0x6a09e667,b=0xbb67ae85,c=0x3c6ef372,d=0xa54ff53a,e=0x510e527f,f=0x9b05688c,g=0x1f83d9ab,h=0x5be0cd19;
    for (int i = 0; i < 64; i++) { u32 S1 = ROR(e,6)^ROR(e,11)^ROR(e,25), ch = (e&f)^(~e&g), t1 = h+S1+ch+K[i]+w[i], S0 = ROR(a,2)^ROR(a,13)^ROR(a,22), mj = (a&b)^(a&c)^(b&c), t2 = S0+mj; h=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2; }
    u32 o[8] = {a+0x6a09e667,b+0xbb67ae85,c+0x3c6ef372,d+0xa54ff53a,e+0x510e527f,f+0x9b05688c,g+0x1f83d9ab,h+0x5be0cd19};
    for (int i = 0; i < 8; i++) { digests[t*32+4*i] = o[i] >> 24; digests[t*32+4*i+1] = o[i] >> 16; digests[t*32+4*i+2] = o[i] >> 8; digests[t*32+4*i+3] = o[i]; }
}'''


def t_sha256():
    n = 4096; rng = np.random.default_rng(9); msgs = np.zeros((n, 64), dtype=np.uint8); ref = []
    for i in range(n):
        body = rng.integers(0, 256, 40, dtype=np.uint8).tobytes(); ref.append(hashlib.sha256(body).digest())
        pad = body + b"\x80" + b"\x00" * (64 - 8 - len(body) - 1) + (len(body) * 8).to_bytes(8, "big"); msgs[i] = np.frombuffer(pad, dtype=np.uint8)
    k = cp.RawKernel(SHA, "sha256_one_block"); dm = cp.asarray(msgs); dd = cp.zeros((n, 32), dtype=cp.uint8)
    k((n // 128,), (128,), (dm, dd, np.int32(n))); cp.cuda.Device().synchronize()
    got = cp.asnumpy(dd); chk("sha256_4096_messages_vs_hashlib", all(bytes(got[i]) == ref[i] for i in range(n)))


TESTS = {"cg": t_cg, "asynccopy": t_asynccopy, "sha256": t_sha256}
if __name__ == "__main__":
    if len(sys.argv) < 2 or sys.argv[1] == "list":
        print(" ".join(TESTS)); sys.exit(0)
    print("[nvrtc] device", torch.cuda.get_device_name(0), "cupy", cp.__version__, "include", INC, file=sys.stderr)
    TESTS[sys.argv[1]]()
    print(f"NVRTC_DONE {sys.argv[1]}", flush=True)
