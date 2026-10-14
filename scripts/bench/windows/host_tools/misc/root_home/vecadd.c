/*
 * vecadd.c -- the rung-5 proof: a real CUDA workload, end to end, inside an OCI
 * container inside a Kata VM, with every ioctl forwarded by nvkvm to the host
 * driver.
 *
 * Deliberately uses the CUDA *driver* API via dlopen("libcuda.so.1") and
 * embedded PTX, so it needs no CUDA toolkit, no nvcc, and no runtime library --
 * only the host driver userspace that CDI mounts into the container.  That
 * keeps the guest rootfs free of CUDA (docs/design/04-guest-kernel.md) and makes
 * the binary trivially portable into any container image with a libc.
 *
 * It touches every layer that could be broken:
 *   cuInit           -> open /dev/nvidiactl, RM_ALLOC of the root client
 *   cuDeviceGet      -> RM_CONTROL enumeration
 *   cuCtxCreate      -> open /dev/nvidiaN, channel + address-space setup
 *   cuMemAlloc       -> RM alloc + map
 *   cuMemcpyHtoD/DtoH-> bulk DMA both directions
 *   cuModuleLoadData -> the PTX JIT (libnvidia-ptxjitcompiler) in the container
 *   cuLaunchKernel   -> doorbell
 *   cuCtxSynchronize -> fence/completion
 * and then CHECKS THE ARITHMETIC, because "it ran" and "it computed" are
 * different claims.
 *
 * Build: gcc -O2 -o vecadd vecadd.c -ldl
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int CUresult;
typedef int CUdevice;
typedef void *CUcontext;
typedef void *CUmodule;
typedef void *CUfunction;
typedef void *CUstream;
typedef unsigned long long CUdeviceptr;

static CUresult (*cuInit)(unsigned);
static CUresult (*cuDeviceGetCount)(int *);
static CUresult (*cuDeviceGet)(CUdevice *, int);
static CUresult (*cuDeviceGetName)(char *, int, CUdevice);
static CUresult (*cuCtxCreate)(CUcontext *, unsigned, CUdevice);
static CUresult (*cuMemAlloc)(CUdeviceptr *, size_t);
static CUresult (*cuMemFree)(CUdeviceptr);
static CUresult (*cuMemcpyHtoD)(CUdeviceptr, const void *, size_t);
static CUresult (*cuMemcpyDtoH)(void *, CUdeviceptr, size_t);
static CUresult (*cuModuleLoadData)(CUmodule *, const void *);
static CUresult (*cuModuleGetFunction)(CUfunction *, CUmodule, const char *);
static CUresult (*cuLaunchKernel)(CUfunction, unsigned, unsigned, unsigned,
                                  unsigned, unsigned, unsigned, unsigned,
                                  CUstream, void **, void **);
static CUresult (*cuCtxSynchronize)(void);

/* c[i] = a[i] + b[i], guarded by n.  compute_52 runs on every GPU nvkvm
 * supports (Maxwell and later) and the driver JITs it forward. */
static const char *ptx =
".version 6.0\n"
".target sm_52\n"
".address_size 64\n"
".visible .entry vecAdd(\n"
"  .param .u64 vecAdd_param_0,\n"
"  .param .u64 vecAdd_param_1,\n"
"  .param .u64 vecAdd_param_2,\n"
"  .param .u32 vecAdd_param_3\n"
")\n"
"{\n"
"  .reg .pred %p<2>;\n"
"  .reg .f32  %f<4>;\n"
"  .reg .b32  %r<6>;\n"
"  .reg .b64  %rd<11>;\n"
"  ld.param.u64 %rd1, [vecAdd_param_0];\n"
"  ld.param.u64 %rd2, [vecAdd_param_1];\n"
"  ld.param.u64 %rd3, [vecAdd_param_2];\n"
"  ld.param.u32 %r2,  [vecAdd_param_3];\n"
"  mov.u32 %r3, %ctaid.x;\n"
"  mov.u32 %r4, %ntid.x;\n"
"  mov.u32 %r5, %tid.x;\n"
"  mad.lo.s32 %r1, %r3, %r4, %r5;\n"
"  setp.ge.s32 %p1, %r1, %r2;\n"
"  @%p1 bra $L__END;\n"
"  cvta.to.global.u64 %rd4, %rd1;\n"
"  mul.wide.s32 %rd5, %r1, 4;\n"
"  add.s64 %rd6, %rd4, %rd5;\n"
"  ld.global.f32 %f1, [%rd6];\n"
"  cvta.to.global.u64 %rd7, %rd2;\n"
"  add.s64 %rd8, %rd7, %rd5;\n"
"  ld.global.f32 %f2, [%rd8];\n"
"  add.f32 %f3, %f1, %f2;\n"
"  cvta.to.global.u64 %rd9, %rd3;\n"
"  add.s64 %rd10, %rd9, %rd5;\n"
"  st.global.f32 [%rd10], %f3;\n"
"$L__END:\n"
"  ret;\n"
"}\n";

