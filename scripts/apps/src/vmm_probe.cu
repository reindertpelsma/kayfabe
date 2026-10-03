// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// vmm_probe — the CUDA virtual-memory-management API (cuMemAddressReserve / cuMemCreate / cuMemMap /
// cuMemSetAccess), every shape checked BY VALUE, for the V3 app matrix (release §I: PyTorch's
// expandable segments and vLLM rely on this API, and no other row used it).
// Ported from nvkvm's `tests/mode2/nvdiff/nvd_apis.c` stage `vmm` (run_vmm), with the checks
// docs/design/V3_APP_MATRIX.md §R5 asks for. One line per check, in order:
//   CHECK vmm_supported   the device reports the VMM API
//   CHECK map             reserve + create + map + RW access; a kernel writes, the CPU verifies
//   CHECK alias           the SAME handle at a second VA: written through VA1, read through VA2
//                         (copy engine and a kernel), then written through VA2, read through VA1
//   CHECK remap_same_va   VA1 unmapped and its handle released while VA2 keeps the backing alive; a
//                         FRESH handle mapped at VA1 and written — VA1 must read the new data and
//                         VA2 the old (a stale VA1 leaf would write into VA2's pages)
//   CHECK ro_map          a PROT_READ mapping reads correctly (copy engine and a kernel)
//   CHECK fd_import       a POSIX-fd exportable handle, exported and imported in this process, maps
//                         the same physical memory
//   CHECK teardown        every unmap / release / address free returns success
//   CHECK ro_write        a kernel WRITE through a PROT_READ mapping, in a child process, must FAIL
//                         (bare metal faults it; a host map that dropped the bit would let it land)
//   VMM_PROBE_DONE checks=<n> failed=<m>
// Exit 0 iff every check is ok. ⚠ ro_write faults ON PURPOSE: one Xid 31 per run is expected on both
// the host and the guest (in a kf3 guest: an RC, 719 in the child, a guest Xid naming kayfabe).
// usage: vmm_probe   (vmm_probe ro_write_child is the child's own entry point)
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include <signal.h>
#include <sys/wait.h>
#include <unistd.h>
#include <cuda.h>
#include <cuda_runtime.h>

static const unsigned SEED_A = 0xA11CE001u, SEED_A2 = 0xA11CE002u, SEED_B = 0xB0B0B0B0u,
                      SEED_C = 0xC0FFEE00u, SEED_D = 0xD00D0001u, SEED_E = 0xE0E0E0E0u;

__host__ __device__ inline unsigned pat(size_t i, unsigned seed) { return seed ^ ((unsigned)i * 2654435761u); }

__global__ void fill(unsigned *p, size_t n, unsigned seed) {
  for (size_t i = blockIdx.x * (size_t)blockDim.x + threadIdx.x; i < n; i += (size_t)gridDim.x * blockDim.x)
    p[i] = pat(i, seed);
}
__global__ void copyk(unsigned *dst, const unsigned *src, size_t n) {
  for (size_t i = blockIdx.x * (size_t)blockDim.x + threadIdx.x; i < n; i += (size_t)gridDim.x * blockDim.x)
    dst[i] = src[i];
}

