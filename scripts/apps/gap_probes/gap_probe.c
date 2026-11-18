// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// gap_probe.c: CUDA driver-API probes of features a GPU-paravirtualisation stack tends to leave behind (docs/design/V3_GPUPV_GAP_MATRIX.md).
// STATUS: RESEARCH, 2026-11-18. Compiles with plain gcc (-ldl), no CUDA toolkit and no header: libcuda is opened with dlopen and the few types used are
// declared here (the constants were checked against the CUDA 12.9 cuda.h on 2026-11-18). NOT yet run on a GPU: the first host and guest runs are the test.
//
//   gap_probe list              the case names
//   gap_probe <case>            run one case in THIS process; prints GAPKV lines and one final line
//       GAPPROBE case=<id> status=PASS|NOT_SUPPORTED|FAIL|MISMATCH|NOT_RUN rc=<CUresult> secs=<s> reason=<token>
// A case reads the device attribute that says whether the feature exists, runs it anyway, and judges: it worked and the attribute says yes -> PASS; it was refused
// and the attribute says no -> NOT_SUPPORTED; it worked against the attribute -> MISMATCH; it failed against the attribute, or returned wrong data -> FAIL.
// One case per process: a crash or hang is the harness's to classify (gap_probes.py), never hidden here.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

typedef int CUresult, CUdevice;
typedef unsigned long long CUdeviceptr, CUmemGenericAllocationHandle;
typedef void *CUcontext, *CUmodule, *CUfunction, *CUstream, *CUevent, *CUgraph, *CUgraphExec, *CUmemoryPool;
typedef struct { char reserved[64]; } CUipcMemHandle, CUipcEventHandle;
typedef struct { int type; int id; } CUmemLocation;
typedef struct { int type; int requestedHandleTypes; CUmemLocation location; void *win32; unsigned char comp, rdma; unsigned short usage; unsigned char rsv[4]; } CUmemAllocationProp;
typedef struct { CUmemLocation location; int flags; } CUmemAccessDesc;
typedef struct { int allocType; int handleTypes; CUmemLocation location; void *win32; size_t maxSize; unsigned short usage; unsigned char reserved[54]; } CUmemPoolProps;
typedef struct { int result; void *errorNode; void *errorFromNode; } CUgraphExecUpdateResultInfo;
enum { PASS, NOT_SUPPORTED, FAIL, MISMATCH, NOT_RUN };
static const char *STATUS[] = {"PASS", "NOT_SUPPORTED", "FAIL", "MISMATCH", "NOT_RUN"};
enum { A_UVA = 41, A_MANAGED = 83, A_PAGEABLE = 88, A_CONCURRENT = 89, A_COOP = 95, A_HOSTREG = 99, A_VMM = 102, A_POSIXFD = 103, A_POOLS = 115, A_POOLTYPES = 119, A_IPCEVENT = 125, A_MAXATTR = 160 };

#define CUFUNCS(X) \
  X(CUresult, cuInit, (unsigned)) X(CUresult, cuDeviceGet, (CUdevice *, int)) X(CUresult, cuDeviceGetAttribute, (int *, int, CUdevice)) \
  X(CUresult, cuDriverGetVersion, (int *)) X(CUresult, cuCtxCreate_v2, (CUcontext *, unsigned, CUdevice)) X(CUresult, cuCtxDestroy_v2, (CUcontext)) \
  X(CUresult, cuCtxSynchronize, (void)) X(CUresult, cuMemGetInfo_v2, (size_t *, size_t *)) X(CUresult, cuModuleLoadData, (CUmodule *, const void *)) \
  X(CUresult, cuModuleUnload, (CUmodule)) X(CUresult, cuModuleGetFunction, (CUfunction *, CUmodule, const char *)) \
  X(CUresult, cuLaunchKernel, (CUfunction, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, CUstream, void **, void **)) \
  X(CUresult, cuMemAlloc_v2, (CUdeviceptr *, size_t)) X(CUresult, cuMemFree_v2, (CUdeviceptr)) X(CUresult, cuMemcpyHtoD_v2, (CUdeviceptr, const void *, size_t)) \
  X(CUresult, cuMemcpyDtoH_v2, (void *, CUdeviceptr, size_t)) X(CUresult, cuMemcpyDtoDAsync_v2, (CUdeviceptr, CUdeviceptr, size_t, CUstream)) \
  X(CUresult, cuMemsetD32Async, (CUdeviceptr, unsigned, size_t, CUstream)) \
  X(CUresult, cuStreamCreate, (CUstream *, unsigned)) X(CUresult, cuStreamDestroy_v2, (CUstream)) X(CUresult, cuStreamSynchronize, (CUstream)) \
  X(CUresult, cuEventCreate, (CUevent *, unsigned)) X(CUresult, cuEventDestroy_v2, (CUevent)) X(CUresult, cuEventRecord, (CUevent, CUstream)) \
  X(CUresult, cuEventSynchronize, (CUevent)) X(CUresult, cuMemHostAlloc, (void **, size_t, unsigned)) X(CUresult, cuMemFreeHost, (void *)) \
  X(CUresult, cuMemHostRegister_v2, (void *, size_t, unsigned)) X(CUresult, cuMemHostUnregister, (void *)) X(CUresult, cuMemHostGetDevicePointer_v2, (CUdeviceptr *, void *, unsigned)) \
  X(CUresult, cuMemAllocManaged, (CUdeviceptr *, size_t, unsigned)) X(CUresult, cuMemPrefetchAsync, (CUdeviceptr, size_t, CUdevice, CUstream)) \
  X(CUresult, cuMemAdvise, (CUdeviceptr, size_t, int, CUdevice)) \
  X(CUresult, cuIpcGetMemHandle, (CUipcMemHandle *, CUdeviceptr)) X(CUresult, cuIpcOpenMemHandle_v2, (CUdeviceptr *, CUipcMemHandle, unsigned)) \
  X(CUresult, cuIpcCloseMemHandle, (CUdeviceptr)) X(CUresult, cuIpcGetEventHandle, (CUipcEventHandle *, CUevent)) X(CUresult, cuIpcOpenEventHandle, (CUevent *, CUipcEventHandle)) \
  X(CUresult, cuMemCreate, (CUmemGenericAllocationHandle *, size_t, const CUmemAllocationProp *, unsigned long long)) X(CUresult, cuMemRelease, (CUmemGenericAllocationHandle)) \
  X(CUresult, cuMemAddressReserve, (CUdeviceptr *, size_t, size_t, CUdeviceptr, unsigned long long)) X(CUresult, cuMemAddressFree, (CUdeviceptr, size_t)) \
  X(CUresult, cuMemMap, (CUdeviceptr, size_t, size_t, CUmemGenericAllocationHandle, unsigned long long)) X(CUresult, cuMemUnmap, (CUdeviceptr, size_t)) \
  X(CUresult, cuMemSetAccess, (CUdeviceptr, size_t, const CUmemAccessDesc *, size_t)) X(CUresult, cuMemGetAllocationGranularity, (size_t *, const CUmemAllocationProp *, int)) \
  X(CUresult, cuMemExportToShareableHandle, (void *, CUmemGenericAllocationHandle, int, unsigned long long)) \
  X(CUresult, cuMemImportFromShareableHandle, (CUmemGenericAllocationHandle *, void *, int)) \
  X(CUresult, cuStreamBeginCapture_v2, (CUstream, int)) X(CUresult, cuStreamEndCapture, (CUstream, CUgraph *)) \
  X(CUresult, cuGraphInstantiateWithFlags, (CUgraphExec *, CUgraph, unsigned long long)) X(CUresult, cuGraphLaunch, (CUgraphExec, CUstream)) \
  X(CUresult, cuGraphExecUpdate_v2, (CUgraphExec, CUgraph, CUgraphExecUpdateResultInfo *)) X(CUresult, cuGraphDestroy, (CUgraph)) X(CUresult, cuGraphExecDestroy, (CUgraphExec)) \
  X(CUresult, cuDeviceGetDefaultMemPool, (CUmemoryPool *, CUdevice)) X(CUresult, cuMemAllocAsync, (CUdeviceptr *, size_t, CUstream)) X(CUresult, cuMemFreeAsync, (CUdeviceptr, CUstream)) \
  X(CUresult, cuMemPoolSetAttribute, (CUmemoryPool, int, void *)) X(CUresult, cuMemPoolGetAttribute, (CUmemoryPool, int, void *)) X(CUresult, cuMemPoolTrimTo, (CUmemoryPool, size_t)) \
  X(CUresult, cuMemPoolCreate, (CUmemoryPool *, const CUmemPoolProps *)) X(CUresult, cuMemPoolDestroy, (CUmemoryPool)) \
  X(CUresult, cuMemPoolExportToShareableHandle, (void *, CUmemoryPool, int, unsigned long long))
