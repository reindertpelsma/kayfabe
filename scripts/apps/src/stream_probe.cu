// stream_probe.cu <mode> — one kernel launch + verify, on the stream shape named by <mode>:
//   default     the legacy default stream (0)
//   created     a cudaStreamCreate'd stream
//   nonblocking a cudaStreamNonBlocking stream
//   perthread   cudaStreamPerThread
//   two         two created streams, one launch each
//   created2nd  default stream first (warm), then a created stream
// Written for the V3 app matrix: separates "a second stream" from "a kernel" as the failing shape.
#include <cstdio>
#include <cstring>
#include <cuda_runtime.h>
__global__ void k(int *p, int n, int m) { int i = blockIdx.x * blockDim.x + threadIdx.x; if (i < n) p[i] = i * m; }
static int run(cudaStream_t s, int m, const char *tag) {
  const int n = 1 << 20; int *d, *h = new int[n];
  if (cudaMalloc(&d, n * 4)) { printf("CHECK %s FAIL malloc\n", tag); return 1; }
  k<<<(n + 255) / 256, 256, 0, s>>>(d, n, m);
  cudaError_t e = cudaStreamSynchronize(s);
  if (e) { printf("CHECK %s FAIL sync=%d(%s)\n", tag, (int)e, cudaGetErrorString(e)); return 1; }
  cudaMemcpy(h, d, n * 4, cudaMemcpyDeviceToHost);
  for (int i = 0; i < n; i++) if (h[i] != i * m) { printf("CHECK %s FAIL value[%d]=%d\n", tag, i, h[i]); return 1; }
  printf("CHECK %s ok\n", tag); cudaFree(d); delete[] h; return 0;
}
int main(int c, char **v) {
  const char *m = c > 1 ? v[1] : "default"; cudaStream_t s1, s2; int r = 0;
  cudaFree(0);
  if (!strcmp(m, "default")) r = run(0, 2, m);
  else if (!strcmp(m, "created")) { cudaStreamCreate(&s1); r = run(s1, 3, m); }
  else if (!strcmp(m, "nonblocking")) { cudaStreamCreateWithFlags(&s1, cudaStreamNonBlocking); r = run(s1, 4, m); }
  else if (!strcmp(m, "perthread")) r = run(cudaStreamPerThread, 5, m);
  else if (!strcmp(m, "two")) { cudaStreamCreate(&s1); cudaStreamCreate(&s2); r = run(s1, 6, "two_a") | run(s2, 7, "two_b"); }
  else if (!strcmp(m, "created2nd")) { r = run(0, 8, "c2_default"); cudaStreamCreate(&s1); r |= run(s1, 9, "c2_created"); }
  printf("STREAM_PROBE_DONE %s rc=%d\n", m, r); return r;
}
