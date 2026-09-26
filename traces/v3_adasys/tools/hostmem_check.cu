// hostmem_check.cu — STOCK CUDA only: is GPU <-> system-memory traffic correct on this box?
//
// Every arm verifies BOTH directions independently (the GPU checks what it read, the CPU checks
// what the GPU wrote), so a wrong byte is attributed to a direction, never inferred.
//   pageable        malloc + cudaMemcpy (driver staging)
//   pinned          cudaMallocHost + cudaMemcpyAsync (CE <-> RM-allocated sysmem)
//   zerocopy        cudaHostAlloc(Mapped): a KERNEL reads and writes host memory directly
//   wc_zerocopy     cudaHostAlloc(Mapped|WriteCombined): the non-coherent aperture
//   reg_malloc      cudaHostRegister(malloc)   : OS-descriptor sysmem, CE + kernel
//   reg_memfd       cudaHostRegister(mmap MAP_SHARED of a memfd) : kayfabe's exact backing
//   sem_writevalue  cuStreamWriteValue32 into registered memfd memory, CPU polls (no sync)
//   sem_kernelflag  kernel writes a flag to mapped memory + __threadfence_system, CPU polls
//                   while the kernel is still running (no sync)
// Output: one "CHECK <name> PASS|FAIL <detail>" line per check, then HOSTMEM_VERDICT=.
#include <cuda.h>
#include <cuda_runtime.h>
#include <errno.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

