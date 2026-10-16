// MEASUREMENT ONLY (perf lifecycle campaign S2, 2026-10-16, scratch, never for merge).
// Async device-to-host copies from a VRAM allocation to pinned host memory, through the CUDA DRIVER API
// (libcuda dlopen'ed; the host has no nvcc, no cuda.h: prototypes are declared here, the gather kernel is
// hand-written PTX embedded below).
//
//   cuda_copy_probe memcpy  <sizes> <iters>                  single cuMemcpyDtoHAsync per read, idle GPU
//   cuda_copy_probe gather  <sizes> <iters> <batch>          one kernel launch gathers <batch> (offset,len) reads
//   cuda_copy_probe rate    <mode memcpy|gather> <size> <batch> <per_sec> <secs>   paced open loop (in-flight ring)
//   cuda_copy_probe caps                                     driver/device facts, symbol presence (cuMemcpyBatchAsync)
//
// Latency = host clock from just before the enqueue call to the host OBSERVING completion by polling
// (cuEventQuery for memcpy; a host-visible counter the kernel increments last for gather).
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sched.h>

typedef int CUresult;
typedef int CUdevice;
typedef void *CUcontext, *CUstream, *CUevent, *CUmodule, *CUfunction;
typedef unsigned long long CUdeviceptr;

static CUresult (*cuInit)(unsigned);
static CUresult (*cuDriverGetVersion)(int *);
static CUresult (*cuDeviceGet)(CUdevice *, int);
static CUresult (*cuDeviceGetName)(char *, int, CUdevice);
static CUresult (*cuDeviceGetAttribute)(int *, int, CUdevice);
static CUresult (*cuCtxCreate)(CUcontext *, unsigned, CUdevice);
static CUresult (*cuMemAlloc)(CUdeviceptr *, size_t);
static CUresult (*cuMemHostAlloc)(void **, size_t, unsigned);
static CUresult (*cuMemHostGetDevicePointer)(CUdeviceptr *, void *, unsigned);
static CUresult (*cuMemsetD8)(CUdeviceptr, unsigned char, size_t);
static CUresult (*cuMemcpyDtoHAsync)(void *, CUdeviceptr, size_t, CUstream);
static CUresult (*cuStreamCreate)(CUstream *, unsigned);
static CUresult (*cuStreamSynchronize)(CUstream);
static CUresult (*cuCtxSynchronize)(void);
static CUresult (*cuEventCreate)(CUevent *, unsigned);
static CUresult (*cuEventRecord)(CUevent, CUstream);
static CUresult (*cuEventQuery)(CUevent);
static CUresult (*cuModuleLoadData)(CUmodule *, const void *);
static CUresult (*cuModuleGetFunction)(CUfunction *, CUmodule, const char *);
typedef struct { int type; int id; } MemLoc; typedef struct { int srcAccessOrder; MemLoc srcLoc; MemLoc dstLoc; unsigned flags; } MemcpyAttr;
static CUresult (*cuMemcpyBatchAsync2)(CUdeviceptr *, CUdeviceptr *, size_t *, size_t, MemcpyAttr *, size_t *, size_t, CUstream);
static CUresult (*cuLaunchKernel)(CUfunction, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, CUstream, void **, void **);

#define CK(x) do { CUresult r_ = (x); if (r_) { fprintf(stderr, "%s failed: %d (line %d)\n", #x, r_, __LINE__); exit(2); } } while (0)

static inline uint64_t now_ns(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return (uint64_t)t.tv_sec * 1000000000ull + t.tv_nsec; }
static int cmpu(const void *a, const void *b) { uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b; return x < y ? -1 : x > y; }
static uint64_t xs = 0x9E3779B97F4A7C15ull;
static uint64_t rnd(void) { xs ^= xs << 13; xs ^= xs >> 7; xs ^= xs << 17; return xs; }

