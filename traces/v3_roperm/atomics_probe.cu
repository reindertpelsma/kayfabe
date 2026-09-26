// atomics_probe — GPU atomics on CPU-RESIDENT managed memory (v3-roperm follow-up).
//
// Why: guest UVM maps a sysmem-resident managed page on a PCIe GPU READ_WRITE with ATOMIC_DISABLE
// (no native GPU->CPU atomics, uvm_va_space.c:862-892), so on bare metal a GPU atomic FAULTS and
// UVM migrates the page. kf3 cannot deliver that fault: if the host twin carries ATOMIC_DISABLE
// the atomic is a host RC (719); if it does not, the atomic executes on the sysmem page, which is
// the authoritative copy. This probe measures which one is CORRECT, host vs guest.
//
// build: nvcc -O2 -arch=sm_86 -cudart static -o atomics_probe atomics_probe.cu
// usage: atomics_probe <mode>        every value checked; prints CHECK <mode> ok|FAIL ...
//   accessedby     managed, CPU first touch, then SetAccessedBy(GPU): GPU atomicAdd (device scope)
//   accessedby_sys as accessedby, atomicAdd_system
//   prefcpu        + SetPreferredLocation(CPU): the page stays in sysmem
//   prefetchcpu    SetAccessedBy(GPU), CPU init, prefetch->GPU, prefetch->CPU, then GPU atomics
//   write          CONTROL: as accessedby but plain stores (no atomics) — the mapping itself works
//   devmem         CONTROL: the same atomics on cudaMalloc memory (vidmem, never a UVM question)
#include <cstdio>
#include <cstring>
#include <cuda_runtime.h>

#define CK(x) do { cudaError_t e_ = (x); if (e_ != cudaSuccess) { \
  printf("CHECK %s FAIL %s -> %d (%s)\n", mode, #x, (int)e_, cudaGetErrorString(e_)); return 2; } } while (0)

static const int N = 1 << 20;        // 4 MiB of ints: two 2 MiB UVM blocks
static const int REP = 4;            // each element is incremented by REP different threads
// x[0..N) elements, x[N] a counter every thread hits (full contention)

__global__ void k_atomic(int *x, int n, int total) {
  int t = blockIdx.x * blockDim.x + threadIdx.x;
  if (t < total) { atomicAdd(&x[t % n], 1); atomicAdd(&x[n], 1); }
}
__global__ void k_atomic_sys(int *x, int n, int total) {
  int t = blockIdx.x * blockDim.x + threadIdx.x;
  if (t < total) { atomicAdd_system(&x[t % n], 1); atomicAdd_system(&x[n], 1); }
}
__global__ void k_store(int *x, int n) {
  int t = blockIdx.x * blockDim.x + threadIdx.x;
  if (t < n) x[t] = x[t] + REP;          // one writer per element: the non-atomic control
  if (t == 0) x[n] = n * REP;
}

int main(int argc, char **argv) {
  const char *mode = argc > 1 ? argv[1] : "accessedby";
  const int dev = 0;
  const size_t bytes = (size_t)(N + 1) * sizeof(int);
  const int total = N * REP, B = 256, G = (total + B - 1) / B;
  int *x = nullptr;
  const bool devmem = !strcmp(mode, "devmem");
  int *h = (int *)malloc(bytes);
  for (int i = 0; i < N; i++) h[i] = 7 * i;
  h[N] = 0;
  if (devmem) {
    CK(cudaMalloc(&x, bytes));
    CK(cudaMemcpy(x, h, bytes, cudaMemcpyHostToDevice));
  } else {
    CK(cudaMallocManaged(&x, bytes));
    memcpy(x, h, bytes);                                   // CPU first touch: sysmem-resident
    // ⊘ After the first touch, as um_probe's passing `advise` shape does: the advice then MAPS
    // the populated sysmem pages on the GPU (no demand fault needed to reach them).
    if (!strcmp(mode, "prefcpu")) CK(cudaMemAdvise(x, bytes, cudaMemAdviseSetPreferredLocation, cudaCpuDeviceId));
    CK(cudaMemAdvise(x, bytes, cudaMemAdviseSetAccessedBy, dev));
    if (!strcmp(mode, "prefetchcpu")) {
      CK(cudaMemPrefetchAsync(x, bytes, dev, 0));
      CK(cudaDeviceSynchronize());
      CK(cudaMemPrefetchAsync(x, bytes, cudaCpuDeviceId, 0)); // back to sysmem, GPU still mapped
      CK(cudaDeviceSynchronize());
    }
  }
  if (!strcmp(mode, "write")) k_store<<<(N + B - 1) / B, B>>>(x, N);
  else if (!strcmp(mode, "accessedby_sys")) k_atomic_sys<<<G, B>>>(x, N, total);
  else k_atomic<<<G, B>>>(x, N, total);
  CK(cudaGetLastError());
  CK(cudaDeviceSynchronize());
  const int *r = x;
  if (devmem) { CK(cudaMemcpy(h, x, bytes, cudaMemcpyDeviceToHost)); r = h; }
  long bad = 0; int first = -1;
  for (int i = 0; i < N; i++) if (r[i] != 7 * i + REP) { if (first < 0) first = i; bad++; }
  const int want_ctr = N * REP;
  if (r[N] != want_ctr) bad++;
  printf("CHECK %s %s bad=%ld counter=%d want=%d first_bad=%d(%d vs %d)\n", mode, bad ? "FAIL" : "ok", bad,
         r[N], want_ctr, first, first >= 0 ? r[first] : 0, first >= 0 ? 7 * first + REP : 0);
  return bad ? 1 : 0;
}
