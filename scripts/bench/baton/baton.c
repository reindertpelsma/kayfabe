/*
 * baton.c - GPU time-slice fairness probe (libc + dlopen(libcuda.so.1) only; no nvcc).
 *
 * One process = one CUDA context.  It loops launch + synchronize of a spin kernel that
 * occupies the whole GPU for ~K ms, and records per-iteration start/duration.  Run two
 * (or more) concurrently to see whether one starves the other (a "baton"): the starvation
 * metric is the longest gap between successive iteration COMPLETIONS.
 *
 *   gcc -O2 -o baton baton.c -ldl
 *   ./baton --duration 60 --label A                      (see README.md for the full recipes)
 *
 * Output (stdout): header lines '# ...', then 'sec iterations max_iter_ms' one line per
 * second (bucketed by completion time), then 'SUMMARY k=v ...' and SLOW_CSV_BEGIN/END with
 * every iteration longer than --slow-ms (default 50).  --dump-all FILE writes all iterations.
 * Times are seconds since t0 (the start of the timed loop); t0_epoch is CLOCK_REALTIME.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

/* ---- minimal CUDA driver API surface (ABI-stable types) ---- */
typedef int CUresult;
typedef int CUdevice;
typedef struct CUctx_st *CUcontext;
typedef struct CUmod_st *CUmodule;
typedef struct CUfunc_st *CUfunction;
typedef struct CUstream_st *CUstream;

#define CU_CTX_SCHED_SPIN          0x01
#define CU_CTX_SCHED_BLOCKING_SYNC 0x04

static CUresult (*p_cuInit)(unsigned);
static CUresult (*p_cuDeviceGet)(CUdevice *, int);
static CUresult (*p_cuDeviceGetName)(char *, int, CUdevice);
static CUresult (*p_cuCtxCreate)(CUcontext *, unsigned, CUdevice);
static CUresult (*p_cuModuleLoadData)(CUmodule *, const void *);
static CUresult (*p_cuModuleGetFunction)(CUfunction *, CUmodule, const char *);
static CUresult (*p_cuLaunchKernel)(CUfunction, unsigned, unsigned, unsigned, unsigned, unsigned,
                                    unsigned, unsigned, CUstream, void **, void **);
static CUresult (*p_cuStreamCreate)(CUstream *, unsigned);
static CUresult (*p_cuMemAlloc)(unsigned long long *, size_t);
static CUresult (*p_cuStreamSynchronize)(CUstream);
static CUresult (*p_cuGetErrorString)(CUresult, const char **);
static CUresult (*p_cuDriverGetVersion)(int *);

/* Hand-written PTX: spin on %clock64 until `cycles` SM cycles have elapsed. */
static const char PTX[] =
    ".version 6.3\n"
    ".target sm_75\n"
    ".address_size 64\n"
    ".visible .entry spin(.param .u64 cycles, .param .u64 sink)\n"
    "{\n"
    "  .reg .u64 %rd<6>;\n"
    "  .reg .pred %p<2>;\n"
    "  ld.param.u64 %rd1, [cycles];\n"
    "  mov.u64 %rd2, %clock64;\n"
    "LOOP:\n"
    "  mov.u64 %rd3, %clock64;\n"
    "  sub.u64 %rd4, %rd3, %rd2;\n"
    "  setp.lt.u64 %p1, %rd4, %rd1;\n"
    "  @%p1 bra LOOP;\n"
    "  ret;\n"
    "}\n"
    /* burn: a fixed amount of dependent integer work per thread.  Unlike spin, its run time is
     * GPU occupancy time: clock64 keeps counting while a kernel is time-sliced out, so a spin
     * kernel's duration is wall time and overstates work done under sharing.  The conditional
     * store (never taken) keeps ptxas from deleting the loop. */
    ".visible .entry burn(.param .u64 iters, .param .u64 sink)\n"
    "{\n"
    "  .reg .u64 %rd<6>;\n"
    "  .reg .u32 %r<6>;\n"
    "  .reg .pred %p<3>;\n"
    "  ld.param.u64 %rd1, [iters];\n"
    "  ld.param.u64 %rd5, [sink];\n"
    "  mov.u32 %r1, %tid.x;\n"
    "  mov.u32 %r2, %ctaid.x;\n"
    "  mad.lo.u32 %r3, %r2, 1000003, %r1;\n"
    "  mov.u64 %rd2, 0;\n"
    "BLOOP:\n"
    "  mad.lo.u32 %r3, %r3, 1664525, 1013904223;\n"
    "  add.u64 %rd2, %rd2, 1;\n"
    "  setp.lt.u64 %p1, %rd2, %rd1;\n"
    "  @%p1 bra BLOOP;\n"
    "  setp.eq.u32 %p2, %r3, 305419896;\n"
    "  @%p2 st.global.u32 [%rd5], %r3;\n"
    "  ret;\n"
    "}\n";