#define DECL(r, n, a) static r(*n) a;
CUFUNCS(DECL)
static const char *MISSING;
static void load_syms(void) {
  void *h = dlopen("libcuda.so.1", RTLD_NOW);
  if (!h) { MISSING = "no_libcuda"; return; }
#define LOAD(r, n, a) if (!(n = (r(*) a)dlsym(h, #n)) && !MISSING) MISSING = "missing_" #n;
  CUFUNCS(LOAD)
}

// ---------------------------------------------------------------------------------------------------------------- reporting
static const char *CASE = "?", *REASON = "-";
static CUresult RC;
static void kv(const char *key, const char *fmt, ...) {
  va_list ap; char buf[256];
  va_start(ap, fmt); vsnprintf(buf, sizeof buf, fmt, ap); va_end(ap);
  printf("GAPKV %s %s=%s\n", CASE, key, buf);
}
static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }
static int fail(const char *why, CUresult rc) { REASON = why; RC = rc; return FAIL; }
#define CK(call) do { CUresult r_ = (call); if (r_) return fail(#call, r_); } while (0)
#define REQ(...) do { const void *p_[] = {__VA_ARGS__}; for (unsigned i_ = 0; i_ < sizeof p_ / sizeof *p_; i_++) if (!p_[i_]) { REASON = "missing_symbol"; return NOT_RUN; } } while (0)
static int judge(int attr_ok, CUresult rc, int verified, const char *why) {
  RC = rc;
  if (rc == 0 && verified) return attr_ok ? PASS : (REASON = "worked_against_attribute", MISMATCH);
  if (rc == 0) return fail(why, 0);
  REASON = why;
  return attr_ok ? FAIL : NOT_SUPPORTED;
}

// ---------------------------------------------------------------------------------------------------------------- device, module
static CUdevice dev; static CUcontext ctx; static CUmodule mod; static CUfunction f_addone, f_fill, f_touch, f_spin, f_fault, f_trap;
static const char PTX[] =
  ".version 7.0\n.target sm_75\n.address_size 64\n"
  ".visible .entry addone(.param .u64 p, .param .u32 n){ .reg .pred q; .reg .b32 r<6>; .reg .b64 d<5>;\n"
  " ld.param.u64 d1,[p]; ld.param.u32 r1,[n]; mov.u32 r2,%ctaid.x; mov.u32 r3,%ntid.x; mov.u32 r4,%tid.x; mad.lo.u32 r5,r2,r3,r4;\n"
  " setp.ge.u32 q,r5,r1; @q bra D; mul.wide.u32 d2,r5,4; add.u64 d3,d1,d2; ld.u32 r0,[d3]; add.u32 r0,r0,1; st.u32 [d3],r0; D: ret; }\n"
  ".visible .entry fill(.param .u64 p, .param .u32 n, .param .u32 v){ .reg .pred q; .reg .b32 r<7>; .reg .b64 d<5>;\n"
  " ld.param.u64 d1,[p]; ld.param.u32 r1,[n]; ld.param.u32 r6,[v]; mov.u32 r2,%ctaid.x; mov.u32 r3,%ntid.x; mov.u32 r4,%tid.x; mad.lo.u32 r5,r2,r3,r4;\n"
  " setp.ge.u32 q,r5,r1; @q bra D; mul.wide.u32 d2,r5,4; add.u64 d3,d1,d2; add.u32 r0,r5,r6; st.u32 [d3],r0; D: ret; }\n"
  ".visible .entry touchpages(.param .u64 p, .param .u64 np){ .reg .pred q; .reg .b32 r<6>; .reg .b64 d<7>;\n"
  " ld.param.u64 d1,[p]; ld.param.u64 d2,[np]; mov.u32 r2,%ctaid.x; mov.u32 r3,%ntid.x; mov.u32 r4,%tid.x; mad.lo.u32 r5,r2,r3,r4; cvt.u64.u32 d3,r5;\n"
  " setp.ge.u64 q,d3,d2; @q bra D; mul.lo.u64 d4,d3,4096; add.u64 d5,d1,d4; st.u32 [d5],r5; D: ret; }\n"
  ".visible .entry spin(.param .u64 ns, .param .u64 out){ .reg .pred q; .reg .b32 r1; .reg .b64 d<7>;\n"
  " ld.param.u64 d1,[ns]; ld.param.u64 d2,[out]; mov.u64 d3,%globaltimer; L: mov.u64 d4,%globaltimer; sub.u64 d5,d4,d3; setp.lt.u64 q,d5,d1; @q bra L;\n"
  " mov.u32 r1,1; st.u32 [d2],r1; ret; }\n"
  ".visible .entry fault(.param .u64 p){ .reg .b32 r1; .reg .b64 d1; ld.param.u64 d1,[p]; mov.u32 r1,7; st.u32 [d1],r1; ret; }\n"
  ".visible .entry trapk(){ trap; }\n";
static int setup(int want_module) {
  load_syms();
  if (MISSING) { REASON = MISSING; return NOT_RUN; }
  CUresult r = cuInit(0);
  if (r) { RC = r; REASON = "cuInit_failed"; return NOT_RUN; }
  if ((r = cuDeviceGet(&dev, 0)) || (r = cuCtxCreate_v2(&ctx, 0, dev))) { RC = r; REASON = "no_context"; return NOT_RUN; }
  if (!want_module) return PASS;
  if ((r = cuModuleLoadData(&mod, PTX))) { RC = r; REASON = "ptx_load_failed"; return NOT_RUN; }
  if (cuModuleGetFunction(&f_addone, mod, "addone") || cuModuleGetFunction(&f_fill, mod, "fill") || cuModuleGetFunction(&f_touch, mod, "touchpages") ||
      cuModuleGetFunction(&f_spin, mod, "spin") || cuModuleGetFunction(&f_fault, mod, "fault") || cuModuleGetFunction(&f_trap, mod, "trapk")) { REASON = "ptx_symbols"; return NOT_RUN; }
  return PASS;
}
static int attr(int a) { int v = -1; if (cuDeviceGetAttribute(&v, a, dev)) return -1; return v; }
static CUresult launch1(CUfunction f, unsigned grid, unsigned block, CUstream s, void **params) { return cuLaunchKernel(f, grid, 1, 1, block, 1, 1, 0, s, params, NULL); }
static CUresult run_addone(CUdeviceptr p, unsigned n, CUstream s) { void *a[] = {&p, &n}; return launch1(f_addone, (n + 255) / 256, 256, s, a); }
static CUresult run_fill(CUdeviceptr p, unsigned n, unsigned v, CUstream s) { void *a[] = {&p, &n, &v}; return launch1(f_fill, (n + 255) / 256, 256, s, a); }
static int check_fill(const unsigned *h, unsigned n, unsigned v) { for (unsigned i = 0; i < n; i++) if (h[i] != v + i) return 0; return 1; }
#define MIB (1ULL << 20)
#define GIB (1ULL << 30)

