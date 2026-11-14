/* cuda_drv_probe.c -- byte-exact CUDA DRIVER-API probe. STATUS: LIVE, 2026-11-12.
 *
 * Needs no CUDA toolkit and no nvcc: libcuda is dlopen()ed, the types are declared here, and the kernel is PTX
 * that the DRIVER JIT-compiles (so a driver without its JIT, CUDA error 221, is a named failure). The PTX targets
 * sm_75 (the lowest supported consumer die), so the same text loads on sm_75 .. sm_120.
 * Steps, each one a CHECK line (every check is byte-exact against a value computed on the CPU, never captured):
 *   cuda_init     cuInit, driver version, device 0, name, compute capability
 *   cuda_ctx      cuCtxCreate
 *   cuda_roundtrip 8 MiB LCG pattern: HtoD, DtoH, memcmp
 *   cuda_memset   cuMemsetD8 0xA5 over 1 MiB, then 0x00 over the first half; DtoH, memcmp
 *   cuda_ptx_jit  cuModuleLoadData(PTX); 221 is reported as "JIT compiler missing"
 *   cuda_kernel   y[i] = (x[i]*3u + 7u) ^ i over 1M uint32; every element compared
 *   cuda_teardown cuMemFree, cuModuleUnload, cuCtxDestroy all return CUDA_SUCCESS
 * Failures print the CUresult NAME (cuGetErrorName) and number. Teardown is the normal one: main() returns,
 * it never calls _exit(), so a crash in the driver's teardown shows up as a non-zero exit status.
 * Exit 0 only when every check is ok; the last line is CUDA_DRV_PROBE_DONE rc=<0|1> checks=<ok>/<all>.
 * Build: gcc -O2 cuda_drv_probe.c -o cuda_drv_probe -ldl
 */
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int CUresult;
typedef int CUdevice;
typedef struct CUctx_st *CUcontext;
typedef struct CUmod_st *CUmodule;
typedef struct CUfunc_st *CUfunction;
typedef unsigned long long CUdeviceptr;

#define SYMS(X) \
  X(CUresult, cuInit, (unsigned)) \
  X(CUresult, cuDriverGetVersion, (int*)) \
  X(CUresult, cuDeviceGet, (CUdevice*, int)) \
  X(CUresult, cuDeviceGetName, (char*, int, CUdevice)) \
  X(CUresult, cuDeviceGetAttribute, (int*, int, CUdevice)) \
  X(CUresult, cuCtxCreate_v2, (CUcontext*, unsigned, CUdevice)) \
  X(CUresult, cuCtxDestroy_v2, (CUcontext)) \
  X(CUresult, cuCtxSynchronize, (void)) \
  X(CUresult, cuMemAlloc_v2, (CUdeviceptr*, size_t)) \
  X(CUresult, cuMemFree_v2, (CUdeviceptr)) \
  X(CUresult, cuMemcpyHtoD_v2, (CUdeviceptr, const void*, size_t)) \
  X(CUresult, cuMemcpyDtoH_v2, (void*, CUdeviceptr, size_t)) \
  X(CUresult, cuMemsetD8_v2, (CUdeviceptr, unsigned char, size_t)) \
  X(CUresult, cuModuleLoadData, (CUmodule*, const void*)) \
  X(CUresult, cuModuleUnload, (CUmodule)) \
  X(CUresult, cuModuleGetFunction, (CUfunction*, CUmodule, const char*)) \
  X(CUresult, cuLaunchKernel, (CUfunction, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, unsigned, void*, void**, void**)) \
  X(CUresult, cuGetErrorName, (CUresult, const char**))
#define DECL(R, N, A) static R (*N) A;
SYMS(DECL)