static int g_fail = 0, g_pass = 0;
static void check(const char *name, bool ok, const char *fmt, ...) {
  char buf[512];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(buf, sizeof buf, fmt, ap);
  va_end(ap);
  printf("CHECK %-34s %s %s\n", name, ok ? "PASS" : "FAIL", buf);
  fflush(stdout);
  if (ok) g_pass++; else g_fail++;
}
#define CK(x)                                                                                   \
  do {                                                                                          \
    cudaError_t e_ = (x);                                                                       \
    if (e_ != cudaSuccess) {                                                                    \
      check(#x, false, "%s (%s:%d)", cudaGetErrorString(e_), __FILE__, __LINE__);               \
      return;                                                                                   \
    }                                                                                           \
  } while (0)
#define CKD(x)                                                                                  \
  do {                                                                                          \
    CUresult r_ = (x);                                                                          \
    if (r_ != CUDA_SUCCESS) {                                                                   \
      const char *s_ = "?";                                                                     \
      cuGetErrorString(r_, &s_);                                                                \
      check(#x, false, "%s (%s:%d)", s_, __FILE__, __LINE__);                                   \
      return;                                                                                   \
    }                                                                                           \
  } while (0)

static const size_t BYTES = 64ull << 20;  // 16384 host pages per buffer
static const size_t WORDS = BYTES / 4;

static inline uint32_t pat(uint32_t seed, size_t i) { return seed ^ (uint32_t)(i * 2654435761u); }
static void fill(uint32_t *p, uint32_t seed) { for (size_t i = 0; i < WORDS; i++) p[i] = pat(seed, i); }
static size_t bad_words(const volatile uint32_t *p, uint32_t seed, size_t *first, uint32_t *got) {
  size_t bad = 0;
  *first = (size_t)-1;
  for (size_t i = 0; i < WORDS; i++) {
    uint32_t v = p[i];
    if (v != pat(seed, i)) {
      if (!bad) { *first = i; *got = v; }
      bad++;
    }
  }
  return bad;
}

// GPU side: count words of `src` that differ from the pattern, and write pattern `wseed` to `dst`.
__global__ void verify_and_write(const volatile uint32_t *src, uint32_t rseed, uint32_t *dst, uint32_t wseed,
                                 size_t n, unsigned long long *bad) {
  size_t stride = (size_t)gridDim.x * blockDim.x;
  unsigned long long local = 0;
  for (size_t i = (size_t)blockIdx.x * blockDim.x + threadIdx.x; i < n; i += stride) {
    if (src && src[i] != (rseed ^ (uint32_t)(i * 2654435761u))) local++;
    if (dst) dst[i] = wseed ^ (uint32_t)(i * 2654435761u);
  }
  if (local) atomicAdd(bad, local);
}

__global__ void set_flag_then_spin(volatile uint32_t *flag, uint32_t v, volatile uint32_t *release) {
  if (threadIdx.x == 0 && blockIdx.x == 0) {
    *flag = v;
    __threadfence_system();
    // Keep the kernel alive until the CPU acknowledges (bounded by the CPU side's timeout).
    long long t0 = clock64();
    while (*release == 0 && clock64() - t0 < 20000000000ll) {}
  }
}

static unsigned long long *d_bad;
static uint32_t *d_buf, *d_buf2;

static void gpu_check_and_write(const char *name, const uint32_t *src_dev, uint32_t rseed, uint32_t *dst_dev,
                                uint32_t wseed, cudaStream_t s) {
  CK(cudaMemsetAsync(d_bad, 0, sizeof *d_bad, s));
  verify_and_write<<<512, 256, 0, s>>>(src_dev, rseed, dst_dev, wseed, WORDS, d_bad);
  CK(cudaGetLastError());
  unsigned long long bad = 0;
  CK(cudaMemcpyAsync(&bad, d_bad, sizeof bad, cudaMemcpyDeviceToHost, s));
  CK(cudaStreamSynchronize(s));
  if (src_dev) check(name, bad == 0, "GPU read %llu wrong words of %zu", bad, WORDS);
}

static void cpu_check(const char *name, const volatile uint32_t *p, uint32_t seed) {
  size_t first; uint32_t got = 0;
  size_t bad = bad_words(p, seed, &first, &got);
  if (bad) check(name, false, "CPU read %zu wrong words of %zu; first at word %zu = %#x want %#x", bad, WORDS, first, got, pat(seed, first));
  else check(name, true, "CPU read 0 wrong words of %zu", WORDS);
}

// CE both ways between `h` (host) and device memory, the GPU checking the H2D result on the device.
static void ce_roundtrip(const char *tag, uint32_t *h, cudaStream_t s, bool async) {
  char n1[64], n2[64];
  snprintf(n1, sizeof n1, "%s_h2d_ce", tag);
  snprintf(n2, sizeof n2, "%s_d2h_ce", tag);
  fill(h, 0x11110000u ^ (uint32_t)(uintptr_t)tag);
  if (async) CK(cudaMemcpyAsync(d_buf, h, BYTES, cudaMemcpyHostToDevice, s));
  else CK(cudaMemcpy(d_buf, h, BYTES, cudaMemcpyHostToDevice));
  gpu_check_and_write(n1, d_buf, 0x11110000u ^ (uint32_t)(uintptr_t)tag, d_buf2, 0x22220000u, s);
  memset(h, 0, BYTES);
  if (async) { CK(cudaMemcpyAsync(h, d_buf2, BYTES, cudaMemcpyDeviceToHost, s)); CK(cudaStreamSynchronize(s)); }
  else CK(cudaMemcpy(h, d_buf2, BYTES, cudaMemcpyDeviceToHost));
  cpu_check(n2, h, 0x22220000u);
}

// A kernel reading and writing host memory through its device pointer (no copy engine).
static void kernel_direct(const char *tag, uint32_t *h_src, uint32_t *h_dst, cudaStream_t s) {
  char n1[64], n2[64];
  snprintf(n1, sizeof n1, "%s_kernel_reads_host", tag);
  snprintf(n2, sizeof n2, "%s_kernel_writes_host", tag);
  uint32_t *dsrc = nullptr, *ddst = nullptr;
  CK(cudaHostGetDevicePointer((void **)&dsrc, h_src, 0));
  CK(cudaHostGetDevicePointer((void **)&ddst, h_dst, 0));
  fill(h_src, 0x33330000u);
  memset(h_dst, 0, BYTES);
  gpu_check_and_write(n1, dsrc, 0x33330000u, ddst, 0x44440000u, s);
  cpu_check(n2, h_dst, 0x44440000u);
}

static void *memfd_map(size_t bytes, int *fd_out) {
  int fd = (int)syscall(SYS_memfd_create, "hostmem_check", 0u);
  if (fd < 0 || ftruncate(fd, (off_t)bytes) != 0) return nullptr;
  void *p = mmap(nullptr, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  *fd_out = fd;
  return p == MAP_FAILED ? nullptr : p;
}

static double now_us() {
  struct timespec t;
  clock_gettime(CLOCK_MONOTONIC, &t);
  return t.tv_sec * 1e6 + t.tv_nsec / 1e3;
}

static void arm_pageable(cudaStream_t s) {
  uint32_t *h = (uint32_t *)malloc(BYTES);
  ce_roundtrip("pageable", h, s, false);
  free(h);
}
static void arm_pinned(cudaStream_t s) {
  uint32_t *h;
  CK(cudaMallocHost((void **)&h, BYTES));
  ce_roundtrip("pinned", h, s, true);
  CK(cudaFreeHost(h));
}
static void arm_zerocopy(cudaStream_t s, bool wc) {
  uint32_t *a, *b;
  unsigned f = cudaHostAllocMapped | (wc ? cudaHostAllocWriteCombined : 0);
  CK(cudaHostAlloc((void **)&a, BYTES, f));
  CK(cudaHostAlloc((void **)&b, BYTES, cudaHostAllocMapped));
  kernel_direct(wc ? "wc_zerocopy" : "zerocopy", a, b, s);
  if (wc) ce_roundtrip("wc_pinned", a, s, true);
  CK(cudaFreeHost(a));
  CK(cudaFreeHost(b));
}
static void arm_registered(cudaStream_t s, bool memfd) {
  const char *tag = memfd ? "reg_memfd" : "reg_malloc";
  int fa = -1, fb = -1;
  uint32_t *a, *b;
  if (memfd) {
    a = (uint32_t *)memfd_map(BYTES, &fa);
    b = (uint32_t *)memfd_map(BYTES, &fb);
    if (!a || !b) { check(tag, false, "memfd/mmap: %s", strerror(errno)); return; }
  } else {
    a = (uint32_t *)aligned_alloc(4096, BYTES);
    b = (uint32_t *)aligned_alloc(4096, BYTES);
  }
  memset(a, 0, BYTES);  // populate the pages before pinning (as kayfabe's view is populated)
  memset(b, 0, BYTES);
  CK(cudaHostRegister(a, BYTES, cudaHostRegisterMapped));
  CK(cudaHostRegister(b, BYTES, cudaHostRegisterMapped));
  ce_roundtrip(tag, a, s, true);
  kernel_direct(tag, a, b, s);
  CK(cudaHostUnregister(a));
  CK(cudaHostUnregister(b));
  if (memfd) { munmap(a, BYTES); munmap(b, BYTES); close(fa); close(fb); }
  else { free(a); free(b); }
}

// A semaphore RELEASE into system memory, polled by the CPU with no synchronize — the shape of the
// failing kayfabe check (a release semaphore in guest RAM read 0).
static void arm_semaphores(cudaStream_t s) {
  int fd = -1;
  const size_t PAGE = 4096;
  volatile uint32_t *sem = (volatile uint32_t *)memfd_map(PAGE * 4, &fd);
  if (!sem) { check("sem_memfd", false, "memfd: %s", strerror(errno)); return; }
  memset((void *)sem, 0, PAGE * 4);
  CK(cudaHostRegister((void *)sem, PAGE * 4, cudaHostRegisterMapped));
  CUdeviceptr dsem;
  CKD(cuMemHostGetDevicePointer(&dsem, (void *)sem, 0));
  int ops = 0;
  CUdevice dev;
  CKD(cuCtxGetDevice(&dev));
  cuDeviceGetAttribute(&ops, CU_DEVICE_ATTRIBUTE_CAN_USE_STREAM_MEM_OPS_V1, dev);
  // 1) cuStreamWriteValue32 (a host-engine semaphore release) x 64 values, each polled by the CPU.
  int ok = 0, seen_before_sync = 0;
  double worst = 0;
  for (uint32_t k = 1; k <= 64; k++) {
    uint32_t v = 0x5E4A0000u | k;
    CKD(cuStreamWriteValue32((CUstream)s, dsem + 0xff0, v, CU_STREAM_WRITE_VALUE_DEFAULT));
    double t0 = now_us();
    while (sem[0xff0 / 4] != v && now_us() - t0 < 2e6) {}
    double dt = now_us() - t0;
    if (sem[0xff0 / 4] == v) { seen_before_sync++; if (dt > worst) worst = dt; }
    CK(cudaStreamSynchronize(s));
    if (sem[0xff0 / 4] == v) ok++;
  }
  check("sem_writevalue_seen_by_cpu_polling", seen_before_sync == 64, "%d/64 seen before any sync (worst %.0f us); mem_ops_v1_attr=%d", seen_before_sync, worst, ops);
  check("sem_writevalue_after_sync", ok == 64, "%d/64 correct after cudaStreamSynchronize", ok);
  // 2) A kernel flag write + system fence, polled while the kernel is still running.
  volatile uint32_t *flag = sem + PAGE / 4, *release = sem + 2 * PAGE / 4;
  *flag = 0; *release = 0;
  set_flag_then_spin<<<1, 32, 0, s>>>((volatile uint32_t *)(dsem + PAGE), 0xF1A6F1A6u, (volatile uint32_t *)(dsem + 2 * PAGE));
  CK(cudaGetLastError());
  double t0 = now_us();
  while (*flag != 0xF1A6F1A6u && now_us() - t0 < 5e6) {}
  double dt = now_us() - t0;
  bool seen = *flag == 0xF1A6F1A6u;
  *release = 1;
  __sync_synchronize();
  CK(cudaStreamSynchronize(s));
  check("sem_kernelflag_seen_while_running", seen, "flag=%#x after %.0f us of polling (kernel still resident)", *flag, dt);
  CK(cudaHostUnregister((void *)sem));
  munmap((void *)sem, PAGE * 4);
  close(fd);
}

int main() {
  cudaDeviceProp p;
  if (cudaGetDeviceProperties(&p, 0) != cudaSuccess) { printf("HOSTMEM_VERDICT=FAIL (no device)\n"); return 1; }
  int drv = 0, rt = 0;
  cudaDriverGetVersion(&drv);
  cudaRuntimeGetVersion(&rt);
  printf("HOSTMEM_START device=\"%s\" sm_%d%d pci=%04x:%02x:%02x cuda_driver=%d runtime=%d canMapHostMemory=%d pageableMemoryAccess=%d hostRegisterSupported=%d\n",
         p.name, p.major, p.minor, p.pciDomainID, p.pciBusID, p.pciDeviceID, drv, rt, p.canMapHostMemory,
         p.pageableMemoryAccess, p.hostRegisterSupported);
  cudaSetDeviceFlags(cudaDeviceMapHost);
  cudaStream_t s;
  if (cudaStreamCreateWithFlags(&s, cudaStreamNonBlocking) != cudaSuccess) { printf("HOSTMEM_VERDICT=FAIL (stream)\n"); return 1; }
  if (cudaMalloc(&d_bad, sizeof *d_bad) != cudaSuccess || cudaMalloc(&d_buf, BYTES) != cudaSuccess ||
      cudaMalloc(&d_buf2, BYTES) != cudaSuccess) { printf("HOSTMEM_VERDICT=FAIL (cudaMalloc)\n"); return 1; }
  arm_pageable(s);
  arm_pinned(s);
  arm_zerocopy(s, false);
  arm_zerocopy(s, true);
  arm_registered(s, false);
  arm_registered(s, true);
  arm_semaphores(s);
  cudaError_t e = cudaDeviceSynchronize();
  check("device_healthy_at_end", e == cudaSuccess, "%s", cudaGetErrorString(e));
  printf("HOSTMEM_SUMMARY pass=%d fail=%d\n", g_pass, g_fail);
  printf("HOSTMEM_VERDICT=%s\n", g_fail ? "FAIL" : "PASS");
  return g_fail ? 1 : 0;
}