// ---------------------------------------------------------------------------------------------------------------- cases: attributes
static int c_attr_dump(void) {
  int n = 0, ver = 0; size_t fr, tot;
  CK(cuDriverGetVersion(&ver)); kv("driver_api_version", "%d", ver);
  for (int a = 1; a < A_MAXATTR; a++) { int v; CUresult r = cuDeviceGetAttribute(&v, a, dev); if (r == 0) { kv("attr", "%d:%d", a, v); n++; } else kv("attr", "%d:E%d", a, r); }
  CK(cuMemGetInfo_v2(&fr, &tot)); kv("mem_total_mib", "%zu", tot >> 20);
  kv("attrs_ok", "%d", n);
  return n >= 60 ? PASS : fail("fewer_than_60_attributes", 0);
}
// ---------------------------------------------------------------------------------------------------------------- cases: pinned and host memory
static int pinned_n(size_t bytes) {
  REQ(cuMemHostAlloc, cuMemFreeHost);
  void *h = NULL; CUdeviceptr d; size_t chunk = 64 * MIB; unsigned *buf;
  CUresult r = cuMemHostAlloc(&h, bytes, 0);
  kv("alloc_rc", "%d", r);
  if (r) return judge(1, r, 0, "cuMemHostAlloc");
  memset(h, 0x5a, bytes);
  CK(cuMemAlloc_v2(&d, chunk)); buf = malloc(chunk);
  CK(cuMemcpyHtoD_v2(d, (char *)h + bytes - chunk, chunk)); CK(cuMemcpyDtoH_v2(buf, d, chunk));
  int ok = !memcmp(buf, (char *)h + bytes - chunk, chunk);
  CK(cuMemFreeHost(h));
  return judge(1, 0, ok, "pinned_roundtrip_mismatch");
}
static int c_pinned_1g(void) { return pinned_n(1 * GIB); }
static int c_pinned_4g(void) { return pinned_n(4 * GIB); }
static int c_host_register_512m(void) {
  REQ(cuMemHostRegister_v2, cuMemHostUnregister);
  size_t n = 512 * MIB; void *p = mmap(NULL, n, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  if (p == MAP_FAILED) return fail("mmap", 0);
  memset(p, 0x33, n);
  CUresult r = cuMemHostRegister_v2(p, n, 0x02); kv("register_rc", "%d", r);
  if (r) return judge(attr(A_HOSTREG) == 1, r, 0, "cuMemHostRegister");
  CUdeviceptr d; CK(cuMemAlloc_v2(&d, 16 * MIB)); CK(cuMemcpyHtoD_v2(d, p, 16 * MIB));
  unsigned *b = malloc(16 * MIB); CK(cuMemcpyDtoH_v2(b, d, 16 * MIB));
  int ok = !memcmp(b, p, 16 * MIB);
  CK(cuMemHostUnregister(p));
  return judge(attr(A_HOSTREG) == 1, 0, ok, "registered_roundtrip_mismatch");
}
static int c_mapped_zerocopy(void) {
  REQ(cuMemHostAlloc, cuMemHostGetDevicePointer_v2);
  unsigned n = 1 << 20; unsigned *h = NULL; CUdeviceptr d;
  CK(cuMemHostAlloc((void **)&h, n * 4, 0x02));
  for (unsigned i = 0; i < n; i++) h[i] = i;
  CK(cuMemHostGetDevicePointer_v2(&d, h, 0));
  CK(run_addone(d, n, NULL)); CK(cuCtxSynchronize());
  for (unsigned i = 0; i < n; i++) if (h[i] != i + 1) return fail("zero_copy_value", 0);
  return PASS;
}
static int c_pinned_huge_refused(void) {   // the guest must get an error, promptly, for an absurd pinned request (guest-reachable unbounded resource)
  REQ(cuMemHostAlloc);
  void *h = NULL; double t = now(); CUresult r = cuMemHostAlloc(&h, 1ULL << 40, 0); double s = now() - t;
  kv("rc", "%d", r); kv("refused_in_under_30s", "%d", s < 30);
  if (r == 0) return fail("a_tib_of_pinned_host_memory_was_granted", 0);
  RC = r; return s < 30 ? PASS : fail("refusal_took_too_long", r);
}
// ---------------------------------------------------------------------------------------------------------------- cases: managed memory
static int c_managed_prefetch_advise(void) {
  REQ(cuMemAllocManaged, cuMemPrefetchAsync, cuMemAdvise);
  unsigned n = 16u << 20; CUdeviceptr p; kv("attr_managed", "%d", attr(A_MANAGED)); kv("attr_concurrent", "%d", attr(A_CONCURRENT)); kv("attr_pageable", "%d", attr(A_PAGEABLE));
  CUresult r = cuMemAllocManaged(&p, (size_t)n * 4, 1);
  if (r) return judge(attr(A_MANAGED) == 1, r, 0, "cuMemAllocManaged");
  unsigned *h = (unsigned *)(uintptr_t)p; for (unsigned i = 0; i < n; i += 1024) h[i] = i;
  CUresult a1 = cuMemAdvise(p, (size_t)n * 4, 3, dev), a2 = cuMemAdvise(p, (size_t)n * 4, 1, dev), a3 = cuMemAdvise(p, (size_t)n * 4, 5, dev);
  kv("advise_rc", "%d,%d,%d", a1, a2, a3);
  CUresult pf = cuMemPrefetchAsync(p, (size_t)n * 4, dev, NULL); kv("prefetch_dev_rc", "%d", pf);
  CK(run_addone(p, n, NULL)); CK(cuCtxSynchronize());
  CUresult pc = cuMemPrefetchAsync(p, (size_t)n * 4, -1, NULL); kv("prefetch_cpu_rc", "%d", pc); CK(cuCtxSynchronize());
  for (unsigned i = 0; i < n; i += 1024) if (h[i] != i + 1) return fail("managed_value_after_kernel", 0);
  return judge(attr(A_MANAGED) == 1, 0, 1, "-");
}
static int c_managed_concurrent(void) {
  REQ(cuMemAllocManaged);
  int ok = attr(A_CONCURRENT) == 1; kv("attr_concurrent", "%d", attr(A_CONCURRENT));
  if (!ok) { kv("attempted", "0"); REASON = "attr_concurrent_managed_access_is_0"; return NOT_SUPPORTED; }   // a CPU touch during a kernel would fault the process: not attempted
  CUdeviceptr p, flag; CK(cuMemAllocManaged(&p, 4 * MIB, 1)); CK(cuMemAlloc_v2(&flag, 4));
  unsigned long long ns = 1500000000ULL; void *a[] = {&ns, &flag}; CK(launch1(f_spin, 1, 1, NULL, a));
  volatile unsigned *h = (volatile unsigned *)(uintptr_t)p; unsigned cnt = 0; double t = now();
  while (now() - t < 1.0) { h[cnt % 1024] = cnt; cnt++; }
  CK(cuCtxSynchronize()); kv("cpu_writes_during_kernel", "%d", cnt > 0);
  return judge(1, 0, cnt > 0, "-");
}
static int c_managed_oversub(void) {
  REQ(cuMemAllocManaged);
  size_t fr, tot; CK(cuMemGetInfo_v2(&fr, &tot)); size_t base = tot < 8 * GIB ? tot : 8 * GIB, bytes = base + base / 4;
  kv("device_total_mib", "%zu", tot >> 20); kv("bytes_mib", "%zu", bytes >> 20); kv("oversubscribes_device", "%d", bytes > tot);
  CUdeviceptr p; CUresult r = cuMemAllocManaged(&p, bytes, 1);
  if (r) return judge(attr(A_CONCURRENT) == 1, r, 0, "cuMemAllocManaged_oversub");
  unsigned long long np = bytes / 4096; void *a[] = {&p, &np};
  CK(launch1(f_touch, (unsigned)((np + 255) / 256), 256, NULL, a)); CK(cuCtxSynchronize());
  unsigned *h = (unsigned *)(uintptr_t)p; for (unsigned long long i = 0; i < np; i += np / 64 + 1) if (h[i * 1024] != (unsigned)i) return fail("oversub_value", 0);
  return judge(attr(A_CONCURRENT) == 1 || bytes <= tot, 0, 1, "-");
}
// ---------------------------------------------------------------------------------------------------------------- cases: IPC (parent here, child is an exec of this binary)
static int spawn_self(const char *a1, const char *a2, const char *a3, pid_t *pid) {
  fflush(stdout); pid_t c = fork(); if (c < 0) return -1;
  if (c == 0) { execl("/proc/self/exe", "gap_probe", a1, a2, a3, (char *)NULL); _exit(127); }
  *pid = c; return 0;
}
static void hex(const char *b, int n, char *out) { for (int i = 0; i < n; i++) sprintf(out + 2 * i, "%02x", (unsigned char)b[i]); }
static void unhex(const char *s, char *b, int n) { for (int i = 0; i < n; i++) { unsigned v; sscanf(s + 2 * i, "%2x", &v); b[i] = (char)v; } }
static int child_status(pid_t c) { int st = 0; waitpid(c, &st, 0); return WIFEXITED(st) ? WEXITSTATUS(st) : 128 + WTERMSIG(st); }
static int c_ipc_legacy(void) {
  unsigned n = 1 << 18; CUdeviceptr d; CUipcMemHandle h; char hx[129]; pid_t c; unsigned *buf = malloc(n * 4);
  REQ(cuIpcGetMemHandle, cuIpcOpenMemHandle_v2, cuIpcCloseMemHandle);
  CK(cuMemAlloc_v2(&d, n * 4)); CK(run_fill(d, n, 100, NULL)); CK(cuCtxSynchronize());
  CUresult r = cuIpcGetMemHandle(&h, d); kv("get_rc", "%d", r);
  if (r) return judge(attr(A_UVA) == 1, r, 0, "cuIpcGetMemHandle");
  hex((char *)&h, 64, hx);
  if (spawn_self("ipc_child", hx, "-", &c)) return fail("fork", 0);
  int crc = child_status(c); kv("child_exit", "%d", crc);
  CK(cuMemcpyDtoH_v2(buf, d, n * 4));
  int ok = crc == 0 && check_fill(buf, n, 101);   // the child added one through its own mapping
  return judge(attr(A_UVA) == 1, ok ? 0 : (RC = crc), ok, "child_open_or_write_failed");
}
static int c_ipc_child(const char *hx) {
  CUipcMemHandle h; CUdeviceptr p; unhex(hx, (char *)&h, 64); unsigned n = 1 << 18;
  CK(cuIpcOpenMemHandle_v2(&p, h, 1)); CK(run_addone(p, n, NULL)); CK(cuCtxSynchronize()); CK(cuIpcCloseMemHandle(p));
  return PASS;
}
static int c_ipc_event(void) {
  REQ(cuIpcGetEventHandle, cuIpcOpenEventHandle);
  CUevent ev; CUipcEventHandle h; kv("attr_ipc_event", "%d", attr(A_IPCEVENT));
  CK(cuEventCreate(&ev, 0x2 | 0x4)); CK(cuEventRecord(ev, NULL));
  CUresult r = cuIpcGetEventHandle(&h, ev); kv("get_rc", "%d", r);
  if (r) return judge(attr(A_IPCEVENT) == 1, r, 0, "cuIpcGetEventHandle");
  char hx[129]; pid_t c; hex((char *)&h, 64, hx);
  if (spawn_self("ipc_event_child", hx, "-", &c)) return fail("fork", 0);
  int crc = child_status(c); kv("child_exit", "%d", crc);
  return judge(attr(A_IPCEVENT) == 1, crc ? (RC = crc) : 0, crc == 0, "child_event_wait_failed");
}
static int c_ipc_event_child(const char *hx) {
  CUipcEventHandle h; CUevent ev; unhex(hx, (char *)&h, 64);
  CK(cuIpcOpenEventHandle(&ev, h)); CK(cuEventSynchronize(ev)); return PASS;
}
static void send_fd(int s, int fd) {
  char cb[CMSG_SPACE(sizeof(int))] = {0}; char x = 'x'; struct iovec iov = {&x, 1};
  struct msghdr m = {0}; m.msg_iov = &iov; m.msg_iovlen = 1; m.msg_control = cb; m.msg_controllen = sizeof cb;
  struct cmsghdr *c = CMSG_FIRSTHDR(&m); c->cmsg_level = SOL_SOCKET; c->cmsg_type = SCM_RIGHTS; c->cmsg_len = CMSG_LEN(sizeof(int)); memcpy(CMSG_DATA(c), &fd, sizeof fd);
  sendmsg(s, &m, 0);
}
static int recv_fd(int s) {
  char cb[CMSG_SPACE(sizeof(int))] = {0}; char x; struct iovec iov = {&x, 1};
  struct msghdr m = {0}; m.msg_iov = &iov; m.msg_iovlen = 1; m.msg_control = cb; m.msg_controllen = sizeof cb;
  if (recvmsg(s, &m, 0) <= 0) return -1;
  struct cmsghdr *c = CMSG_FIRSTHDR(&m); int fd = -1; if (c) memcpy(&fd, CMSG_DATA(c), sizeof fd); return fd;
}
static CUmemAllocationProp vmm_prop(int handle_types) { CUmemAllocationProp p; memset(&p, 0, sizeof p); p.type = 1; p.requestedHandleTypes = handle_types; p.location.type = 1; p.location.id = dev; return p; }
static int vmm_map(CUmemGenericAllocationHandle h, size_t sz, CUdeviceptr *va) {
  CUmemAccessDesc ad = {{1, dev}, 3};
  CK(cuMemAddressReserve(va, sz, 0, 0, 0)); CK(cuMemMap(*va, sz, 0, h, 0)); CK(cuMemSetAccess(*va, sz, &ad, 1)); return 0;
}
static int c_ipc_vmm_fd(void) {
  REQ(cuMemCreate, cuMemExportToShareableHandle, cuMemGetAllocationGranularity);
  int ok_attr = attr(A_POSIXFD) == 1; kv("attr_posix_fd", "%d", attr(A_POSIXFD)); kv("attr_vmm", "%d", attr(A_VMM));
  CUmemAllocationProp p = vmm_prop(1); size_t g; CK(cuMemGetAllocationGranularity(&g, &p, 1));
  CUmemGenericAllocationHandle h; CUresult r = cuMemCreate(&h, g, &p, 0);
  if (r) return judge(ok_attr, r, 0, "cuMemCreate_fd_exportable");
  CUdeviceptr va; unsigned n = (unsigned)(g / 4); if (vmm_map(h, g, &va)) return FAIL;
  CK(run_fill(va, n, 7, NULL)); CK(cuCtxSynchronize());
  int fd = -1; r = cuMemExportToShareableHandle(&fd, h, 1, 0); kv("export_rc", "%d", r);
  if (r) return judge(ok_attr, r, 0, "cuMemExportToShareableHandle");
  int sv[2]; if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv)) return fail("socketpair", 0);
  char a2[16], a3[32]; snprintf(a2, sizeof a2, "%d", sv[1]); snprintf(a3, sizeof a3, "%zu", g); pid_t c;
  if (spawn_self("ipc_vmm_child", a2, a3, &c)) return fail("fork", 0);
  send_fd(sv[0], fd); int crc = child_status(c); kv("child_exit", "%d", crc);
  unsigned *buf = malloc(g); CK(cuMemcpyDtoH_v2(buf, va, g));
  int ok = crc == 0 && check_fill(buf, n, 8);
  return judge(ok_attr, ok ? 0 : (RC = crc), ok, "child_import_map_or_write_failed");
}
static int c_ipc_vmm_child(const char *sock, const char *gs) {
  int s = atoi(sock); size_t g = strtoull(gs, NULL, 10); int fd = recv_fd(s); if (fd < 0) return fail("recv_fd", 0);
  CUmemGenericAllocationHandle h; CUdeviceptr va; CK(cuMemImportFromShareableHandle(&h, (void *)(intptr_t)fd, 1));
  if (vmm_map(h, g, &va)) return FAIL;
  CK(run_addone(va, (unsigned)(g / 4), NULL)); CK(cuCtxSynchronize()); return PASS;
}
// ---------------------------------------------------------------------------------------------------------------- cases: virtual memory management
static int c_vmm_granularity(void) {
  REQ(cuMemGetAllocationGranularity);
  CUmemAllocationProp p = vmm_prop(0); size_t gmin = 0, grec = 0;
  CUresult r = cuMemGetAllocationGranularity(&gmin, &p, 0); CUresult r2 = cuMemGetAllocationGranularity(&grec, &p, 1);
  kv("attr_vmm", "%d", attr(A_VMM)); kv("gran_min", "%zu", gmin); kv("gran_recommended", "%zu", grec);
  int sane = gmin >= 4096 && (gmin & (gmin - 1)) == 0 && grec >= gmin && grec % gmin == 0;
  return judge(attr(A_VMM) == 1, r | r2, sane, "granularity_not_sane");
}
static int c_vmm_basic(void) {
  REQ(cuMemCreate, cuMemGetAllocationGranularity, cuMemAddressReserve);
  CUmemAllocationProp p = vmm_prop(0); size_t g; CK(cuMemGetAllocationGranularity(&g, &p, 1)); size_t sz = 4 * g;
  CUmemGenericAllocationHandle h; CUresult r = cuMemCreate(&h, sz, &p, 0);
  if (r) return judge(attr(A_VMM) == 1, r, 0, "cuMemCreate");
  CUdeviceptr va; if (vmm_map(h, sz, &va)) return FAIL;
  unsigned n = (unsigned)(sz / 4); CK(run_fill(va, n, 3, NULL)); CK(cuCtxSynchronize());
  unsigned *buf = malloc(sz); CK(cuMemcpyDtoH_v2(buf, va, sz)); int ok = check_fill(buf, n, 3);
  CK(cuMemUnmap(va, sz)); CK(cuMemRelease(h)); CK(cuMemAddressFree(va, sz));
  return judge(attr(A_VMM) == 1, 0, ok, "vmm_value");
}
static int c_vmm_remap(void) {   // two physical pieces behind one contiguous VA; then the order is swapped: the data must follow the handles
  REQ(cuMemCreate, cuMemGetAllocationGranularity, cuMemAddressReserve);
  CUmemAllocationProp p = vmm_prop(0); size_t g; CK(cuMemGetAllocationGranularity(&g, &p, 1));
  CUmemGenericAllocationHandle h1, h2; CUresult r = cuMemCreate(&h1, g, &p, 0);
  if (r) return judge(attr(A_VMM) == 1, r, 0, "cuMemCreate");
  CK(cuMemCreate(&h2, g, &p, 0)); CUdeviceptr va; CUmemAccessDesc ad = {{1, dev}, 3};
  CK(cuMemAddressReserve(&va, 2 * g, 0, 0, 0));
  CK(cuMemMap(va, g, 0, h1, 0)); CK(cuMemMap(va + g, g, 0, h2, 0)); CK(cuMemSetAccess(va, 2 * g, &ad, 1));
  unsigned n = (unsigned)(2 * g / 4); CK(run_fill(va, n, 0, NULL)); CK(cuCtxSynchronize());
  CK(cuMemUnmap(va, 2 * g)); CK(cuMemMap(va, g, 0, h2, 0)); CK(cuMemMap(va + g, g, 0, h1, 0)); CK(cuMemSetAccess(va, 2 * g, &ad, 1));
  unsigned *buf = malloc(2 * g); CK(cuMemcpyDtoH_v2(buf, va, 2 * g));
  unsigned half = n / 2, ok = 1;
  for (unsigned i = 0; i < half; i++) if (buf[i] != half + i || buf[half + i] != i) { ok = 0; break; }
  return judge(attr(A_VMM) == 1, 0, ok, "remap_data_did_not_follow_handles");
}
// ---------------------------------------------------------------------------------------------------------------- cases: graphs (stream capture, instantiate, update, replay)
static int c_graph_capture_launch(void) {
  REQ(cuStreamBeginCapture_v2, cuGraphInstantiateWithFlags, cuGraphLaunch);
  unsigned n = 1 << 16; CUdeviceptr d; CUstream s; CUgraph g; CUgraphExec e;
  CK(cuMemAlloc_v2(&d, n * 4)); CK(cuMemsetD32Async(d, 0, n, NULL)); CK(cuCtxSynchronize()); CK(cuStreamCreate(&s, 1));
  CK(cuStreamBeginCapture_v2(s, 0)); for (int i = 0; i < 4; i++) CK(run_addone(d, n, s)); CK(cuStreamEndCapture(s, &g));
  CK(cuGraphInstantiateWithFlags(&e, g, 0));
  double t = now(); for (int i = 0; i < 1000; i++) CK(cuGraphLaunch(e, s)); CK(cuStreamSynchronize(s)); kv("launches_per_s", "%d", (int)(1000 / (now() - t)) / 100 * 100);
  unsigned *h = malloc(n * 4); CK(cuMemcpyDtoH_v2(h, d, n * 4));
  for (unsigned i = 0; i < n; i++) if (h[i] != 4000) return fail("graph_replay_value", 0);
  return PASS;
}
static int capture_add(CUdeviceptr d, unsigned n, CUstream s, CUgraph *g) {
  CK(cuStreamBeginCapture_v2(s, 0)); CK(run_addone(d, n, s)); CK(cuStreamEndCapture(s, g)); return 0;
}
static int c_graph_update(void) {
  REQ(cuGraphExecUpdate_v2, cuGraphInstantiateWithFlags);
  unsigned n = 4096; CUdeviceptr a, b; CUstream s; CUgraph g1, g2; CUgraphExec e;
  CK(cuMemAlloc_v2(&a, n * 4)); CK(cuMemAlloc_v2(&b, n * 4)); CK(cuMemsetD32Async(a, 0, n, NULL)); CK(cuMemsetD32Async(b, 0, n, NULL)); CK(cuCtxSynchronize());
  CK(cuStreamCreate(&s, 1)); if (capture_add(a, n, s, &g1) || capture_add(b, n, s, &g2)) return FAIL;
  CK(cuGraphInstantiateWithFlags(&e, g1, 0)); CK(cuGraphLaunch(e, s)); CK(cuStreamSynchronize(s));
  CUgraphExecUpdateResultInfo ri = {0}; CUresult r = cuGraphExecUpdate_v2(e, g2, &ri); kv("update_rc", "%d", r); kv("update_result", "%d", ri.result);
  if (r || ri.result) return fail("cuGraphExecUpdate", r);
  CK(cuGraphLaunch(e, s)); CK(cuStreamSynchronize(s));
  unsigned *ha = malloc(n * 4), *hb = malloc(n * 4); CK(cuMemcpyDtoH_v2(ha, a, n * 4)); CK(cuMemcpyDtoH_v2(hb, b, n * 4));
  for (unsigned i = 0; i < n; i++) if (ha[i] != 1 || hb[i] != 1) return fail("graph_update_target_not_switched", 0);
  return PASS;
}
static int c_graph_mem_nodes(void) {
  REQ(cuStreamBeginCapture_v2, cuGraphInstantiateWithFlags, cuMemcpyDtoDAsync_v2);
  unsigned n = 1 << 14; CUdeviceptr a, b; CUstream s; CUgraph g; CUgraphExec e;
  CK(cuMemAlloc_v2(&a, n * 4)); CK(cuMemAlloc_v2(&b, n * 4)); CK(cuStreamCreate(&s, 1));
  CK(cuStreamBeginCapture_v2(s, 0)); CK(cuMemsetD32Async(a, 5, n, s)); CK(cuMemcpyDtoDAsync_v2(b, a, n * 4, s)); CK(run_addone(b, n, s)); CK(cuStreamEndCapture(s, &g));
  CK(cuGraphInstantiateWithFlags(&e, g, 0)); for (int i = 0; i < 50; i++) CK(cuGraphLaunch(e, s)); CK(cuStreamSynchronize(s));
  unsigned *h = malloc(n * 4); CK(cuMemcpyDtoH_v2(h, b, n * 4)); for (unsigned i = 0; i < n; i++) if (h[i] != 6) return fail("graph_memset_memcpy_value", 0);
  return PASS;
}
// ---------------------------------------------------------------------------------------------------------------- cases: stream-ordered allocator
static int c_pool_alloc_free(void) {
  REQ(cuDeviceGetDefaultMemPool, cuMemAllocAsync, cuMemFreeAsync);
  int ok = attr(A_POOLS) == 1; CUstream s; CK(cuStreamCreate(&s, 1)); CUdeviceptr p; unsigned n = 1 << 20;
  CUresult r = cuMemAllocAsync(&p, n * 4, s); kv("attr_pools", "%d", attr(A_POOLS));
  if (r) return judge(ok, r, 0, "cuMemAllocAsync");
  CK(cuMemFreeAsync(p, s));
  unsigned *h = malloc(n * 4);
  for (int i = 1; i <= 100; i++) {
    CK(cuMemAllocAsync(&p, (size_t)i * 64 * 1024, s)); CK(run_fill(p, i * 16 * 1024, (unsigned)i, s)); CK(cuStreamSynchronize(s)); CK(cuMemcpyDtoH_v2(h, p, 4096)); CK(cuMemFreeAsync(p, s));
    if (!check_fill(h, 1024, (unsigned)i)) return fail("pool_value", 0);
  }
  CK(cuStreamSynchronize(s)); return judge(ok, 0, 1, "-");
}
static int c_pool_threshold_trim(void) {
  REQ(cuDeviceGetDefaultMemPool, cuMemPoolSetAttribute, cuMemPoolGetAttribute, cuMemPoolTrimTo);
  CUmemoryPool pool; CK(cuDeviceGetDefaultMemPool(&pool, dev)); CUstream s; CK(cuStreamCreate(&s, 1));
  unsigned long long thr = ~0ULL, cur = 0; CK(cuMemPoolSetAttribute(pool, 4, &thr)); CUdeviceptr p;
  CK(cuMemAllocAsync(&p, 256 * MIB, s)); CK(cuMemFreeAsync(p, s)); CK(cuStreamSynchronize(s));
  CK(cuMemPoolGetAttribute(pool, 5, &cur)); kv("reserved_after_free_ge_256mib", "%d", cur >= 256 * MIB);
  if (cur < 256 * MIB) return fail("threshold_not_honoured", 0);
  CK(cuMemPoolTrimTo(pool, 0)); CK(cuMemPoolGetAttribute(pool, 5, &cur)); kv("reserved_after_trim_is_0", "%d", cur == 0);
  return cur == 0 ? PASS : fail("trim_did_not_release", 0);
}
static int c_pool_ipc_export(void) {
  REQ(cuMemPoolCreate, cuMemPoolExportToShareableHandle);
  int ok = (attr(A_POOLTYPES) & 1) != 0; kv("attr_pool_handle_types", "%d", attr(A_POOLTYPES));
  CUmemPoolProps pp; memset(&pp, 0, sizeof pp); pp.allocType = 1; pp.handleTypes = 1; pp.location.type = 1; pp.location.id = dev;
  CUmemoryPool pool; CUresult r = cuMemPoolCreate(&pool, &pp);
  if (r) return judge(ok, r, 0, "cuMemPoolCreate_exportable");
  int fd = -1; r = cuMemPoolExportToShareableHandle(&fd, pool, 1, 0); kv("export_rc", "%d", r);
  return judge(ok, r, fd >= 0, "export_returned_no_fd");
}
// ---------------------------------------------------------------------------------------------------------------- cases: object churn and process fan-out (the observed per-boot CUDA process budget is about 60)
static int c_churn_ctx_200(void) {
  for (int i = 0; i < 200; i++) {
    CUcontext c; CUmodule m; CUfunction f; CUdeviceptr d; unsigned one = 1, v = 0;
    CK(cuCtxCreate_v2(&c, 0, dev)); CK(cuModuleLoadData(&m, PTX)); CK(cuModuleGetFunction(&f, m, "addone")); CK(cuMemAlloc_v2(&d, 4));
    CK(cuMemsetD32Async(d, 0, 1, NULL)); void *a[] = {&d, &one}; CK(launch1(f, 1, 1, NULL, a)); CK(cuCtxSynchronize()); CK(cuMemcpyDtoH_v2(&v, d, 4));
    if (v != 1) return fail("churn_ctx_value", 0);
    CK(cuMemFree_v2(d)); CK(cuModuleUnload(m)); CK(cuCtxDestroy_v2(c));
  }
  kv("cycles", "200"); return PASS;
}
static int c_churn_module_200(void) {
  char *src = malloc(sizeof PTX + 64 * 200);
  for (int i = 0; i < 200; i++) {
    CUmodule m; CUfunction f; CUdeviceptr d; unsigned one = 1, v = 0; char name[32]; snprintf(name, sizeof name, "k%d", i);
    snprintf(src, sizeof PTX + 64, ".version 7.0\n.target sm_75\n.address_size 64\n.visible .entry %s(.param .u64 p, .param .u32 n){ .reg .b32 r<4>; .reg .b64 d<3>;\n"
             " ld.param.u64 d1,[p]; mov.u32 r1,%d; st.u32 [d1],r1; ret; }\n", name, i + 1000);
    CK(cuModuleLoadData(&m, src)); CK(cuModuleGetFunction(&f, m, name)); CK(cuMemAlloc_v2(&d, 4));
    void *a[] = {&d, &one}; CK(launch1(f, 1, 1, NULL, a)); CK(cuCtxSynchronize()); CK(cuMemcpyDtoH_v2(&v, d, 4));
    if (v != (unsigned)(i + 1000)) return fail("churn_module_value", 0);
    CK(cuMemFree_v2(d)); CK(cuModuleUnload(m));
  }
  kv("modules", "200"); return PASS;
}
static int c_churn_streams_events(void) {
  enum { NS = 2048, NE = 4096 }; CUstream *s = calloc(NS, sizeof *s); CUevent *e = calloc(NE, sizeof *e);
  for (int i = 0; i < NS; i++) CK(cuStreamCreate(&s[i], 1));
  for (int i = 0; i < NE; i++) CK(cuEventCreate(&e[i], 0));
  for (int i = 0; i < NE; i++) CK(cuEventRecord(e[i], s[i % NS]));
  CK(cuCtxSynchronize());
  for (int i = 0; i < NE; i++) CK(cuEventDestroy_v2(e[i]));
  for (int i = 0; i < NS; i++) CK(cuStreamDestroy_v2(s[i]));
  kv("streams", "%d", NS); kv("events", "%d", NE); return PASS;
}
static int c_canary(void) {
  unsigned n = 4096; CUdeviceptr d; CK(cuMemAlloc_v2(&d, n * 4)); CK(cuMemsetD32Async(d, 0, n, NULL)); CK(run_addone(d, n, NULL)); CK(cuCtxSynchronize());
  unsigned *h = malloc(n * 4); CK(cuMemcpyDtoH_v2(h, d, n * 4)); for (unsigned i = 0; i < n; i++) if (h[i] != 1) return fail("canary_value", 0);
  return PASS;
}
static int c_fanout_child(const char *hold) { int r = c_canary(); sleep((unsigned)atoi(hold)); return r; }
static int c_fanout_procs_24(void) {
  enum { N = 24 }; pid_t pid[N]; int bad = 0, spawned = 0;
  for (int i = 0; i < N; i++) if (!spawn_self("fanout_child", "4", "-", &pid[i])) spawned++;
  for (int i = 0; i < spawned; i++) if (child_status(pid[i]) != 0) bad++;
  kv("spawned", "%d", spawned); kv("failed", "%d", bad);
  return bad == 0 && spawned == N ? PASS : fail("a_process_could_not_use_the_gpu_while_23_others_did", bad);
}
// ---------------------------------------------------------------------------------------------------------------- cases: long kernels, faults, recovery
static int spin_for(unsigned long long ns) {
  CUdeviceptr flag; CK(cuMemAlloc_v2(&flag, 4)); CK(cuMemsetD32Async(flag, 0, 1, NULL)); void *a[] = {&ns, &flag};
  double t = now(); CK(launch1(f_spin, 1, 1, NULL, a)); CUresult r = cuCtxSynchronize(); double s = now() - t;
  kv("secs", "%d", (int)s); unsigned v = 0; if (r == 0) CK(cuMemcpyDtoH_v2(&v, flag, 4));
  if (r) return fail("a_kernel_that_only_runs_long_was_killed", r);
  return v == 1 && s >= ns / 1e9 * 0.9 ? PASS : fail("spin_flag_or_time", 0);
}
static int c_spin_5s(void) { return spin_for(5000000000ULL); }
static int c_spin_30s(void) { return spin_for(30000000000ULL); }
static int recover_after(CUfunction f, void **params, const char *what) {
  CUresult r = launch1(f, 1, 1, NULL, params); if (!r) r = cuCtxSynchronize();
  kv("fault_rc", "%d", r); if (r == 0) return fail(what, 0);   // a faulting kernel that reports success is a defect of its own
  cuCtxDestroy_v2(ctx); ctx = NULL; CUresult s = cuCtxCreate_v2(&ctx, 0, dev); kv("new_ctx_rc", "%d", s); if (s) return fail("no_new_context_after_fault", s);
  if (cuModuleLoadData(&mod, PTX)) return fail("module_after_fault", 0);
  if (cuModuleGetFunction(&f_addone, mod, "addone")) return fail("symbol_after_fault", 0);
  int c = c_canary(); kv("canary_after", "%s", STATUS[c]); if (c != PASS) return fail("device_unusable_after_fault", RC);
  RC = r; return PASS;
}
static int c_fault_recovery(void) { unsigned long long p = 0; void *a[] = {&p}; return recover_after(f_fault, a, "illegal_store_was_not_reported"); }
static int c_trap_recovery(void) { return recover_after(f_trap, NULL, "trap_was_not_reported"); }
static int c_launch_flood_100k(void) {
  CUdeviceptr d; unsigned zero = 0, v = 0; CK(cuMemAlloc_v2(&d, 4)); CK(cuMemcpyHtoD_v2(d, &zero, 4)); double t = now();
  for (int i = 0; i < 100000; i++) CK(run_addone(d, 1, NULL));
  CK(cuCtxSynchronize()); kv("secs", "%d", (int)(now() - t)); CK(cuMemcpyDtoH_v2(&v, d, 4));
  return v == 100000 ? PASS : fail("flood_count", 0);
}
static int c_kill_target(void) {   // life_kill_* : runs kernels until killed; the harness kills it mid-kernel and then runs the canary
  unsigned long long ns = 2000000000ULL; CUdeviceptr flag; CK(cuMemAlloc_v2(&flag, 4)); void *a[] = {&ns, &flag};
  printf("GAPREADY\n"); fflush(stdout);
  for (;;) { CK(launch1(f_spin, 1, 1, NULL, a)); CK(cuCtxSynchronize()); }
}