static char why[512];
static int checks = 0, failed = 0;
static void say(const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); vsnprintf(why, sizeof why, fmt, ap); va_end(ap);
}
static bool check(const char *name, bool ok) {
  checks++; if (!ok) failed++;
  printf("CHECK %s %s %s\n", name, ok ? "ok" : "FAIL", why);
  fflush(stdout); why[0] = 0;
  return ok;
}
#define CU(x) do { CUresult r_ = (x); if (r_ != CUDA_SUCCESS) { const char *n_ = nullptr; cuGetErrorName(r_, &n_); \
  say("%s -> %d (%s)", #x, (int)r_, n_ ? n_ : "?"); return false; } } while (0)
#define RT(x) do { cudaError_t e_ = (x); if (e_ != cudaSuccess) { \
  say("%s -> %d (%s)", #x, (int)e_, cudaGetErrorString(e_)); return false; } } while (0)

static CUdevice dev;
static size_t sz = 0, n = 0;  // bytes per allocation, and its 32-bit words
static CUmemAllocationProp prop;
static CUmemAccessDesc rw, ro;
static std::vector<unsigned> host;
static unsigned *chk = nullptr;  // a cudaMalloc'd buffer a kernel copies into
static CUdeviceptr base = 0, va1 = 0, va2 = 0, va3 = 0;
static CUmemGenericAllocationHandle h1 = 0, h2 = 0;
static bool m1 = false, m2 = false, m3 = false, h1live = false, h2live = false;

static unsigned *P(CUdeviceptr v) { return (unsigned *)(uintptr_t)v; }
static bool launch_fill(CUdeviceptr v, unsigned seed) {
  fill<<<256, 256>>>(P(v), n, seed);
  RT(cudaGetLastError());
  RT(cudaDeviceSynchronize());
  return true;
}
// Read `sz` bytes at `v` through the copy engine and count words that are not pat(i, seed).
static bool verify(CUdeviceptr v, unsigned seed, const char *what) {
  CU(cuMemcpyDtoH(host.data(), v, sz));
  long bad = 0; size_t first = 0;
  for (size_t i = 0; i < n; i++) if (host[i] != pat(i, seed)) { if (!bad) first = i; bad++; }
  if (bad) { say("%s: %ld of %zu words wrong (first [%zu] = 0x%08x, want 0x%08x)", what, bad, n, first,
                 host[first], pat(first, seed)); return false; }
  return true;
}
// The same, through the GR engine: a kernel copies `v` into `chk`, then the copy engine reads `chk`.
static bool verify_by_kernel(CUdeviceptr v, unsigned seed, const char *what) {
  copyk<<<256, 256>>>(chk, P(v), n);
  RT(cudaGetLastError());
  RT(cudaDeviceSynchronize());
  return verify((CUdeviceptr)(uintptr_t)chk, seed, what);
}

static bool init_prop() {
  memset(&prop, 0, sizeof prop);
  prop.type = CU_MEM_ALLOCATION_TYPE_PINNED;
  prop.requestedHandleTypes = CU_MEM_HANDLE_TYPE_NONE;
  prop.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
  prop.location.id = (int)dev;
  memset(&rw, 0, sizeof rw);
  rw.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
  rw.location.id = (int)dev;
  rw.flags = CU_MEM_ACCESS_FLAGS_PROT_READWRITE;
  ro = rw;
  ro.flags = CU_MEM_ACCESS_FLAGS_PROT_READ;
  size_t gran = 0;
  CU(cuMemGetAllocationGranularity(&gran, &prop, CU_MEM_ALLOC_GRANULARITY_MINIMUM));
  // 4 MiB rounded up to the device's granularity — cuMemCreate refuses anything else, and a refusal
  // there would measure our arithmetic, not the device.
  sz = gran ? (((size_t)(4u << 20) + gran - 1) / gran) * gran : (size_t)(4u << 20);
  n = sz / sizeof(unsigned);
  host.assign(n, 0);
  return true;
}

static bool step_supported() {
  int vmm = 0, fd = 0;
  RT(cudaSetDevice(0));
  RT(cudaFree(0));  // the runtime's primary context, current on this thread for the driver calls too
  CU(cuDeviceGet(&dev, 0));
  CU(cuDeviceGetAttribute(&vmm, CU_DEVICE_ATTRIBUTE_VIRTUAL_MEMORY_MANAGEMENT_SUPPORTED, dev));
  CU(cuDeviceGetAttribute(&fd, CU_DEVICE_ATTRIBUTE_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR_SUPPORTED, dev));
  if (!init_prop()) return false;
  say("vmm=%d posix_fd=%d size=%zu", vmm, fd, sz);
  return vmm == 1;
}

static bool step_map() {
  RT(cudaMalloc((void **)&chk, sz));
  CU(cuMemAddressReserve(&base, 3 * sz, 0, 0, 0));
  va1 = base; va2 = base + sz; va3 = base + 2 * sz;
  CU(cuMemCreate(&h1, sz, &prop, 0)); h1live = true;
  CU(cuMemMap(va1, sz, 0, h1, 0)); m1 = true;
  CU(cuMemSetAccess(va1, sz, &rw, 1));
  if (!launch_fill(va1, SEED_A)) return false;
  return verify(va1, SEED_A, "VA1 after a kernel wrote it");
}

static bool step_alias() {
  CU(cuMemMap(va2, sz, 0, h1, 0)); m2 = true;
  CU(cuMemSetAccess(va2, sz, &rw, 1));
  if (!verify(va2, SEED_A, "VA2 (copy engine) after a kernel wrote VA1")) return false;
  if (!verify_by_kernel(va2, SEED_A, "VA2 (kernel) after a kernel wrote VA1")) return false;
  if (!launch_fill(va2, SEED_A2)) return false;
  return verify(va1, SEED_A2, "VA1 after a kernel wrote VA2");
}

static bool step_remap() {
  CU(cuMemUnmap(va1, sz)); m1 = false;
  CU(cuMemRelease(h1)); h1live = false;  // VA2's mapping keeps h1's backing alive
  CU(cuMemCreate(&h2, sz, &prop, 0)); h2live = true;
  CU(cuMemMap(va1, sz, 0, h2, 0)); m1 = true;
  CU(cuMemSetAccess(va1, sz, &rw, 1));
  if (!launch_fill(va1, SEED_B)) return false;
  if (!verify(va1, SEED_B, "VA1 (fresh handle) after a kernel wrote it")) return false;
  if (!verify(va2, SEED_A2, "VA2 (the OLD handle) after VA1 was remapped and written")) return false;
  return verify_by_kernel(va2, SEED_A2, "VA2 (kernel) after VA1 was remapped and written");
}

static bool step_ro_map() {
  CU(cuMemMap(va3, sz, 0, h2, 0)); m3 = true;
  CU(cuMemSetAccess(va3, sz, &ro, 1));
  if (!verify(va3, SEED_B, "VA3 (PROT_READ, copy engine)")) return false;
  return verify_by_kernel(va3, SEED_B, "VA3 (PROT_READ, kernel)");
}

static bool step_fd_import() {
  int fdsup = 0;
  CU(cuDeviceGetAttribute(&fdsup, CU_DEVICE_ATTRIBUTE_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR_SUPPORTED, dev));
  if (!fdsup) { say("the device reports no POSIX-fd handle support"); return false; }
  CUmemAllocationProp ep = prop;
  ep.requestedHandleTypes = CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR;
  CUmemGenericAllocationHandle he = 0, himp = 0;
  CUdeviceptr vb = 0;
  int fd = -1;
  bool ok = false;
  // a lambda so every exit path below runs the same cleanup
  auto body = [&]() -> bool {
    CU(cuMemCreate(&he, sz, &ep, 0));
    CU(cuMemAddressReserve(&vb, 2 * sz, 0, 0, 0));
    CU(cuMemMap(vb, sz, 0, he, 0));
    CU(cuMemSetAccess(vb, sz, &rw, 1));
    if (!launch_fill(vb, SEED_C)) return false;
    CU(cuMemExportToShareableHandle(&fd, he, CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR, 0));
    CU(cuMemImportFromShareableHandle(&himp, (void *)(uintptr_t)fd, CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR));
    CU(cuMemMap(vb + sz, sz, 0, himp, 0));
    CU(cuMemSetAccess(vb + sz, sz, &rw, 1));
    return verify(vb + sz, SEED_C, "the imported mapping of memory a kernel wrote through the exporter");
  };
  ok = body();
  char keep[sizeof why]; memcpy(keep, why, sizeof why);
  if (vb) { cuMemUnmap(vb + sz, sz); cuMemUnmap(vb, sz); }
  if (himp) cuMemRelease(himp);
  if (he) cuMemRelease(he);
  if (fd >= 0) close(fd);
  if (vb) cuMemAddressFree(vb, 2 * sz);
  memcpy(why, keep, sizeof why);
  return ok;
}

static bool step_teardown() {
  if (m3) { CU(cuMemUnmap(va3, sz)); m3 = false; }
  if (m1) { CU(cuMemUnmap(va1, sz)); m1 = false; }
  if (m2) { CU(cuMemUnmap(va2, sz)); m2 = false; }
  if (h2live) { CU(cuMemRelease(h2)); h2live = false; }
  if (h1live) { CU(cuMemRelease(h1)); h1live = false; }
  if (base) { CU(cuMemAddressFree(base, 3 * sz)); base = 0; }
  if (chk) { RT(cudaFree(chk)); chk = nullptr; }
  RT(cudaDeviceSynchronize());
  return true;
}

// ★ The child: its own context, so the fault it provokes kills nothing the parent still needs.
// Exit 0 = the write through PROT_READ returned an error (correct); 1 = it SUCCEEDED (the bit was
// dropped); 3 = the setup itself failed. SIGALRM after 30 s = the faulting write hung.
static int ro_write_child() {
  alarm(30);
  if (!step_supported()) { printf("RO_WRITE_CHILD setup %s\n", why); fflush(stdout); _exit(3); }
  CUdeviceptr a = 0;
  CUmemGenericAllocationHandle h = 0;
  auto setup = [&]() -> bool {
    CU(cuMemAddressReserve(&a, 2 * sz, 0, 0, 0));
    CU(cuMemCreate(&h, sz, &prop, 0));
    CU(cuMemMap(a, sz, 0, h, 0));
    CU(cuMemSetAccess(a, sz, &rw, 1));
    CU(cuMemMap(a + sz, sz, 0, h, 0));
    CU(cuMemSetAccess(a + sz, sz, &ro, 1));
    return launch_fill(a, SEED_D);
  };
  if (!setup()) { printf("RO_WRITE_CHILD setup %s\n", why); fflush(stdout); _exit(3); }
  fill<<<256, 256>>>(P(a + sz), n, SEED_E);  // the write through the READ-ONLY mapping
  cudaError_t e1 = cudaGetLastError();
  cudaError_t e2 = cudaDeviceSynchronize();
  if (e1 == cudaSuccess && e2 == cudaSuccess) {
    unsigned w = 0;
    cuMemcpyDtoH(&w, a, sizeof w);
    printf("RO_WRITE_CHILD sync=0 landed=%d\n", w == pat(0, SEED_E) ? 1 : 0);
    fflush(stdout);
    _exit(1);
  }
  printf("RO_WRITE_CHILD launch=%d sync=%d (%s)\n", (int)e1, (int)e2, cudaGetErrorString(e2));
  fflush(stdout);
  _exit(0);  // no CUDA teardown on a dead context
}

static bool step_ro_write(const char *self) {
  fflush(stdout);
  pid_t pid = fork();
  if (pid < 0) { say("fork failed"); return false; }
  if (pid == 0) { execl("/proc/self/exe", self, "ro_write_child", (char *)nullptr); _exit(127); }
  int st = 0;
  if (waitpid(pid, &st, 0) != pid) { say("waitpid failed"); return false; }
  if (WIFEXITED(st) && WEXITSTATUS(st) == 0) { say("the write faulted: the read-only mapping is honoured"); return true; }
  if (WIFEXITED(st) && WEXITSTATUS(st) == 1) { say("a kernel write through a PROT_READ mapping SUCCEEDED"); return false; }
  if (WIFEXITED(st)) { say("child exit %d (setup failed or could not exec)", WEXITSTATUS(st)); return false; }
  if (WIFSIGNALED(st) && WTERMSIG(st) == SIGALRM) { say("the faulting write HUNG for 30 s: no error reached the app"); return false; }
  say("child killed by signal %d", WIFSIGNALED(st) ? WTERMSIG(st) : -1);
  return false;
}

int main(int argc, char **argv) {
  setvbuf(stdout, nullptr, _IOLBF, 0);
  if (argc > 1 && !strcmp(argv[1], "ro_write_child")) return ro_write_child();
  bool ok = check("vmm_supported", step_supported());
  if (ok) ok = check("map", step_map());
  else { say("skipped: the VMM API is not supported"); check("map", false); }
  if (ok) ok = check("alias", step_alias());
  else { say("skipped: map failed"); check("alias", false); }
  if (ok) ok = check("remap_same_va", step_remap());
  else { say("skipped: alias failed"); check("remap_same_va", false); }
  if (ok) check("ro_map", step_ro_map());
  else { say("skipped: remap_same_va failed"); check("ro_map", false); }
  if (sz) check("fd_import", step_fd_import());
  else { say("skipped: no allocation granularity"); check("fd_import", false); }
  if (sz) check("teardown", step_teardown());
  else { say("skipped"); check("teardown", false); }
  if (sz) check("ro_write", step_ro_write(argv[0]));
  else { say("skipped"); check("ro_write", false); }
  printf("VMM_PROBE_DONE checks=%d failed=%d\n", checks, failed);
  return failed ? 1 : 0;
}
