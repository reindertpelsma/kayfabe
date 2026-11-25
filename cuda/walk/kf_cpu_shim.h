/* kf_cpu_shim.h — compile kf_walk.cu's DEVICE half with a host C++ compiler (2026-11-13).
 *
 * ⊘ Not an emulator of the GPU. It makes the CUDA keywords vanish so that the functions that are
 * single-threaded by construction -- the serial walk (`kf_walk_one`), the decode, the vetoes, the
 * coalescer, the device-side descriptor check -- can be CALLED on a CPU with no GPU and no CUDA SDK.
 * The warp-parallel kernels compile but are never called here: their warp intrinsics are stubs that
 * would give wrong answers, so `kf_cpu_walk.cpp` must not launch them (it has no way to).
 * What this buys is GPU-free CI over the REAL source text: the same `kf_walk.cu` NVRTC turns into
 * the shipped PTX, not a copy of its logic. The parallel walk is graded by `kf_tests` on a GPU.
 */
#pragma once
#include <stdint.h>
#include <stddef.h>
#include <string.h>

#define __device__
#define __global__
#define __host__
#define __forceinline__ inline
#define __shared__
#define __launch_bounds__(x)

struct kf_cpu_dim3 { unsigned x, y, z; };
static kf_cpu_dim3 threadIdx, blockIdx, blockDim, gridDim;

/* Single-threaded: an atomic is a read-modify-write. */
template <class T> static inline T atomicAdd(T *p, T v) { T o = *p; *p = o + v; return o; }
template <class T> static inline T atomicOr(T *p, T v) { T o = *p; *p = o | v; return o; }
/* Stubs for the warp kernels, which this harness never calls. */
static inline unsigned __ballot_sync(unsigned, int) { return 0u; }
template <class T> static inline T __shfl_sync(unsigned, T v, int) { return v; }
template <class T> static inline T __shfl_up_sync(unsigned, T v, unsigned) { return v; }
static inline void __syncwarp(void) {}
static inline void __syncthreads(void) {}
static inline int __popc(unsigned x) { return __builtin_popcount(x); }
