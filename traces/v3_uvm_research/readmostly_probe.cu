// readmostly_probe — does kayfabe honour the GUEST UVM's read duplication?
// (docs/design/V3_UVM_DEMAND_PAGING.md §3). Every CUDA status and every value is checked.
// build: nvcc -O2 -arch=sm_86 -cudart static -o readmostly_probe readmostly_probe.cu
// usage: readmostly_probe <mode>
//   reprefetch : ReadMostly, CPU init, prefetch->GPU, GPU read, CPU read, CPU write,
//                prefetch->GPU again, GPU read.  Needs NO demand fault: the collapse
//                (CPU write) must UNMAP the GPU duplicate before the second prefetch.
//   fault      : as reprefetch but WITHOUT the second prefetch: the GPU read after the
//                collapse needs a replayable fault (expected 719 on kf3 today).
//   gpuwrite   : ReadMostly, CPU init, prefetch->GPU (RO duplicate), GPU WRITE, sync,
//                CPU read.  Bare metal: the GPU write faults, UVM collapses, CPU sees the
//                new value.  kf3 today (READ_ONLY not propagated to the host twin): the
//                write lands in the stale duplicate, CPU reads the OLD value — SILENT.
//   downgrade  : CPU init, prefetch->GPU (RW, GPU-resident), THEN ReadMostly, CPU read
//                (read-dup: the GPU mapping is DOWNGRADED RW->RO in place), GPU write,
//                sync, CPU read.  Same prediction as gpuwrite, reached through a
//                permission downgrade instead of a fresh RO map.
#include <cstdio>
#include <cstring>
#include <cuda_runtime.h>

#define CK(x) do { cudaError_t e_ = (x); if (e_ != cudaSuccess) { \
  printf("CHECK %s FAIL %s -> %d (%s)\n", mode, #x, (int)e_, cudaGetErrorString(e_)); return 2; } } while (0)

__global__ void gsum(const int *x, int n, unsigned long long *out) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) atomicAdd(out, (unsigned long long)x[i]);
}
__global__ void gset(int *x, int n, int v) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) x[i] = v + i;
}

int main(int argc, char **argv) {
  const char *mode = argc > 1 ? argv[1] : "reprefetch";
  const int n = 1 << 20;                  // 4 MiB: two 2 MiB UVM blocks
  const int dev = 0;
  int *x = nullptr; unsigned long long *sum = nullptr;
  CK(cudaMallocManaged(&x, n * sizeof(int)));
  CK(cudaMalloc(&sum, sizeof(*sum)));      // device memory: never a UVM question
  const int B = 256, G = (n + B - 1) / B;
  auto expect_sum = [&](int base) {        // sum of base+i, i in [0,n)
    return (unsigned long long)base * n + (unsigned long long)n * (n - 1) / 2; };
  auto gpu_sum = [&](unsigned long long *h) -> cudaError_t {
    cudaError_t e;
    if ((e = cudaMemset(sum, 0, sizeof(*sum))) != cudaSuccess) return e;
    gsum<<<G, B>>>(x, n, sum);
    if ((e = cudaGetLastError()) != cudaSuccess) return e;
    if ((e = cudaDeviceSynchronize()) != cudaSuccess) return e;
    return cudaMemcpy(h, sum, sizeof(*h), cudaMemcpyDeviceToHost);
  };
  long bad = 0; unsigned long long h = 0;

  if (!strcmp(mode, "downgrade")) {
    for (int i = 0; i < n; i++) x[i] = 1 + i;
    CK(cudaMemPrefetchAsync(x, n * sizeof(int), dev, 0));
    CK(cudaDeviceSynchronize());
    CK(cudaMemAdvise(x, n * sizeof(int), cudaMemAdviseSetReadMostly, dev));
    for (int i = 0; i < n; i++) if (x[i] != 1 + i) bad++;       // CPU read -> read-dup
    gset<<<G, B>>>(x, n, 7);                                      // GPU write to RO dup
    CK(cudaGetLastError()); CK(cudaDeviceSynchronize());
    for (int i = 0; i < n; i++) if (x[i] != 7 + i) bad++;
    printf("CHECK %s %s bad=%ld\n", mode, bad ? "FAIL" : "ok", bad);
    return bad ? 1 : 0;
  }

  CK(cudaMemAdvise(x, n * sizeof(int), cudaMemAdviseSetReadMostly, dev));
  for (int i = 0; i < n; i++) x[i] = 1 + i;                     // CPU first touch
  CK(cudaMemPrefetchAsync(x, n * sizeof(int), dev, 0));          // GPU RO duplicate
  CK(cudaDeviceSynchronize());

  if (!strcmp(mode, "gpuwrite")) {
    gset<<<G, B>>>(x, n, 5);
    CK(cudaGetLastError()); CK(cudaDeviceSynchronize());
    for (int i = 0; i < n; i++) if (x[i] != 5 + i) bad++;
    printf("CHECK %s %s bad=%ld\n", mode, bad ? "FAIL" : "ok", bad);
    return bad ? 1 : 0;
  }

  CK(gpu_sum(&h)); if (h != expect_sum(1)) { printf("GPU read #1 sum=%llu want=%llu\n", h, expect_sum(1)); bad++; }
  for (int i = 0; i < n; i++) if (x[i] != 1 + i) bad++;       // CPU read: duplicate, no fault
  for (int i = 0; i < n; i++) x[i] = 3 + i;                     // CPU write: collapse to CPU
  if (!strcmp(mode, "reprefetch")) {
    CK(cudaMemPrefetchAsync(x, n * sizeof(int), dev, 0));
    CK(cudaDeviceSynchronize());
  }
  CK(gpu_sum(&h)); if (h != expect_sum(3)) { printf("GPU read #2 sum=%llu want=%llu\n", h, expect_sum(3)); bad++; }
  printf("CHECK %s %s bad=%ld\n", mode, bad ? "FAIL" : "ok", bad);
  return bad ? 1 : 0;
}