// ---------------------------------------------------------------------------------------------------------------- cases: CUPTI (callback API and activity API; libcupti comes from the toolkit at run time, GAP_CUPTI_LIB names it)
typedef void *CUptiSub; typedef void (*CuptiCb)(void *, int, unsigned, const void *);
static int (*cuptiSubscribe)(CUptiSub *, CuptiCb, void *); static int (*cuptiUnsubscribe)(CUptiSub); static int (*cuptiEnableDomain)(unsigned, CUptiSub, int);
static int (*cuptiActivityRegisterCallbacks)(void (*)(uint8_t **, size_t *, size_t *), void (*)(CUcontext, uint32_t, uint8_t *, size_t, size_t));
static int (*cuptiActivityEnable)(int); static int (*cuptiActivityFlushAll)(unsigned); static int (*cuptiActivityGetNextRecord)(uint8_t *, size_t, void **);
static long cb_enter, act_kernels;
static void cupti_cb(void *u, int dom, unsigned id, const void *d) { if (dom == 1 && d && *(const int *)d == 0) cb_enter++; }
static void act_req(uint8_t **b, size_t *sz, size_t *mx) { *b = aligned_alloc(8, 1 << 20); *sz = 1 << 20; *mx = 0; }
static void act_done(CUcontext c, uint32_t sid, uint8_t *b, size_t sz, size_t valid) {
  void *rec = NULL; while (cuptiActivityGetNextRecord(b, valid, &rec) == 0) { int kind = *(int *)rec; if (kind == 3 || kind == 10) act_kernels++; } free(b);
}
static int cupti_open(void) {
  const char *p = getenv("GAP_CUPTI_LIB"); void *h = p && *p ? dlopen(p, RTLD_NOW) : dlopen("libcupti.so.12", RTLD_NOW);
  if (!h) h = dlopen("libcupti.so.13", RTLD_NOW);
  if (!h) { REASON = "no_libcupti"; return NOT_RUN; }
  cuptiSubscribe = dlsym(h, "cuptiSubscribe"); cuptiUnsubscribe = dlsym(h, "cuptiUnsubscribe"); cuptiEnableDomain = dlsym(h, "cuptiEnableDomain");
  cuptiActivityRegisterCallbacks = dlsym(h, "cuptiActivityRegisterCallbacks"); cuptiActivityEnable = dlsym(h, "cuptiActivityEnable");
  cuptiActivityFlushAll = dlsym(h, "cuptiActivityFlushAll"); cuptiActivityGetNextRecord = dlsym(h, "cuptiActivityGetNextRecord");
  if (!cuptiSubscribe || !cuptiEnableDomain || !cuptiActivityEnable || !cuptiActivityGetNextRecord) { REASON = "libcupti_symbols"; return NOT_RUN; }
  return PASS;
}
static int c_cupti_callback(void) {
  int o = cupti_open(); if (o != PASS) return o;
  CUptiSub sub; int r = cuptiSubscribe(&sub, cupti_cb, NULL); kv("subscribe_rc", "%d", r); if (r) return fail("cuptiSubscribe", r);
  r = cuptiEnableDomain(1, sub, 1); kv("enable_domain_rc", "%d", r); if (r) return fail("cuptiEnableDomain", r);
  CUdeviceptr d; CK(cuMemAlloc_v2(&d, 4096)); for (int i = 0; i < 10; i++) CK(run_addone(d, 1024, NULL)); CK(cuCtxSynchronize());
  kv("driver_api_enter_callbacks_ge_10", "%d", cb_enter >= 10); cuptiUnsubscribe(sub);
  return cb_enter >= 10 ? PASS : fail("cupti_saw_fewer_than_10_driver_calls", 0);
}
static int c_cupti_activity(void) {
  int o = cupti_open(); if (o != PASS) return o;
  int r = cuptiActivityRegisterCallbacks(act_req, act_done); kv("register_rc", "%d", r); if (r) return fail("cuptiActivityRegisterCallbacks", r);
  r = cuptiActivityEnable(10); kv("enable_kernel_rc", "%d", r); if (r) return judge(0, r, 0, "cuptiActivityEnable");   // a refusal is an answer: the harness compares it with the host's
  CUdeviceptr d; CK(cuMemAlloc_v2(&d, 4096)); for (int i = 0; i < 10; i++) CK(run_addone(d, 1024, NULL)); CK(cuCtxSynchronize());
  r = cuptiActivityFlushAll(1); kv("flush_rc", "%d", r); kv("kernel_records_ge_10", "%d", act_kernels >= 10);
  return act_kernels >= 10 ? PASS : fail("cupti_activity_returned_fewer_than_10_kernel_records", r);
}

