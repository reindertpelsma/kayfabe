/* gpuprobe — is one tenant's GPU wedge containable?
 *
 * Uses the CUDA *driver* API (libcuda.so ships with the driver) + hand-written
 * PTX, so no CUDA toolkit is needed.  Three kernels in one module:
 *   spin   — reads a global flag forever; the flag is never written  => WEDGE
 *   burn   — a long but TERMINATING loop                            => CONTROL (busy, not wedged)
 *   addone — trivial, verifiable work                               => VICTIM (the other tenant)
 *
 * The control matters: without it, "the victim was slow" reads as "the victim
 * was wedged".  A boolean witness cannot attribute.
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <dlfcn.h>

typedef int CUresult;
typedef int CUdevice;
typedef void *CUcontext;
typedef void *CUmodule;
typedef void *CUfunction;
typedef unsigned long long CUdeviceptr;

static CUresult (*p_cuInit)(unsigned);
static CUresult (*p_cuDeviceGet)(CUdevice *, int);
static CUresult (*p_cuCtxCreate)(CUcontext *, unsigned, CUdevice);
static CUresult (*p_cuModuleLoadData)(CUmodule *, const void *);
static CUresult (*p_cuModuleLoadDataEx)(CUmodule *, const void *, unsigned,
                                        int *, void **);
static CUresult (*p_cuModuleGetFunction)(CUfunction *, CUmodule, const char *);
static CUresult (*p_cuMemAlloc)(CUdeviceptr *, size_t);
static CUresult (*p_cuMemcpyHtoD)(CUdeviceptr, const void *, size_t);
static CUresult (*p_cuMemcpyDtoH)(void *, CUdeviceptr, size_t);
static CUresult (*p_cuLaunchKernel)(CUfunction, unsigned, unsigned, unsigned,
                                    unsigned, unsigned, unsigned, unsigned,
                                    void *, void **, void **);
static CUresult (*p_cuCtxSynchronize)(void);
static CUresult (*p_cuGetErrorName)(CUresult, const char **);

static const char *PTX =
".version 7.1\n"   /* sm_86 needs >= 7.1; 7.0 is rejected by ptxas */
".target sm_86\n"
".address_size 64\n"
".visible .entry spin(.param .u64 p)\n"
"{\n"
"  .reg .pred %p1;\n"
"  .reg .b32 %r1;\n"
"  .reg .b64 %rd1, %rd2;\n"
"  ld.param.u64 %rd1, [p];\n"
"  cvta.to.global.u64 %rd2, %rd1;\n"
"L1:\n"
"  ld.global.volatile.u32 %r1, [%rd2];\n"
"  setp.eq.s32 %p1, %r1, 0;\n"
"  @%p1 bra L1;\n"
"  ret;\n"
"}\n"
".visible .entry burn(.param .u64 p, .param .u32 n)\n"
"{\n"
"  .reg .pred %p1;\n"
"  .reg .b32 %r1, %r2, %r3;\n"
"  .reg .b64 %rd1, %rd2;\n"
"  ld.param.u64 %rd1, [p];\n"
"  ld.param.u32 %r2, [n];\n"
"  cvta.to.global.u64 %rd2, %rd1;\n"
"  mov.u32 %r1, 0;\n"
"  mov.u32 %r3, 0;\n"
"L2:\n"
"  add.u32 %r3, %r3, %r1;\n"
"  add.u32 %r1, %r1, 1;\n"
"  setp.lt.u32 %p1, %r1, %r2;\n"
"  @%p1 bra L2;\n"
"  st.global.u32 [%rd2], %r3;\n"
"  ret;\n"
"}\n"
/* A kernel that FAULTS: stores through a wild, unmapped device VA.  This is the
 * shape a hang is not — it produces a real MMU fault and an Xid, which is the
 * only way to reach RM's channel-recovery/escalation path. */
".visible .entry fault(.param .u64 p)\n"
"{\n"
"  .reg .b32 %r1;\n"
"  .reg .b64 %rd1;\n"
"  mov.u64 %rd1, 0x0000700000000000;\n"
"  mov.u32 %r1, 3735928559;\n"
"  st.global.u32 [%rd1], %r1;\n"
"  ret;\n"
"}\n"
".visible .entry addone(.param .u64 p)\n"
"{\n"
"  .reg .b32 %r1, %r2, %r3;\n"
"  .reg .b64 %rd1, %rd2, %rd3, %rd4;\n"
"  ld.param.u64 %rd1, [p];\n"
"  cvta.to.global.u64 %rd2, %rd1;\n"
"  mov.u32 %r1, %tid.x;\n"
"  mul.wide.u32 %rd3, %r1, 4;\n"
"  add.s64 %rd4, %rd2, %rd3;\n"
"  ld.global.u32 %r2, [%rd4];\n"
"  add.s32 %r3, %r2, 1;\n"
"  st.global.u32 [%rd4], %r3;\n"
"  ret;\n"
"}\n";

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}