static const char PTX[] =
".version 7.0\n.target sm_70\n.address_size 64\n"
// gather(tab (host-mapped device ptr: u64 src_off, u32 len, u32 dst_off per entry), src_base, dst (host-mapped), flag (host-mapped u32))
".visible .entry gather(.param .u64 tab, .param .u64 src_base, .param .u64 dst, .param .u64 flag)\n"
"{\n"
" .reg .pred %p<4>; .reg .b32 %r<12>; .reg .b64 %rd<20>;\n"
" ld.param.u64 %rd1, [tab]; ld.param.u64 %rd2, [src_base]; ld.param.u64 %rd3, [dst]; ld.param.u64 %rd4, [flag];\n"
" mov.u32 %r1, %ctaid.x; mov.u32 %r2, %tid.x; mov.u32 %r3, %ntid.x;\n"
" mul.wide.u32 %rd5, %r1, 16; add.s64 %rd6, %rd1, %rd5;\n"
" ld.global.u64 %rd7, [%rd6];\n"          // src offset
" ld.global.u32 %r4, [%rd6+8];\n"         // len bytes (multiple of 4)
" ld.global.u32 %r5, [%rd6+12];\n"        // dst offset
" add.s64 %rd8, %rd2, %rd7;\n"
" cvt.u64.u32 %rd9, %r5; add.s64 %rd10, %rd3, %rd9;\n"
" shr.u32 %r6, %r4, 2;\n"                 // words
" mov.u32 %r7, %r2;\n"
"LOOP: setp.ge.u32 %p1, %r7, %r6; @%p1 bra DONE;\n"
" mul.wide.u32 %rd11, %r7, 4; add.s64 %rd12, %rd8, %rd11; add.s64 %rd13, %rd10, %rd11;\n"
" ld.global.u32 %r8, [%rd12]; st.global.u32 [%rd13], %r8;\n"
" add.u32 %r7, %r7, %r3; bra LOOP;\n"
"DONE: membar.sys; bar.sync 0;\n"
" setp.ne.u32 %p2, %r2, 0; @%p2 bra EXIT;\n"
" atom.global.add.u32 %r9, [%rd4], 1;\n"
"EXIT: ret;\n}\n";

static CUdeviceptr dev; static size_t DEVBYTES = 256u << 20;
static CUstream st; static CUfunction fn;
static void *hbuf; static CUdeviceptr hbuf_d;       // pinned destination (host-mapped)
static volatile uint32_t *flag; static CUdeviceptr flag_d;
static uint64_t *tab; static CUdeviceptr tab_d;      // host-mapped table (16 B per entry)

static void setup(void) {
  void *h = dlopen("libcuda.so.1", RTLD_NOW);
  if (!h) { fprintf(stderr, "dlopen libcuda: %s\n", dlerror()); exit(2); }
#define S(n, sym) do { *(void **)&n = dlsym(h, sym); if (!n) { fprintf(stderr, "missing %s\n", sym); exit(2); } } while (0)
  S(cuInit, "cuInit"); S(cuDriverGetVersion, "cuDriverGetVersion"); S(cuDeviceGet, "cuDeviceGet"); S(cuDeviceGetName, "cuDeviceGetName");
  S(cuDeviceGetAttribute, "cuDeviceGetAttribute"); S(cuCtxCreate, "cuCtxCreate_v2"); S(cuMemAlloc, "cuMemAlloc_v2"); S(cuMemHostAlloc, "cuMemHostAlloc");
  S(cuMemHostGetDevicePointer, "cuMemHostGetDevicePointer_v2"); S(cuMemsetD8, "cuMemsetD8_v2"); S(cuMemcpyDtoHAsync, "cuMemcpyDtoHAsync_v2");
  S(cuStreamCreate, "cuStreamCreate"); S(cuStreamSynchronize, "cuStreamSynchronize"); S(cuCtxSynchronize, "cuCtxSynchronize"); S(cuEventCreate, "cuEventCreate");
  S(cuEventRecord, "cuEventRecord"); S(cuEventQuery, "cuEventQuery"); S(cuModuleLoadData, "cuModuleLoadData"); S(cuModuleGetFunction, "cuModuleGetFunction");
  S(cuLaunchKernel, "cuLaunchKernel");
  *(void **)&cuMemcpyBatchAsync2 = dlsym(h, "cuMemcpyBatchAsync_v2");
  CK(cuInit(0));
  int ver; CK(cuDriverGetVersion(&ver));
  CUdevice d; CK(cuDeviceGet(&d, 0)); char name[128]; CK(cuDeviceGetName(name, sizeof name, d));
  printf("CAPS driver_cuda_version=%d device=%s\n", ver, name);
  CUcontext ctx; CK(cuCtxCreate(&ctx, 0, d));
  CK(cuMemAlloc(&dev, DEVBYTES)); CK(cuMemsetD8(dev, 0x5a, DEVBYTES)); CK(cuCtxSynchronize());
  CK(cuStreamCreate(&st, 1 /*non-blocking*/));
  CK(cuMemHostAlloc(&hbuf, 16u << 20, 2 /*DEVICEMAP*/)); CK(cuMemHostGetDevicePointer(&hbuf_d, hbuf, 0));
  void *f; CK(cuMemHostAlloc(&f, 4096, 2)); flag = f; CK(cuMemHostGetDevicePointer(&flag_d, f, 0)); *flag = 0;
  void *t; CK(cuMemHostAlloc(&t, 1u << 20, 2)); tab = t; CK(cuMemHostGetDevicePointer(&tab_d, t, 0));
  CUmodule mod; CK(cuModuleLoadData(&mod, PTX)); CK(cuModuleGetFunction(&fn, mod, "gather"));
}