typedef struct { const char *name; int (*fn)(void); int module; } Case;
static const Case CASES[] = {
  {"attr_dump", c_attr_dump, 0}, {"pinned_1g", c_pinned_1g, 0}, {"pinned_4g", c_pinned_4g, 0}, {"host_register_512m", c_host_register_512m, 0},
  {"mapped_zerocopy", c_mapped_zerocopy, 1}, {"pinned_huge_refused", c_pinned_huge_refused, 0}, {"managed_prefetch_advise", c_managed_prefetch_advise, 1},
  {"managed_concurrent", c_managed_concurrent, 1}, {"managed_oversub", c_managed_oversub, 1}, {"ipc_legacy", c_ipc_legacy, 1}, {"ipc_event", c_ipc_event, 0},
  {"ipc_vmm_fd", c_ipc_vmm_fd, 1}, {"vmm_granularity", c_vmm_granularity, 0}, {"vmm_basic", c_vmm_basic, 1}, {"vmm_remap", c_vmm_remap, 1},
  {"graph_capture_launch", c_graph_capture_launch, 1}, {"graph_update", c_graph_update, 1}, {"graph_mem_nodes", c_graph_mem_nodes, 1},
  {"pool_alloc_free", c_pool_alloc_free, 1}, {"pool_threshold_trim", c_pool_threshold_trim, 0}, {"pool_ipc_export", c_pool_ipc_export, 0},
  {"churn_ctx_200", c_churn_ctx_200, 0}, {"churn_module_200", c_churn_module_200, 0}, {"churn_streams_events", c_churn_streams_events, 0},
  {"fanout_procs_24", c_fanout_procs_24, 0}, {"spin_5s", c_spin_5s, 1}, {"spin_30s", c_spin_30s, 1}, {"fault_recovery", c_fault_recovery, 1},
  {"trap_recovery", c_trap_recovery, 1}, {"launch_flood_100k", c_launch_flood_100k, 1}, {"canary", c_canary, 1}, {"kill_target", c_kill_target, 1}, {"cupti_callback", c_cupti_callback, 1}, {"cupti_activity", c_cupti_activity, 1},
};

