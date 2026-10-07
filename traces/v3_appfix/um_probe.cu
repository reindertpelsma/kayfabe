// um_probe — managed-memory shapes, each fully checked (every CUDA status, every value).
// usage: um_probe <mode>   mode = attrs | cpuinit | prefetch | gpufirst | advise | malloc
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cuda_runtime.h>

#define CK(x) do { cudaError_t e_ = (x); if (e_ != cudaSuccess) { \
  printf("CHECK %s FAIL %s -> %d (%s)\n", mode, #x, (int)e_, cudaGetErrorString(e_)); return 2; } } while (0)

__global__ void axpy(float *y, const float *x, float a, int n) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) y[i] = a * x[i] + y[i];
}
__global__ void fill(float *y, float v, int n) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) y[i] = v + i;
}

int main(int argc, char **argv) {
  const char *mode = argc > 1 ? argv[1] : "cpuinit";
  int dev = 0, cma = -1, pma = -1, mm = -1;
  CK(cudaDeviceGetAttribute(&cma, cudaDevAttrConcurrentManagedAccess, dev));
  CK(cudaDeviceGetAttribute(&pma, cudaDevAttrPageableMemoryAccess, dev));
  CK(cudaDeviceGetAttribute(&mm, cudaDevAttrManagedMemory, dev));
  printf("ATTR managed=%d concurrentManagedAccess=%d pageableMemoryAccess=%d\n", mm, cma, pma);
  if (!strcmp(mode, "attrs")) return 0;
  const int n = 1 << 21;  // 8 MiB per buffer (values stay exact in fp32)
  float *x = nullptr, *y = nullptr;
  if (!strcmp(mode, "pageable") || !strcmp(mode, "hostalloc") || !strcmp(mode, "d2h")) {
    // pageable: a kernel writes plain malloc memory (needs pageableMemoryAccess / HMM);
    // hostalloc: a kernel writes cudaHostAlloc'd pinned memory; d2h: cudaMemcpy into malloc memory.
    float *h = nullptr, *d = nullptr;
    if (!strcmp(mode, "hostalloc")) CK(cudaHostAlloc(&h, n * sizeof(float), cudaHostAllocDefault));
    else h = (float *)malloc(n * sizeof(float));
    for (int i = 0; i < n; i++) h[i] = -1.0f;
    if (!strcmp(mode, "d2h")) {
      CK(cudaMalloc(&d, n * sizeof(float)));
      fill<<<(n + 255) / 256, 256>>>(d, 1.0f, n);
      CK(cudaMemcpy(h, d, n * sizeof(float), cudaMemcpyDeviceToHost));
    } else {
      fill<<<(n + 255) / 256, 256>>>(h, 1.0f, n);
    }
    CK(cudaGetLastError());
    CK(cudaDeviceSynchronize());
    long badp = 0;
    for (int i = 0; i < n; i++) if (h[i] != 1.0f + i) badp++;
    printf("CHECK %s %s bad=%ld\n", mode, badp ? "FAIL" : "ok", badp);
    return badp ? 1 : 0;
  }
  if (!strcmp(mode, "malloc")) {
    CK(cudaMalloc(&x, n * sizeof(float)));
    CK(cudaMalloc(&y, n * sizeof(float)));
    fill<<<(n + 255) / 256, 256>>>(x, 1.0f, n);
    fill<<<(n + 255) / 256, 256>>>(y, 2.0f, n);
  } else {
    CK(cudaMallocManaged(&x, n * sizeof(float)));
    CK(cudaMallocManaged(&y, n * sizeof(float)));
    if (!strcmp(mode, "gpufirst")) {
      fill<<<(n + 255) / 256, 256>>>(x, 1.0f, n);
      fill<<<(n + 255) / 256, 256>>>(y, 2.0f, n);
    } else {
      for (int i = 0; i < n; i++) { x[i] = 1.0f + i; y[i] = 2.0f + i; }
      if (!strcmp(mode, "prefetch")) {
        CK(cudaMemPrefetchAsync(x, n * sizeof(float), dev, 0));
        CK(cudaMemPrefetchAsync(y, n * sizeof(float), dev, 0));
      } else if (!strcmp(mode, "advise")) {
        CK(cudaMemAdvise(x, n * sizeof(float), cudaMemAdviseSetAccessedBy, dev));
        CK(cudaMemAdvise(y, n * sizeof(float), cudaMemAdviseSetAccessedBy, dev));
      }
    }
  }
  axpy<<<(n + 255) / 256, 256>>>(y, x, 3.0f, n);
  CK(cudaGetLastError());
  CK(cudaDeviceSynchronize());
  long bad = 0;
  if (!strcmp(mode, "malloc")) {
    float *h = (float *)malloc(n * sizeof(float));
    CK(cudaMemcpy(h, y, n * sizeof(float), cudaMemcpyDeviceToHost));
    for (int i = 0; i < n; i++) if (h[i] != 3.0f * (1.0f + i) + (2.0f + i)) bad++;
    free(h);
  } else {
    for (int i = 0; i < n; i++) if (y[i] != 3.0f * (1.0f + i) + (2.0f + i)) bad++;
  }
  printf("CHECK %s %s bad=%ld\n", mode, bad ? "FAIL" : "ok", bad);
  return bad ? 1 : 0;
}