static void report(const char *tag, int size, int batch, uint64_t *v, int n, uint64_t sub_total, double secs) {
  qsort(v, n, sizeof *v, cmpu);
  uint64_t sum = 0; for (int i = 0; i < n; i++) sum += v[i];
  printf("RESULT %s size=%d batch=%d n=%d p50_us=%.1f p90_us=%.1f p99_us=%.1f max_us=%.1f mean_us=%.1f enqueue_mean_us=%.2f achieved_per_s=%.0f\n", tag, size, batch, n,
         v[n / 2] / 1e3, v[(int)(n * 0.9)] / 1e3, v[(int)(n * 0.99)] / 1e3, v[n - 1] / 1e3, sum / 1e3 / n, sub_total / 1e3 / n, secs > 0 ? n / secs : 0.0);
}

// one single-copy: returns latency ns, enqueue ns in *enq
static uint64_t one_memcpy(int size, CUevent ev, uint64_t *enq) {
  uint64_t off = (rnd() % (DEVBYTES / size)) * size;
  uint64_t t0 = now_ns();
  CK(cuMemcpyDtoHAsync(hbuf, dev + off, size, st));
  CK(cuEventRecord(ev, st));
  uint64_t t1 = now_ns(); *enq = t1 - t0;
  while (cuEventQuery(ev) != 0) { }
  return now_ns() - t0;
}

static uint64_t one_gather(int size, int batch, uint64_t seq, uint64_t *enq) {
  for (int i = 0; i < batch; i++) {
    uint64_t off = (rnd() % (DEVBYTES / size)) * size;
    tab[i * 2] = off; ((uint32_t *)&tab[i * 2 + 1])[0] = size; ((uint32_t *)&tab[i * 2 + 1])[1] = i * size;
  }
  __sync_synchronize();
  void *args[] = { &tab_d, &dev, &hbuf_d, &flag_d };
  uint64_t t0 = now_ns();
  CK(cuLaunchKernel(fn, batch, 1, 1, 128, 1, 1, 0, st, args, NULL));
  uint64_t t1 = now_ns(); *enq = t1 - t0;
  uint32_t want = (uint32_t)(seq * batch);
  while (*flag != want) { }
  return now_ns() - t0;
}

// cuMemcpyBatchAsync_v2 (CUDA 13 signature, no failIdx): <batch> copies of <size> bytes in one call
static uint64_t one_bmemcpy(int size, int batch, CUevent ev, uint64_t *enq) {
  CUdeviceptr d[256], sr[256]; size_t sz[256]; size_t idx[1] = { 0 };
  MemcpyAttr at; memset(&at, 0, sizeof at); at.srcAccessOrder = 1;
  for (int i = 0; i < batch; i++) { uint64_t off = (rnd() % (DEVBYTES / size)) * size; sr[i] = dev + off; d[i] = hbuf_d + (CUdeviceptr)i * size; sz[i] = size; }
  uint64_t t0 = now_ns();
  CUresult r = cuMemcpyBatchAsync2(d, sr, sz, batch, &at, idx, 1, st);
  if (r) { fprintf(stderr, "cuMemcpyBatchAsync_v2 failed: %d\n", r); exit(3); }
  CK(cuEventRecord(ev, st));
  uint64_t t1 = now_ns(); *enq = t1 - t0;
  while (cuEventQuery(ev) != 0) { }
  return now_ns() - t0;
}