#ifdef GAP_SELFTEST   // gcc -DGAP_SELFTEST: prints the judge() table, no libcuda needed (scripts/ci/test_gap_probes.py)
int main(void) {
  int t[][4] = {{1, 0, 1, PASS}, {0, 0, 1, MISMATCH}, {0, 801, 0, NOT_SUPPORTED}, {1, 801, 0, FAIL}, {1, 0, 0, FAIL}, {0, 0, 0, FAIL}};
  for (unsigned i = 0; i < sizeof t / sizeof *t; i++) printf("judge attr_ok=%d rc=%d verified=%d -> %s\n", t[i][0], t[i][1], t[i][2], STATUS[judge(t[i][0], t[i][1], t[i][2], "x")]);
  return 0;
}
#else
int main(int argc, char **argv) {
  if (argc >= 2 && !strcmp(argv[1], "list")) { for (unsigned i = 0; i < sizeof CASES / sizeof *CASES; i++) puts(CASES[i].name); return 0; }
  if (argc < 2) { fprintf(stderr, "usage: gap_probe list | <case>\n"); return 2; }
  signal(SIGPIPE, SIG_IGN);
  const char *name = argv[1]; CASE = name; const Case *c = NULL; int child = 0, st;
  for (unsigned i = 0; i < sizeof CASES / sizeof *CASES; i++) if (!strcmp(CASES[i].name, name)) c = &CASES[i];
  if (!strcmp(name, "ipc_child") || !strcmp(name, "ipc_event_child") || !strcmp(name, "ipc_vmm_child") || !strcmp(name, "fanout_child")) child = 1;
  if (!c && !child) { fprintf(stderr, "unknown case %s\n", name); return 2; }
  double t0 = now();
  st = setup(child || c->module);
  if (st == PASS) {
    if (child) {
      st = !strcmp(name, "ipc_child") ? c_ipc_child(argv[2]) : !strcmp(name, "ipc_event_child") ? c_ipc_event_child(argv[2])
         : !strcmp(name, "ipc_vmm_child") ? c_ipc_vmm_child(argv[2], argv[3]) : c_fanout_child(argv[2]);
      if (st != PASS) fprintf(stderr, "child %s: %s rc=%d\n", name, REASON, RC);
      return st == PASS ? 0 : 1;
    }
    st = c->fn();
  }
  printf("GAPPROBE case=%s status=%s rc=%d secs=%.1f reason=%s\n", name, STATUS[st], RC, now() - t0, REASON);
  return 0;
}
#endif