static int nchecks, nok;
static void check(const char *name, int ok, const char *detail) {
    nchecks++; nok += ok;
    printf("CHECK %s %s%s%s\n", name, ok ? "ok" : "FAIL", detail[0] ? " " : "", detail);
}
static const char *errname(CUresult r) {
    static char buf[96]; const char *n = 0;
    if (cuGetErrorName && cuGetErrorName(r, &n) == 0 && n) snprintf(buf, sizeof buf, "%s (%d)", n, r);
    else snprintf(buf, sizeof buf, "CUresult %d", r);
    if (r == 221) strncat(buf, " JIT compiler missing", sizeof buf - strlen(buf) - 1);
    return buf;
}
static int fail(const char *name, const char *what, CUresult r) {
    char d[200]; snprintf(d, sizeof d, "%s: %s", what, errname(r)); check(name, 0, d); return 1;
}
static uint32_t lcg(uint32_t *s) { *s = *s * 1664525u + 1013904223u; return *s; }

static const char PTX[] =
    ".version 6.3\n.target sm_75\n.address_size 64\n"
    ".visible .entry kern(.param .u64 px, .param .u64 py, .param .u32 pn)\n{\n"
    "  .reg .pred %p; .reg .b32 %r<8>; .reg .b64 %rd<8>;\n"
    "  ld.param.u64 %rd1, [px]; ld.param.u64 %rd2, [py]; ld.param.u32 %r1, [pn];\n"
    "  mov.u32 %r2, %ctaid.x; mov.u32 %r3, %ntid.x; mov.u32 %r4, %tid.x;\n"
    "  mad.lo.s32 %r5, %r2, %r3, %r4;\n"
    "  setp.ge.u32 %p, %r5, %r1; @%p bra DONE;\n"
    "  mul.wide.u32 %rd3, %r5, 4; add.s64 %rd4, %rd1, %rd3; add.s64 %rd5, %rd2, %rd3;\n"
    "  ld.global.u32 %r6, [%rd4];\n"
    "  mad.lo.u32 %r7, %r6, 3, 7;\n"
    "  xor.b32 %r7, %r7, %r5;\n"
    "  st.global.u32 [%rd5], %r7;\n"
    "DONE: ret;\n}\n";

#define NBYTES (8u << 20)
#define NEL (1u << 20)