static volatile sig_atomic_t g_stop;
static void on_term(int s) { (void)s; g_stop = 1; }

static double mono(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (double)t.tv_sec + (double)t.tv_nsec * 1e-9;
}
static double real(void) {
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    return (double)t.tv_sec + (double)t.tv_nsec * 1e-9;
}

static void die_cu(const char *what, CUresult r) {
    const char *s = NULL;
    if (p_cuGetErrorString) p_cuGetErrorString(r, &s);
    fprintf(stderr, "baton: %s failed: %d (%s)\n", what, r, s ? s : "?");
    exit(2);
}
#define CK(x) do { CUresult _r = (x); if (_r) die_cu(#x, _r); } while (0)

static void *sym(void *h, const char *n, int required) {
    void *p = dlsym(h, n);
    if (!p && required) { fprintf(stderr, "baton: missing symbol %s\n", n); exit(2); }
    return p;
}

static CUfunction g_fn;
static unsigned long long g_sink;
static CUstream g_stream;
static unsigned g_blocks = 128, g_threads = 256;

static double one_launch_ms(unsigned long long cycles) {
    void *args[] = { &cycles, &g_sink };
    double a = mono();
    CK(p_cuLaunchKernel(g_fn, g_blocks, 1, 1, g_threads, 1, 1, 0, g_stream, args, NULL));
    CK(p_cuStreamSynchronize(g_stream));
    return (mono() - a) * 1e3;
}

static int cmp_d(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}
static double median_launch_ms(unsigned long long cycles, int n) {
    double v[32];
    if (n > 32) n = 32;
    for (int i = 0; i < n; i++) v[i] = one_launch_ms(cycles);
    qsort(v, n, sizeof v[0], cmp_d);
    return v[n / 2];
}

/* cycles per millisecond of kernel body, solo, launch overhead removed */
static double calibrate(void) {
    for (int i = 0; i < 5; i++) one_launch_ms(1000000ULL); /* JIT + warm-up */
    double ovh = median_launch_ms(0, 15);
    unsigned long long c = 2000000ULL;
    double cpm = 1e6;
    for (int round = 0; round < 4; round++) {
        double t = median_launch_ms(c, 7);
        double body = t - ovh;
        if (body < 0.05) body = 0.05;
        cpm = (double)c / body;
        c = (unsigned long long)(cpm * 20.0); /* aim at ~20 ms for the next, better-resolved round */
    }
    fprintf(stderr, "baton: launch overhead %.3f ms, %.0f cycles/ms\n", ovh, cpm);
    return cpm;
}

typedef struct { double start, dur_ms, done; } Iter;

static void usage(void) {
    fprintf(stderr,
        "usage: baton [--duration S] [--k-ms K] [--label L] [--cycles-per-ms X] [--calibrate-only]\n"
        "             [--kernel burn|clock]  (burn = fixed work, default; clock = spin on %%clock64)\n"
        "             [--start-at EPOCH_SEC] [--dump-all FILE] [--slow-ms MS] [--blocks N]\n"
        "             [--threads N] [--device N] [--spin]\n");
    exit(1);
}