#define CK(call) do { CUresult _r = (call); if (_r) { \
    fprintf(stderr, "FAIL %s -> %d\n", #call, _r); return 1; } } while (0)

int main(void)
{
    void *h = dlopen("libcuda.so.1", RTLD_NOW);
    if (!h) h = dlopen("libcuda.so", RTLD_NOW);
    if (!h) { fprintf(stderr, "FAIL dlopen libcuda: %s\n", dlerror()); return 1; }
#define SYM(n) do { *(void **)(&n) = dlsym(h, #n); \
    if (!n) { fprintf(stderr, "FAIL dlsym %s\n", #n); return 1; } } while (0)
    /* prefer the _v2 entry point, fall back to the bare name */
#define SYMV(n) do { *(void **)(&n) = dlsym(h, #n "_v2"); \
    if (!n) *(void **)(&n) = dlsym(h, #n); \
    if (!n) { fprintf(stderr, "FAIL dlsym %s\n", #n); return 1; } } while (0)
    /* The driver API's ABI versioning trap: the UNVERSIONED cuMemAlloc,
     * cuMemFree and cuMemcpy* in libcuda are the pre-CUDA-3.2 entry points that
     * take a 32-bit size and a different context binding.  Calling them after a
     * cuCtxCreate_v2 context yields CUDA_ERROR_INVALID_CONTEXT (201) with
     * nothing to suggest the symbol is the problem.  Anything using dlsym rather
     * than the cuda.h #define shims must ask for _v2 explicitly.  Measured here
     * on bare metal before ever running under nvkvm. */
    SYM(cuInit); SYM(cuDeviceGetCount); SYM(cuDeviceGet); SYM(cuDeviceGetName);
    SYM(cuModuleLoadData); SYM(cuModuleGetFunction); SYM(cuLaunchKernel);
    SYM(cuCtxSynchronize);
    SYMV(cuCtxCreate); SYMV(cuMemAlloc); SYMV(cuMemFree);
    SYMV(cuMemcpyHtoD); SYMV(cuMemcpyDtoH);

    CK(cuInit(0));
    int n_dev = 0; CK(cuDeviceGetCount(&n_dev));
    printf("devices: %d\n", n_dev);
    if (n_dev < 1) { fprintf(stderr, "FAIL no CUDA device\n"); return 1; }
    CUdevice dev; CK(cuDeviceGet(&dev, 0));
    char name[256] = {0}; CK(cuDeviceGetName(name, sizeof name, dev));
    printf("device 0: %s\n", name);

    CUcontext ctx; CK(cuCtxCreate(&ctx, 0, dev));
    CUmodule mod; CK(cuModuleLoadData(&mod, ptx));
    CUfunction fn; CK(cuModuleGetFunction(&fn, mod, "vecAdd"));

    const int N = 1 << 20;
    size_t bytes = (size_t)N * sizeof(float);
    float *ha = malloc(bytes), *hb = malloc(bytes), *hc = malloc(bytes);
    for (int i = 0; i < N; i++) { ha[i] = (float)i; hb[i] = (float)(2 * i); hc[i] = -1.0f; }

    CUdeviceptr da, db, dc;
    CK(cuMemAlloc(&da, bytes)); CK(cuMemAlloc(&db, bytes)); CK(cuMemAlloc(&dc, bytes));
    CK(cuMemcpyHtoD(da, ha, bytes)); CK(cuMemcpyHtoD(db, hb, bytes));

    int nn = N;
    void *args[] = { &da, &db, &dc, &nn };
    CK(cuLaunchKernel(fn, (N + 255) / 256, 1, 1, 256, 1, 1, 0, NULL, args, NULL));
    CK(cuCtxSynchronize());
    CK(cuMemcpyDtoH(hc, dc, bytes));

    long bad = 0;
    for (int i = 0; i < N; i++) if (hc[i] != (float)(3 * i)) bad++;
    printf("elements: %d  mismatches: %ld  sample: c[7]=%.1f (want %.1f)\n",
           N, bad, hc[7], 21.0);
    CK(cuMemFree(da)); CK(cuMemFree(db)); CK(cuMemFree(dc));
    if (bad) { printf("VECADD FAIL\n"); return 1; }
    printf("VECADD OK\n");
    return 0;
}
