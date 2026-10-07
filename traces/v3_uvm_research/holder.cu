// holder — keep the GPU registered with UVM so fault_stats exists across bm_run.sh
// (the node is created on GPU registration and removed when the last UVM user exits).
#include <cstdio>
#include <unistd.h>
#include <cuda_runtime.h>
int main() { void *p; if (cudaMallocManaged(&p, 4096) != cudaSuccess) return 1;
  printf("HOLD_READY\n"); fflush(stdout); pause(); return 0; }