#define CK(call, what)                                                        \
    do {                                                                      \
        CUresult _r = (call);                                                 \
        if (_r != 0) {                                                        \
            const char *_n = "?";                                             \
            if (p_cuGetErrorName) p_cuGetErrorName(_r, &_n);                  \
            printf("FAIL %s rc=%d (%s) t=%.2f\n", what, _r, _n, now() - t0);  \
            fflush(stdout);                                                   \
            exit(20);                                                         \
        }                                                                     \
    } while (0)

static double t0;

int main(int argc, char **argv) {
    const char *mode = argc > 1 ? argv[1] : "victim";
    t0 = now();

    void *h = dlopen("libcuda.so.1", RTLD_NOW);
    if (!h) { printf("FAIL dlopen %s\n", dlerror()); return 21; }
#define SYM(v, n) *(void **)(&p_##v) = dlsym(h, n); if (!p_##v) { printf("FAIL dlsym %s\n", n); return 22; }
    SYM(cuInit, "cuInit");
    SYM(cuDeviceGet, "cuDeviceGet");
    SYM(cuCtxCreate, "cuCtxCreate_v2");
    SYM(cuModuleLoadData, "cuModuleLoadData");
    SYM(cuModuleLoadDataEx, "cuModuleLoadDataEx");
    SYM(cuModuleGetFunction, "cuModuleGetFunction");
    SYM(cuMemAlloc, "cuMemAlloc_v2");
    SYM(cuMemcpyHtoD, "cuMemcpyHtoD_v2");
    SYM(cuMemcpyDtoH, "cuMemcpyDtoH_v2");
    SYM(cuLaunchKernel, "cuLaunchKernel");
    SYM(cuCtxSynchronize, "cuCtxSynchronize");
    *(void **)(&p_cuGetErrorName) = dlsym(h, "cuGetErrorName");

    CUdevice dev; CUcontext ctx; CUmodule mod;
    CK(p_cuInit(0), "cuInit");
    CK(p_cuDeviceGet(&dev, 0), "cuDeviceGet");
    printf("[%s] ctx-create t=%.2f\n", mode, now() - t0); fflush(stdout);
    CK(p_cuCtxCreate(&ctx, 0, dev), "cuCtxCreate");
    printf("[%s] ctx-ok t=%.2f\n", mode, now() - t0); fflush(stdout);
    {   /* load with a JIT error log, so a bad PTX says WHY */
        static char elog[8192];
        int opts[2] = { 5 /*ERROR_LOG_BUFFER*/, 6 /*ERROR_LOG_BUFFER_SIZE*/ };
        void *vals[2] = { elog, (void *)(size_t)sizeof elog };
        CUresult r = p_cuModuleLoadDataEx(&mod, PTX, 2, opts, vals);
        if (r != 0) {
            printf("FAIL ptx rc=%d\n--- JIT LOG ---\n%s\n--- END ---\n", r, elog);
            return 23;
        }
    }

    CUdeviceptr d;
    CK(p_cuMemAlloc(&d, 4096), "cuMemAlloc");
    unsigned host[64];
    memset(host, 0, sizeof host);
    CK(p_cuMemcpyHtoD(d, host, sizeof host), "HtoD");

    if (!strcmp(mode, "spin")) {
        CUfunction f; CK(p_cuModuleGetFunction(&f, mod, "spin"), "getfn spin");
        void *a[] = { &d };
        /* ★ Occupancy is the whole point: a 1x32 spin leaves 27 of GA106's 28
         * SMs free, so a victim can run beside it without any preemption being
         * involved.  Default to saturating the device. */
        unsigned grid  = argc > 2 ? (unsigned)strtoul(argv[2], 0, 10) : 224;
        unsigned block = argc > 3 ? (unsigned)strtoul(argv[3], 0, 10) : 1024;
        printf("[spin] LAUNCH grid=%u block=%u (never terminates) t=%.2f\n",
               grid, block, now() - t0); fflush(stdout);
        CK(p_cuLaunchKernel(f, grid,1,1, block,1,1, 0, 0, a, 0), "launch spin");
        CK(p_cuCtxSynchronize(), "sync spin");
        printf("[spin] RETURNED — did not wedge t=%.2f\n", now() - t0);
        return 0;
    }

    if (!strcmp(mode, "burn")) {
        CUfunction f; CK(p_cuModuleGetFunction(&f, mod, "burn"), "getfn burn");
        unsigned n = argc > 2 ? (unsigned)strtoul(argv[2], 0, 10) : 2000000000u;
        void *a[] = { &d, &n };
        printf("[burn] LAUNCH n=%u (terminates) t=%.2f\n", n, now() - t0); fflush(stdout);
        CK(p_cuLaunchKernel(f, 1,1,1, 32,1,1, 0, 0, a, 0), "launch burn");
        CK(p_cuCtxSynchronize(), "sync burn");
        printf("[burn] DONE t=%.2f\n", now() - t0);
        return 0;
    }

    if (!strcmp(mode, "fault")) {
        CUfunction f; CK(p_cuModuleGetFunction(&f, mod, "fault"), "getfn fault");
        void *a[] = { &d };
        printf("[fault] LAUNCH wild store t=%.2f\n", now() - t0); fflush(stdout);
        CUresult lr = p_cuLaunchKernel(f, 1,1,1, 32,1,1, 0, 0, a, 0);
        CUresult sr = p_cuCtxSynchronize();
        const char *ln = "?", *sn = "?";
        if (p_cuGetErrorName) { p_cuGetErrorName(lr, &ln); p_cuGetErrorName(sr, &sn); }
        printf("[fault] launch rc=%d (%s) sync rc=%d (%s) t=%.2f\n",
               lr, ln, sr, sn, now() - t0);
        /* Does this context survive its own fault?  A sticky context is the
         * expected CUDA behaviour; what matters for containment is the OTHER
         * process, which the harness checks separately. */
        CUresult again = p_cuCtxSynchronize();
        printf("[fault] context reusable? rc=%d %s\n", again,
               again == 0 ? "YES" : "NO (sticky, as expected)");
        return 0;
    }

    if (!strcmp(mode, "loop")) {
        /* A victim that HOLDS A LIVE CONTEXT across the attacker's fault —
         * the arm that matters, because a fresh process would silently get a
         * fresh context and hide a context-scoped kill. */
        CUfunction f; CK(p_cuModuleGetFunction(&f, mod, "addone"), "getfn addone");
        int secs = argc > 2 ? atoi(argv[2]) : 30;
        void *a[] = { &d };
        int iter = 0, ok = 0, bad = 0, err = 0;
        printf("[loop] holding one context for %d s t=%.2f\n", secs, now() - t0);
        fflush(stdout);
        while (now() - t0 < secs) {
            for (int i = 0; i < 64; i++) host[i] = i;
            CUresult r = p_cuMemcpyHtoD(d, host, sizeof host);
            if (!r) r = p_cuLaunchKernel(f, 1,1,1, 64,1,1, 0, 0, a, 0);
            if (!r) r = p_cuCtxSynchronize();
            if (!r) r = p_cuMemcpyDtoH(host, d, sizeof host);
            iter++;
            if (r) {
                const char *n = "?";
                if (p_cuGetErrorName) p_cuGetErrorName(r, &n);
                err++;
                printf("[loop] iter=%d ERROR rc=%d (%s) t=%.2f\n", iter, r, n, now() - t0);
                fflush(stdout);
                if (err > 3) break;   /* context is dead; stop hammering */
            } else {
                int b = 0;
                for (int i = 0; i < 64; i++) if (host[i] != (unsigned)i + 1) b++;
                if (b) { bad++; printf("[loop] iter=%d WRONG bad=%d\n", iter, b); }
                else ok++;
            }
        }
        printf("[loop] DONE iters=%d ok=%d wrong=%d errors=%d t=%.2f\n",
               iter, ok, bad, err, now() - t0);
        return err ? 4 : (bad ? 3 : 0);
    }

    /* victim: a real, verified computation by an independent tenant */
    CUfunction f; CK(p_cuModuleGetFunction(&f, mod, "addone"), "getfn addone");
    for (int i = 0; i < 64; i++) host[i] = i;
    CK(p_cuMemcpyHtoD(d, host, sizeof host), "HtoD victim");
    void *a[] = { &d };
    double l0 = now();
    CK(p_cuLaunchKernel(f, 1,1,1, 64,1,1, 0, 0, a, 0), "launch addone");
    CK(p_cuCtxSynchronize(), "sync addone");
    double l1 = now();
    memset(host, 0, sizeof host);
    CK(p_cuMemcpyDtoH(host, d, sizeof host), "DtoH victim");
    int bad = 0;
    for (int i = 0; i < 64; i++) if (host[i] != (unsigned)i + 1) bad++;
    printf("[victim] %s bad=%d kernel_ms=%.1f total_s=%.2f\n",
           bad ? "WRONG" : "OK", bad, (l1 - l0) * 1000.0, now() - t0);
    return bad ? 3 : 0;
}