int main(int argc, char **argv) {
  if (argc < 2) return 1;
  setup();
  if (!strcmp(argv[1], "caps")) {
    void *h = dlopen("libcuda.so.1", RTLD_NOW);
    printf("CAPS cuMemcpyBatchAsync=%p cuMemcpyBatchAsync_v2=%p cuMemcpyAsync=%p\n", dlsym(h, "cuMemcpyBatchAsync"), dlsym(h, "cuMemcpyBatchAsync_v2"), dlsym(h, "cuMemcpyAsync"));
    int v; CUdevice d; cuDeviceGet(&d, 0);
    cuDeviceGetAttribute(&v, 75, d); printf("CAPS cc_major=%d\n", v);
    cuDeviceGetAttribute(&v, 30, d); printf("CAPS async_engine_count=%d\n", v);
    cuDeviceGetAttribute(&v, 36, d); printf("CAPS mem_clock_khz=%d\n", v);
    cuDeviceGetAttribute(&v, 13, d); printf("CAPS sm_clock_khz=%d\n", v);
    return 0;
  }
  CUevent ev; CK(cuEventCreate(&ev, 0 /*default: blocking-sync off*/));
  if (!strcmp(argv[1], "bmemcpy")) {
    if (!cuMemcpyBatchAsync2) { printf("RESULT bmemcpy unavailable (no cuMemcpyBatchAsync_v2)\n"); return 0; }
    char *sz = strdup(argv[2]); int iters = atoi(argv[3]); int batch = atoi(argv[4]);
    uint64_t *v = malloc(sizeof(uint64_t) * iters);
    for (char *p = strtok(sz, ","); p; p = strtok(NULL, ",")) {
      int size = atoi(p); uint64_t enq, esum = 0;
      for (int i = 0; i < 200; i++) one_bmemcpy(size, batch, ev, &enq);
      for (int i = 0; i < iters; i++) { v[i] = one_bmemcpy(size, batch, ev, &enq); esum += enq; }
      report("bmemcpy", size, batch, v, iters, esum, 0);
    }
    return 0;
  }
  if (!strcmp(argv[1], "memcpy") || !strcmp(argv[1], "gather")) {
    int gather = !strcmp(argv[1], "gather");
    char *sz = strdup(argv[2]); int iters = atoi(argv[3]); int batch = gather ? atoi(argv[4]) : 1;
    uint64_t *v = malloc(sizeof(uint64_t) * iters);
    for (char *p = strtok(sz, ","); p; p = strtok(NULL, ",")) {
      int size = atoi(p); uint64_t enq, esum = 0; uint32_t seq = *flag / (batch ? batch : 1);
      for (int i = 0; i < 200; i++) { if (gather) one_gather(size, batch, ++seq, &enq); else one_memcpy(size, ev, &enq); }
      for (int i = 0; i < iters; i++) { v[i] = gather ? one_gather(size, batch, ++seq, &enq) : one_memcpy(size, ev, &enq); esum += enq; }
      report(gather ? "gather" : "memcpy", size, batch, v, iters, esum, 0);
    }
    return 0;
  }
  if (!strcmp(argv[1], "rate")) {
    int gather = !strcmp(argv[2], "gather"); int size = atoi(argv[3]); int batch = gather ? atoi(argv[4]) : 1;
    double per_sec = atof(argv[5]), secs = atof(argv[6]);
    int cap = (int)(per_sec * secs * 1.2) + 1000;
    uint64_t *lat = malloc(sizeof(uint64_t) * cap); int n = 0; uint64_t esum = 0, missed = 0;
    // in-flight ring of events (memcpy mode) / sequence numbers (gather mode); one stream, so completions are in order
    enum { K = 16 };
    CUevent evs[K]; for (int i = 0; i < K; i++) CK(cuEventCreate(&evs[i], 0));
    uint64_t t_enq[K]; int head = 0, tail = 0; uint32_t seq = *flag / (batch ? batch : 1);
    uint64_t period = (uint64_t)(1e9 / per_sec), t_start = now_ns(), next = t_start, end = t_start + (uint64_t)(secs * 1e9);
    while (now_ns() < end || head != tail) {
      uint64_t t = now_ns();
      // retire oldest completed
      while (head != tail) {
        int s = tail % K;
        if (gather) { uint32_t first = seq - (head - tail) + 1; if (*flag >= first * batch) { if (n < cap) lat[n++] = now_ns() - t_enq[s]; tail++; continue; } }
        else if (cuEventQuery(evs[s]) == 0) { if (n < cap) lat[n++] = now_ns() - t_enq[s]; tail++; continue; }
        break;
      }
      if (t >= next && t < end) {
        if (head - tail < K) {
          int s = head % K; uint64_t e0 = now_ns(); t_enq[s] = e0;
          if (gather) {
            // one table region (64 entries) per in-flight slot
            for (int i = 0; i < batch; i++) { uint64_t *e = &tab[(s * 64 + i) * 2]; e[0] = (rnd() % (DEVBYTES / size)) * size; ((uint32_t *)&e[1])[0] = size; ((uint32_t *)&e[1])[1] = (s * 64 + i) * size; }
            __sync_synchronize();
            CUdeviceptr td = tab_d + (CUdeviceptr)(s * 64) * 16; void *args[] = { &td, &dev, &hbuf_d, &flag_d };
            CK(cuLaunchKernel(fn, batch, 1, 1, 128, 1, 1, 0, st, args, NULL)); seq++;
          } else {
            uint64_t off = (rnd() % (DEVBYTES / size)) * size;
            CK(cuMemcpyDtoHAsync((char *)hbuf + (size_t)s * size, dev + off, size, st)); CK(cuEventRecord(evs[s], st));
          }
          esum += now_ns() - e0; head++;
        } else missed++;
        next += period;
      }
    }
    double el = (now_ns() - t_start) / 1e9;
    report(gather ? "rate-gather" : "rate-memcpy", size, batch, lat, n, esum, el);
    printf("RATE target_per_s=%.0f secs=%.1f dropped_ticks_ring_full=%llu\n", per_sec, secs, (unsigned long long)missed);
    return 0;
  }
  return 1;
}
