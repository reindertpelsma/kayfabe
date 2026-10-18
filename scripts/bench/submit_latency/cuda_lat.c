// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// cuda_lat.c [N] [GAP_US]: HOST-native reference for the "GPU leg": tiny GR launch + sync, and a 64 KiB device-to-device copy + sync,
// each timed with CLOCK_MONOTONIC. Driver API through dlopen (no CUDA toolkit needed). Runs on the host GPU beside the VM.
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>
#include <time.h>
#include <unistd.h>
#include <stdint.h>
typedef int CUresult; typedef void *CUcontext, *CUmodule, *CUfunction, *CUstream; typedef unsigned long long CUdeviceptr;
static void *L;
#define SYM(n) static CUresult (*n)(); 
SYM(cuInit) SYM(cuDeviceGet) SYM(cuCtxCreate_v2) SYM(cuModuleLoadData) SYM(cuModuleGetFunction) SYM(cuLaunchKernel) SYM(cuCtxSynchronize)
SYM(cuMemAlloc_v2) SYM(cuMemcpyDtoDAsync_v2) SYM(cuStreamCreate) SYM(cuStreamSynchronize)
static void ld(void) {
  L = dlopen("libcuda.so.1", RTLD_NOW); if (!L) { fprintf(stderr, "no libcuda\n"); exit(1); }
#define G(n) n = dlsym(L, #n); if (!n) { fprintf(stderr, "missing %s\n", #n); exit(1); }
  G(cuInit) G(cuDeviceGet) G(cuCtxCreate_v2) G(cuModuleLoadData) G(cuModuleGetFunction) G(cuLaunchKernel) G(cuCtxSynchronize) G(cuMemAlloc_v2) G(cuMemcpyDtoDAsync_v2) G(cuStreamCreate) G(cuStreamSynchronize)
}
static double now_us(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e6 + t.tv_nsec / 1e3; }
static int cmp(const void *a, const void *b) { double x = *(double *)a, y = *(double *)b; return x < y ? -1 : x > y; }
static void rep(const char *n, double *v, int k) { qsort(v, k, sizeof *v, cmp); printf("%-34s n=%d med=%.1f p90=%.1f max=%.1f min=%.1f us\n", n, k, v[k / 2], v[(int)(k * .9)], v[k - 1], v[0]); }
int main(int argc, char **argv) {
  int N = argc > 1 ? atoi(argv[1]) : 2000, gap = argc > 2 ? atoi(argv[2]) : 0; const char *log = argc > 3 ? argv[3] : 0; int ce = argc > 4 ? atoi(argv[4]) : 0;
  ld(); int dev; CUcontext ctx; CUmodule mod; CUfunction fn; CUstream st; CUdeviceptr a, b;
  if (cuInit(0) || cuDeviceGet(&dev, 0) || cuCtxCreate_v2(&ctx, 0, dev)) { puts("ctx fail"); return 1; }
  const char *ptx = ".version 7.0\n.target sm_50\n.address_size 64\n.visible .entry k() { ret; }\n";
  if (cuModuleLoadData(&mod, ptx) || cuModuleGetFunction(&fn, mod, "k")) { puts("module fail"); return 1; }
  cuStreamCreate(&st, 0); cuMemAlloc_v2(&a, 65536); cuMemAlloc_v2(&b, 65536);
  double *v = malloc(N * sizeof *v);
  for (int i = 0; i < 100; i++) { cuLaunchKernel(fn, 1, 1, 1, 1, 1, 1, 0, st, 0, 0); cuStreamSynchronize(st); }
  for (int i = 0; i < N; i++) { if (gap) usleep(gap); double t0 = now_us(); cuLaunchKernel(fn, 1, 1, 1, 1, 1, 1, 0, st, 0, 0); cuStreamSynchronize(st); v[i] = now_us() - t0; }
  if (log) {   // timeline mode: GR launch+sync (or CE copy+sync if argv[4]=1) every GAP us, "t_ns lat_us" per sample, for N samples
    FILE *o = fopen(log, "w");
    for (int i = 0; i < N; i++) { if (gap) usleep(gap); double t0 = now_us();
      if (ce) cuMemcpyDtoDAsync_v2(b, a, 65536, st); else cuLaunchKernel(fn, 1, 1, 1, 1, 1, 1, 0, st, 0, 0);
      cuStreamSynchronize(st); double t1 = now_us(); fprintf(o, "%.0f %.1f\n", t0 * 1000, t1 - t0); }
    fclose(o); return 0; }
  rep("GR: empty kernel launch+sync", v, N);
  for (int i = 0; i < N; i++) { if (gap) usleep(gap); double t0 = now_us(); cuMemcpyDtoDAsync_v2(b, a, 65536, st); cuStreamSynchronize(st); v[i] = now_us() - t0; }
  rep("CE: 64 KiB D2D copy+sync", v, N);
  return 0;
}