int main(int argc, char **argv) {
    double duration = 10, k_ms = 5, cpm_override = 0, start_at = 0, slow_ms = 50;
    const char *label = "A", *dump_path = NULL;
    int calibrate_only = 0, device = 0, spin = 0;
    const char *kernel = "burn";
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
#define ARG(name) (!strcmp(a, name) && i + 1 < argc)
        if (ARG("--duration")) duration = atof(argv[++i]);
        else if (ARG("--k-ms")) k_ms = atof(argv[++i]);
        else if (ARG("--label")) label = argv[++i];
        else if (ARG("--cycles-per-ms")) cpm_override = atof(argv[++i]);
        else if (ARG("--start-at")) start_at = atof(argv[++i]);
        else if (ARG("--dump-all")) dump_path = argv[++i];
        else if (ARG("--slow-ms")) slow_ms = atof(argv[++i]);
        else if (ARG("--blocks")) g_blocks = (unsigned)atoi(argv[++i]);
        else if (ARG("--threads")) g_threads = (unsigned)atoi(argv[++i]);
        else if (ARG("--kernel")) kernel = argv[++i];
        else if (ARG("--device")) device = atoi(argv[++i]);
        else if (!strcmp(a, "--calibrate-only")) calibrate_only = 1;
        else if (!strcmp(a, "--spin")) spin = 1;
        else usage();
    }
    if (duration <= 0 || k_ms <= 0) usage();
    if (strcmp(kernel, "burn") && strcmp(kernel, "clock")) usage();

    void *h = dlopen("libcuda.so.1", RTLD_NOW);
    if (!h) { fprintf(stderr, "baton: dlopen libcuda.so.1: %s\n", dlerror()); return 2; }
    p_cuInit = sym(h, "cuInit", 1);
    p_cuDeviceGet = sym(h, "cuDeviceGet", 1);
    p_cuDeviceGetName = sym(h, "cuDeviceGetName", 0);
    p_cuCtxCreate = sym(h, "cuCtxCreate_v2", 1);
    p_cuModuleLoadData = sym(h, "cuModuleLoadData", 1);
    p_cuModuleGetFunction = sym(h, "cuModuleGetFunction", 1);
    p_cuLaunchKernel = sym(h, "cuLaunchKernel", 1);
    p_cuStreamCreate = sym(h, "cuStreamCreate", 1);
    p_cuMemAlloc = sym(h, "cuMemAlloc_v2", 1);
    p_cuStreamSynchronize = sym(h, "cuStreamSynchronize", 1);
    p_cuGetErrorString = sym(h, "cuGetErrorString", 0);
    p_cuDriverGetVersion = sym(h, "cuDriverGetVersion", 0);

    CUdevice dev; CUcontext ctx; CUmodule mod;
    CK(p_cuInit(0));
    CK(p_cuDeviceGet(&dev, device));
    CK(p_cuCtxCreate(&ctx, spin ? CU_CTX_SCHED_SPIN : CU_CTX_SCHED_BLOCKING_SYNC, dev));
    CK(p_cuModuleLoadData(&mod, PTX));
    CK(p_cuModuleGetFunction(&g_fn, mod, strcmp(kernel, "clock") ? "burn" : "spin"));
    CK(p_cuMemAlloc(&g_sink, 256));
    CK(p_cuStreamCreate(&g_stream, 0));

    char name[128] = "?"; int drv = 0;
    if (p_cuDeviceGetName) p_cuDeviceGetName(name, sizeof name, dev);
    if (p_cuDriverGetVersion) p_cuDriverGetVersion(&drv);

    double cpm = cpm_override > 0 ? cpm_override : calibrate();
    if (calibrate_only) { printf("CYCLES_PER_MS=%.0f\n", cpm); return 0; }
    unsigned long long cycles = (unsigned long long)(cpm * k_ms + 0.5);

    signal(SIGTERM, on_term);
    signal(SIGINT, on_term);
    if (start_at > 0) {
        while (!g_stop && real() < start_at) {
            double left = start_at - real();
            if (left > 0.002) usleep(left > 0.1 ? 100000 : (useconds_t)((left - 0.001) * 1e6));
        }
    }

    size_t cap = 1 << 16, n = 0;
    Iter *it = malloc(cap * sizeof *it);
    FILE *dump = dump_path ? fopen(dump_path, "w") : NULL;
    if (dump) fprintf(dump, "idx,start_s,dur_ms,done_s\n");

    double m0 = mono(), r0 = real();
    printf("# baton v1\n");
    printf("# label=%s pid=%d k_ms=%.3f cycles=%llu grid=%ux%u duration_s=%g kernel=%s spin=%d device=\"%s\" driver=%d\n",
           label, (int)getpid(), k_ms, cycles, g_blocks, g_threads, duration, kernel, spin,
           name, drv);
    printf("# t0_epoch=%.6f\n", r0);
    printf("sec iterations max_iter_ms\n");
    fflush(stdout);

    int cur_sec = 0; long cur_cnt = 0; double cur_max = 0;
    double busy = 0, gap = 0, gap_end = 0, prev_done = 0, first_done = -1;
    long zero_sec = 0;
    int nsec_window = (int)duration + ((double)(int)duration < duration ? 1 : 0);

    for (;;) {
        double s = mono() - m0;
        if (g_stop || s >= duration) break;
        void *args[] = { &cycles, &g_sink };
        CK(p_cuLaunchKernel(g_fn, g_blocks, 1, 1, g_threads, 1, 1, 0, g_stream, args, NULL));
        CK(p_cuStreamSynchronize(g_stream));
        double d = mono() - m0;
        double dur = (d - s) * 1e3;
        if (n == cap) { cap *= 2; it = realloc(it, cap * sizeof *it); if (!it) return 3; }
        it[n].start = s; it[n].dur_ms = dur; it[n].done = d; n++;
        if (dump) fprintf(dump, "%zu,%.6f,%.3f,%.6f\n", n - 1, s, dur, d);
        busy += dur;
        if (first_done < 0) first_done = d * 1e3;
        if (d - prev_done > gap) { gap = d - prev_done; gap_end = d; }
        prev_done = d;
        int sec = (int)d;
        while (cur_sec < sec) { /* flush finished seconds (including empty ones) */
            printf("%d %ld %.3f\n", cur_sec, cur_cnt, cur_max);
            if (cur_cnt == 0 && cur_sec < nsec_window) zero_sec++;
            cur_sec++; cur_cnt = 0; cur_max = 0;
            if (dump) fflush(dump);
            fflush(stdout);
        }
        cur_cnt++;
        if (dur > cur_max) cur_max = dur;
    }
    /* the last (possibly partial) second */
    printf("%d %ld %.3f\n", cur_sec, cur_cnt, cur_max);
    if (cur_cnt == 0 && cur_sec < nsec_window) zero_sec++;
    for (int s = cur_sec + 1; s < nsec_window; s++) { printf("%d 0 0.000\n", s); zero_sec++; }

    printf("SUMMARY iterations=%zu busy_ms=%.3f work_cycles=%llu longest_gap_ms=%.3f longest_gap_end_s=%.3f "
           "first_done_ms=%.3f zero_sec=%ld mean_iter_ms=%.3f iters_per_s=%.3f aborted=%d\n",
           n, busy, (unsigned long long)n * cycles, gap * 1e3, gap_end, first_done < 0 ? 0.0 : first_done,
           zero_sec, n ? busy / (double)n : 0.0, (double)n / duration, (int)g_stop);
    printf("SLOW_CSV_BEGIN idx,start_s,dur_ms,done_s\n");
    for (size_t i = 0; i < n; i++)
        if (it[i].dur_ms > slow_ms)
            printf("%zu,%.3f,%.3f,%.3f\n", i, it[i].start, it[i].dur_ms, it[i].done);
    printf("SLOW_CSV_END\n");
    fflush(stdout);
    if (dump) fclose(dump);
    return 0;
}
