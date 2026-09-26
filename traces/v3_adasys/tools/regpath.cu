// regpath.cu — which driver path does cudaHostRegister take? Marks bracket the phase for strace.
#include <cuda_runtime.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>
static void mark(const char *m) { syscall(SYS_write, 2, m, strlen(m)); }
int main() {
  const size_t B = 4 << 20;
  void *d; cudaMalloc(&d, B); cudaDeviceSynchronize();
  int fd = (int)syscall(SYS_memfd_create, "regpath", 0u); ftruncate(fd, B);
  void *h = mmap(nullptr, B, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0); memset(h, 0x5a, B);
  mark("MARK_REGISTER_BEGIN\n");
  cudaError_t e = cudaHostRegister(h, B, cudaHostRegisterMapped);
  mark("MARK_REGISTER_END\n");
  cudaMemcpy(d, h, B, cudaMemcpyHostToDevice);
  mark("MARK_COPY_END\n");
  printf("register=%s\n", cudaGetErrorString(e));
  return 0;
}
