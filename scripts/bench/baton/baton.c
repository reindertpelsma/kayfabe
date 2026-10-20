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
#include <dirent.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <glob.h>
#include <pthread.h>
#include <sys/mman.h>
#include <sys/stat.h>
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
static CUresult (*p_cuStreamQuery)(CUstream);
static CUresult (*p_cuCtxSetCurrent)(CUcontext);

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

/* ---- self-watchdog (second thread): if no iteration completes for --wd-ms, dump the guest-side
 * picture (format documented in wd_parse.py) to --wd-out.  Reads only /proc; no strace. ---- */
typedef struct { volatile double last_real; volatile unsigned long long iters; } Hb;
static Hb *g_hb;                 /* this client's mmap'd heartbeat (shared via --hb-dir) */
static const char *g_hb_dir, *g_label = "A", *g_wd_path;
static double g_wd_ms = 2000, g_m0, g_r0;
static CUcontext g_ctx;
static volatile unsigned long long g_iters, g_launched;
static volatile int g_sync_active, g_done;
static volatile double g_last_done_m;

static void slurp(const char *path, char *buf, size_t cap) {
    int fd = open(path, O_RDONLY);
    buf[0] = 0;
    if (fd < 0) { snprintf(buf, cap, "<err %d>", errno); return; }
    ssize_t n = read(fd, buf, cap - 1);
    close(fd);
    if (n < 0) { snprintf(buf, cap, "<err %d>", errno); return; }
    buf[n] = 0;
    while (n > 0 && (buf[n - 1] == '\n')) buf[--n] = 0;
}

static void dump_proc_file(FILE *o, const char *path) {
    char b[4096];
    slurp(path, b, sizeof b);
    if (!strncmp(b, "<err", 4) && strstr(b, " 21>")) return; /* directory */
    fprintf(o, "PROC %s\n%s\nENDPROC\n", path, b);
}

static void dump_tree(FILE *o, const char *dir, int depth) {
    DIR *d = opendir(dir);
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d))) {
        if (e->d_name[0] == '.') continue;
        char p[512];
        snprintf(p, sizeof p, "%s/%s", dir, e->d_name);
        struct stat st;
        if (stat(p, &st)) continue;
        if (S_ISDIR(st.st_mode)) { if (depth > 0) dump_tree(o, p, depth - 1); }
        else if (S_ISREG(st.st_mode)) dump_proc_file(o, p);
    }
    closedir(d);
}

