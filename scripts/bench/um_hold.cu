// um_hold — a guest CUDA process that keeps a managed-memory context (and so kf3's EFS twin space)
// alive for N seconds after touching it from the GPU: the window the E-S1 negative control runs in.
// usage: um_hold <seconds>
#include <cstdio>
#include <cstdlib>
#include <unistd.h>
#include <cuda_runtime.h>
__global__ void fill(float *y, int n) { int i = blockIdx.x * blockDim.x + threadIdx.x; if (i < n) y[i] = i; }
int main(int argc, char **argv) {
  int secs = argc > 1 ? atoi(argv[1]) : 20;
  const int n = 1 << 20;
  float *x = nullptr;
  if (cudaMallocManaged(&x, n * sizeof(float)) != cudaSuccess) { printf("HOLD FAIL alloc\n"); return 2; }
  fill<<<(n + 255) / 256, 256>>>(x, n);
  if (cudaDeviceSynchronize() != cudaSuccess) { printf("HOLD FAIL sync\n"); return 2; }
  long bad = 0; for (int i = 0; i < n; i++) if (x[i] != (float)i) bad++;
  printf("HOLD READY bad=%ld pid=%d\n", bad, getpid()); fflush(stdout);
  sleep(secs);
  printf("HOLD DONE\n");
  return bad ? 1 : 0;
}