int main(void) {
    void *lib = dlopen("libcuda.so.1", RTLD_NOW);
    if (!lib) { check("cuda_init", 0, "dlopen libcuda.so.1 failed"); goto done; }
#define LOAD(R, N, A) N = dlsym(lib, #N); if (!N && strcmp(#N, "cuGetErrorName")) { char d[80]; snprintf(d, sizeof d, "missing symbol %s", #N); check("cuda_init", 0, d); goto done; }
    SYMS(LOAD)
    CUresult r; CUcontext ctx = 0; CUmodule mod = 0; CUdeviceptr d1 = 0, d2 = 0; CUdevice dev;
    uint8_t *a = 0, *b = 0; uint32_t *x = 0, *y = 0;
    if ((r = cuInit(0))) { fail("cuda_init", "cuInit", r); goto done; }
    int ver = 0, cc_major = 0, cc_minor = 0; char name[128] = "";
    if ((r = cuDriverGetVersion(&ver)) || (r = cuDeviceGet(&dev, 0)) || (r = cuDeviceGetName(name, sizeof name, dev))
        || (r = cuDeviceGetAttribute(&cc_major, 75, dev)) || (r = cuDeviceGetAttribute(&cc_minor, 76, dev))) {
        fail("cuda_init", "device query", r); goto done; }
    { char d[200]; snprintf(d, sizeof d, "driver_api=%d device=%s sm_%d%d", ver, name, cc_major, cc_minor);
      check("cuda_init", ver > 0 && cc_major >= 7 && name[0], d); }
    if ((r = cuCtxCreate_v2(&ctx, 0, dev))) { fail("cuda_ctx", "cuCtxCreate", r); goto done; }
    check("cuda_ctx", 1, "");

    /* 8 MiB round trip */
    a = malloc(NBYTES); b = malloc(NBYTES);
    uint32_t s = 12345; for (size_t i = 0; i < NBYTES; i += 4) { uint32_t v = lcg(&s); memcpy(a + i, &v, 4); }
    if ((r = cuMemAlloc_v2(&d1, NBYTES)) || (r = cuMemcpyHtoD_v2(d1, a, NBYTES)) || (r = cuMemcpyDtoH_v2(b, d1, NBYTES))) {
        fail("cuda_roundtrip", "8 MiB copy", r); goto teardown; }
    check("cuda_roundtrip", !memcmp(a, b, NBYTES), "8 MiB");

    /* memset: 0xA5 over 1 MiB, then 0x00 over the first half (exact bytes both sides of the boundary) */
    memset(a, 0xA5, 1 << 20); memset(a, 0x00, 1 << 19); memset(b, 0x77, 1 << 20);
    if ((r = cuMemsetD8_v2(d1, 0xA5, 1 << 20)) || (r = cuMemsetD8_v2(d1, 0x00, 1 << 19)) || (r = cuMemcpyDtoH_v2(b, d1, 1 << 20))) {
        fail("cuda_memset", "cuMemsetD8", r); goto teardown; }
    check("cuda_memset", !memcmp(a, b, 1 << 20), "");

    /* PTX JIT */
    if ((r = cuModuleLoadData(&mod, PTX))) { fail("cuda_ptx_jit", "cuModuleLoadData(PTX)", r); goto teardown; }
    CUfunction fn;
    if ((r = cuModuleGetFunction(&fn, mod, "kern"))) { fail("cuda_ptx_jit", "cuModuleGetFunction", r); goto teardown; }
    check("cuda_ptx_jit", 1, "");

    /* kernel: y[i] = (x[i]*3+7) ^ i, every element */
    x = malloc(NEL * 4); y = malloc(NEL * 4);
    s = 777; for (uint32_t i = 0; i < NEL; i++) x[i] = lcg(&s);
    unsigned n = NEL; CUdeviceptr dx, dy;
    if ((r = cuMemAlloc_v2(&d2, 2ull * NEL * 4))) { fail("cuda_kernel", "cuMemAlloc", r); goto teardown; }
    dx = d2; dy = d2 + (CUdeviceptr)NEL * 4;
    void *args[] = { &dx, &dy, &n };
    memset(y, 0xEE, NEL * 4);
    if ((r = cuMemcpyHtoD_v2(dx, x, NEL * 4)) || (r = cuMemsetD8_v2(dy, 0xEE, NEL * 4))
        || (r = cuLaunchKernel(fn, NEL / 256, 1, 1, 256, 1, 1, 0, 0, args, 0)) || (r = cuCtxSynchronize())
        || (r = cuMemcpyDtoH_v2(y, dy, NEL * 4))) { fail("cuda_kernel", "launch/sync/copy", r); goto teardown; }
    { uint32_t bad = 0, first = 0; for (uint32_t i = 0; i < NEL; i++) if (y[i] != ((x[i] * 3u + 7u) ^ i)) { if (!bad) first = i; bad++; }
      char d[100]; snprintf(d, sizeof d, "%u elements, %u wrong (first at %u)", NEL, bad, first);
      check("cuda_kernel", bad == 0, d); }

teardown: {
    CUresult t1 = 0, t2 = 0, t3 = 0, t4 = 0;
    if (d2) t1 = cuMemFree_v2(d2);
    if (d1) t2 = cuMemFree_v2(d1);
    if (mod) t3 = cuModuleUnload(mod);
    if (ctx) t4 = cuCtxDestroy_v2(ctx);
    char d[200] = ""; CUresult bad = t1 ? t1 : t2 ? t2 : t3 ? t3 : t4;
    if (bad) snprintf(d, sizeof d, "free/unload/destroy: %s", errname(bad));
    check("cuda_teardown", !bad, d);
  }
done:
    printf("CUDA_DRV_PROBE_DONE rc=%d checks=%d/%d\n", nok == nchecks && nchecks >= 7 ? 0 : 1, nok, nchecks);
    fflush(stdout);
    return nok == nchecks && nchecks >= 7 ? 0 : 1;
}