static void watchdog_dump(FILE *o, double idle_ms, int again) {
    char b[4096], p[512];
    double now = real();
    fprintf(o, "WD_BEGIN label=%s pid=%d t=%.3f epoch=%.3f idle_ms=%.1f iters=%llu launched=%llu sync_active=%d again=%d\n",
            g_label, (int)getpid(), mono() - g_m0, now, idle_ms, g_iters, g_launched, g_sync_active, again);
    if (g_hb_dir) {
        DIR *d = opendir(g_hb_dir);
        struct dirent *e;
        while (d && (e = readdir(d))) {
            if (strncmp(e->d_name, "hb.", 3) || !strcmp(e->d_name + 3, g_label)) continue;
            snprintf(p, sizeof p, "%s/%s", g_hb_dir, e->d_name);
            int fd = open(p, O_RDONLY);
            Hb h;
            if (fd >= 0 && pread(fd, &h, sizeof h, 0) == (ssize_t)sizeof h)
                fprintf(o, "HB label=%s age_ms=%.1f iters=%llu\n", e->d_name + 3, (now - h.last_real) * 1e3, h.iters);
            if (fd >= 0) close(fd);
        }
        if (d) closedir(d);
    }
    if (p_cuStreamQuery) {
        CUresult r = p_cuStreamQuery(g_stream);
        fprintf(o, "STREAMQ rc=%d name=%s\n", r, r == 0 ? "SUCCESS" : r == 600 ? "NOT_READY" : "ERROR");
    }
    DIR *d = opendir("/proc/self/task");
    struct dirent *e;
    while (d && (e = readdir(d))) {
        if (e->d_name[0] == '.') continue;
        char comm[64], wch[128], sc[160], stat[512], *st_state;
        snprintf(p, sizeof p, "/proc/self/task/%s/comm", e->d_name); slurp(p, comm, sizeof comm);
        snprintf(p, sizeof p, "/proc/self/task/%s/wchan", e->d_name); slurp(p, wch, sizeof wch);
        snprintf(p, sizeof p, "/proc/self/task/%s/syscall", e->d_name); slurp(p, sc, sizeof sc);
        snprintf(p, sizeof p, "/proc/self/task/%s/stat", e->d_name); slurp(p, stat, sizeof stat);
        st_state = strrchr(stat, ')'); st_state = st_state ? st_state + 2 : stat;
        long nr = -1; char a0[40] = "", a1[40] = "", a2[40] = "";
        if (sscanf(sc, "%ld %39s %39s %39s", &nr, a0, a1, a2) < 1) nr = -1;
        fprintf(o, "TASK tid=%s comm=%s state=%c wchan=%s syscall=%ld args=%s,%s,%s\n", e->d_name, comm,
                *st_state, wch[0] ? wch : "0", nr, a0[0] ? a0 : "-", a1[0] ? a1 : "-", a2[0] ? a2 : "-");
        fprintf(o, "STAT tid=%s %s\n", e->d_name, stat);
        snprintf(p, sizeof p, "/proc/self/task/%s/stack", e->d_name); slurp(p, b, sizeof b);
        fprintf(o, "STACK tid=%s\n%s\nENDSTACK\n", e->d_name, b);
        snprintf(p, sizeof p, "/proc/self/task/%s/status", e->d_name); slurp(p, b, sizeof b);
        char *c = strstr(b, "voluntary_ctxt_switches");
        fprintf(o, "CTXT tid=%s %s\n", e->d_name, c ? c : "?");
    }
    if (d) closedir(d);
    d = opendir("/proc/self/fd");
    while (d && (e = readdir(d))) {
        if (e->d_name[0] == '.') continue;
        char t[256]; ssize_t n;
        snprintf(p, sizeof p, "/proc/self/fd/%s", e->d_name);
        n = readlink(p, t, sizeof t - 1);
        if (n > 0) { t[n] = 0; fprintf(o, "FD %s -> %s\n", e->d_name, t); }
    }
    if (d) closedir(d);
    dump_tree(o, "/proc/driver/nvidia/gpus", 3);
    dump_tree(o, "/proc/driver/nvidia-uvm", 3);
    dump_proc_file(o, "/proc/driver/nvidia/warnings");
    slurp("/proc/interrupts", b, sizeof b);
    fprintf(o, "PROC /proc/interrupts(nvidia|virtio)\n");
    { char *l = strtok(b, "\n"); while (l) { if (strstr(l, "nvidia") || strstr(l, "virtio") || strstr(l, "CPU0")) fprintf(o, "%s\n", l); l = strtok(NULL, "\n"); } }
    fprintf(o, "ENDPROC\n");
    dump_proc_file(o, "/proc/loadavg");
    fprintf(o, "WD_END label=%s\n", g_label);
    fflush(o);
}

static void *watchdog_main(void *arg) {
    (void)arg;
    FILE *o = fopen(g_wd_path, "a");
    if (!o) return NULL;
    if (p_cuCtxSetCurrent) p_cuCtxSetCurrent(g_ctx);
    unsigned long long seen = g_iters;
    double last = g_m0, episode_start = 0, next_dump = 0;
    int in_ep = 0;
    while (!g_done) {
        usleep(100000);
        unsigned long long it = g_iters;
        double nowm = mono();
        if (it != seen) {
            if (in_ep) fprintf(o, "WD_RECOVER label=%s t=%.3f gap_ms=%.1f\n", g_label, g_last_done_m - g_m0, (g_last_done_m - last) * 1e3), fflush(o);
            in_ep = 0; seen = it;
            continue;
        }
        last = g_last_done_m > g_m0 ? g_last_done_m : g_m0;
        double idle = (nowm - last) * 1e3;
        if (idle >= g_wd_ms && (!in_ep || nowm >= next_dump)) {
            watchdog_dump(o, idle, in_ep);
            if (!in_ep) episode_start = nowm;
            in_ep = 1; next_dump = nowm + 10.0;
        }
    }
    (void)episode_start;
    fclose(o);
    return NULL;
}

static void hb_open(void) {
    if (!g_hb_dir) return;
    char p[512];
    snprintf(p, sizeof p, "%s/hb.%s", g_hb_dir, g_label);
    int fd = open(p, O_RDWR | O_CREAT, 0644);
    if (fd < 0) return;
    if (ftruncate(fd, sizeof(Hb))) { close(fd); return; }
    g_hb = mmap(NULL, sizeof(Hb), PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (g_hb == MAP_FAILED) g_hb = NULL;
    close(fd);
}

static void usage(void) {
    fprintf(stderr,
        "usage: baton [--duration S] [--k-ms K] [--label L] [--cycles-per-ms X] [--calibrate-only]\n"
        "             [--kernel burn|clock]  (burn = fixed work, default; clock = spin on %%clock64)\n"
        "             [--start-at EPOCH_SEC] [--dump-all FILE] [--slow-ms MS] [--blocks N]\n"
        "             [--threads N] [--device N] [--spin] [--burst N]\n"
        "             [--wd-ms MS (2000, 0=off)] [--wd-out FILE] [--hb-dir DIR] [--sleep-ms MS (after each iteration)]\n");
    exit(1);
}

int main(int argc, char **argv) {
    double duration = 10, k_ms = 5, cpm_override = 0, start_at = 0, slow_ms = 50;
    const char *label = "A", *dump_path = NULL;
    int calibrate_only = 0, device = 0, spin = 0, burst = 1;
    double sleep_ms = 0;
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
        else if (ARG("--burst")) burst = atoi(argv[++i]);
        else if (ARG("--sleep-ms")) sleep_ms = atof(argv[++i]);
        else if (ARG("--wd-ms")) g_wd_ms = atof(argv[++i]);
        else if (ARG("--wd-out")) g_wd_path = argv[++i];
        else if (ARG("--hb-dir")) g_hb_dir = argv[++i];
        else if (!strcmp(a, "--calibrate-only")) calibrate_only = 1;
        else if (!strcmp(a, "--spin")) spin = 1;
        else usage();
    }
    if (duration <= 0 || k_ms <= 0 || burst < 1) usage();
    g_label = label;
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
    p_cuStreamQuery = sym(h, "cuStreamQuery", 0);
    p_cuCtxSetCurrent = sym(h, "cuCtxSetCurrent", 0);

    CUdevice dev; CUcontext ctx; CUmodule mod;
    CK(p_cuInit(0));
    CK(p_cuDeviceGet(&dev, device));
    CK(p_cuCtxCreate(&ctx, spin ? CU_CTX_SCHED_SPIN : CU_CTX_SCHED_BLOCKING_SYNC, dev));
    g_ctx = ctx;
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
    g_m0 = m0; g_r0 = r0;
    hb_open();
    pthread_t wdt; int wd_on = 0;
    if (g_wd_ms > 0 && g_wd_path && pthread_create(&wdt, NULL, watchdog_main, NULL) == 0) wd_on = 1;
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
        for (int b = 0; b < burst; b++) {
            CK(p_cuLaunchKernel(g_fn, g_blocks, 1, 1, g_threads, 1, 1, 0, g_stream, args, NULL));
            g_launched++;
        }
        g_sync_active = 1;
        CK(p_cuStreamSynchronize(g_stream));
        g_sync_active = 0;
        double d = mono() - m0;
        g_iters++; g_last_done_m = m0 + d;
        if (g_hb) { g_hb->iters = g_iters; g_hb->last_real = r0 + d; }
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
        if (sleep_ms > 0) usleep((useconds_t)(sleep_ms * 1000.0));
    }
    g_done = 1;
    if (wd_on) pthread_join(wdt, NULL);
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
