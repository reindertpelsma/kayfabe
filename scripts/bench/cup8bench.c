/* ★★★★★ w311 cup8bench.c — THE THROUGHPUT INSTRUMENT. Derived from scripts/bench/cup8.c.
 *
 * ## What this measures and why it is not cup8
 *
 * `cup8` answered CORRECTNESS at scale (w308: N=2048, bad=0, maxerr=0, bit-exact). It runs
 * ONE launch and reports one wall time, so its 39 s figure is a one-shot number that mixes
 * cuInit, ctx creation, PTX JIT, allocation, publication, two copies, one launch and a
 * host-side O(N^2) verify. ⊘ THAT NUMBER IS NOT A THROUGHPUT FIGURE and must never be quoted
 * as one.
 *
 * An LLM is not one matmul: it is thousands of launches over memory that is ALREADY
 * RESIDENT. So this program separates the three costs that a one-shot run fuses:
 *
 *   1. STARTUP        — cuInit / ctx / module / alloc / first H2D. Paid ONCE per process.
 *   2. FIRST LAUNCH   — carries publication + first-touch backing. Reported SEPARATELY.
 *   3. STEADY STATE   — iterations 1..I-1 over the SAME buffers. ★ THE LLM-RELEVANT NUMBER.
 *
 * and it reports a DISTRIBUTION (min / median / p90 / max), never a single mean, because the
 * bench is shared and one stall must not be readable as the cost of the plane.
 *
 * ## ★★★ THE CORRECTNESS ASSERTION IS INSIDE THE TIMED LOOP, and it is a REAL falsifier
 *
 * A benchmark that stops checking its output benchmarks the wrong thing. Two specific ways
 * that goes wrong here, and what this program does about each:
 *
 *  - ⊘⊘ **A launch that does NOTHING would still verify** if C simply kept the previous
 *    iteration's correct contents. So C is POISONED with 0xDEADBEEF before EVERY launch
 *    (not zeroed: a zero fill is indistinguishable from an unbacked/zeroed leaf, which is a
 *    diagnosis we want to keep separate). A no-op launch therefore reads back as poison and
 *    is counted `bad`. The poison write is OUTSIDE the timed window and is followed by an
 *    explicit sync, so its cost cannot leak into the launch measurement.
 *  - The readback + O(N^2) verify are likewise OUTSIDE the timed window but are still run
 *    every `BENCH_VERIFY`-th iteration (default: EVERY one), and every mismatch is counted.
 *
 * The verifier is cup8's: A[i][k]=(i&3)+1, B[k][j]=(j&3)+1, both independent of k, so
 * C[i][j] = N*((i&3)+1)*((j&3)+1) EXACTLY in fp32. The kernel still loads every element of
 * A and B, so a backing hole anywhere shows up as a wrong value.
 * ⚠ Unlike cup8.c there is NO early-exit guard on the verify loop: `bad` here IS a whole-
 *   matrix mismatch total and `maxerr` IS a whole-matrix maximum. cup8's `bad<8` guard made
 *   its numbers partial, which had to be caveated in every report of it.
 *
 * ## ★★ THE BATCH PHASE — the measurement that separates SUBMIT cost from COMPLETION cost
 *
 * After the per-launch loop, K launches are enqueued back-to-back with a SINGLE sync at the
 * end. If batched per-launch cost ≈ solo per-launch cost, the time is in the ENGINE. If it
 * is far lower, the time is in the per-launch SUBMIT/COMPLETION round trip — which is
 * exactly the plane we forward, and exactly what an LLM would amortise away. ⇒ the two
 * numbers together say whether the LLM target is reachable by batching alone.
 *
 * ## ⊘ NO cuda.h — DELIBERATE, and it is what makes the ratio a ratio
 *
 * The bench host has libcuda but NO CUDA toolkit, while the guest image has both. Including
 * <cuda.h> would mean the two arms are built from different headers by different toolchains,
 * and the whole deliverable here is a GUEST ÷ NATIVE ratio. The CUDA Driver API is a stable
 * C ABI, so the dozen entry points used are declared here and linked with -lcuda. ⇒ ONE
 * source, ONE compiler invocation, BOTH arms. The v2 symbol names are the ones <cuda.h>
 * macro-maps to, spelled out rather than #define'd.
 *
 *   build: gcc -O2 -o cup8bench cup8bench.c -lcuda -lm
 *   env:   BENCH_SIZES (default "1024,2048")  BENCH_ITERS (20)  BENCH_BATCH (10)
 *          BENCH_VERIFY (1 = verify every iteration; k = every k-th)
 */
#define _POSIX_C_SOURCE 200809L
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <time.h>
#include <sys/resource.h>

/* ---- CUDA Driver API, declared locally (see header note) ------------------------------ */
typedef unsigned long long CUdeviceptr;
typedef int   CUdevice;
typedef void *CUcontext;
typedef void *CUmodule;
typedef void *CUfunction;
typedef void *CUstream;
#define CUDA_SUCCESS 0
extern int cuInit(unsigned int);
extern int cuDeviceGetCount(int *);
extern int cuDeviceGet(CUdevice *, int);
extern int cuDeviceGetName(char *, int, CUdevice);
extern int cuCtxCreate_v2(CUcontext *, unsigned int, CUdevice);
extern int cuModuleLoadData(CUmodule *, const void *);
extern int cuModuleGetFunction(CUfunction *, CUmodule, const char *);
extern int cuMemAlloc_v2(CUdeviceptr *, size_t);
/* w320 BENCH_HOSTMEM: pinned HOST memory, mapped into the device's address space. The kernel
 * then reaches its operands over PCIe instead of out of VRAM — which is the arrangement our
 * guest's buffers are alleged to be in. */
extern int cuMemHostAlloc(void **, size_t, unsigned int);
extern int cuMemHostGetDevicePointer_v2(CUdeviceptr *, void *, unsigned int);
extern int cuMemFreeHost(void *);
extern int cuMemFree_v2(CUdeviceptr);
extern int cuMemcpyHtoD_v2(CUdeviceptr, const void *, size_t);
extern int cuMemcpyDtoH_v2(void *, CUdeviceptr, size_t);
extern int cuMemsetD32_v2(CUdeviceptr, unsigned int, size_t);
extern int cuLaunchKernel(CUfunction, unsigned, unsigned, unsigned,
                          unsigned, unsigned, unsigned,
                          unsigned, CUstream, void **, void **);
extern int cuCtxSynchronize(void);
extern int cuGetErrorString(int, const char **);

#define CK(x) do{ int r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)
/* quiet variant for calls inside the loop — an `ok` line per iteration would swamp the log */
#define CQ(x) do{ int r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} }while(0)

/* ⊘ BYTE-IDENTICAL to the PTX in scripts/bench/cup8.c (and to the C artifact's cup8.c).
 *   The kernel is NOT what this rung varies; only the harness around it is. */
static const char *PTX =
".version 7.8\n.target sm_86\n.address_size 64\n"
".visible .entry mm(.param .u64 pC,.param .u64 pA,.param .u64 pB,.param .u32 pN){\n"
"  .reg .pred %p<4>; .reg .f32 %f<5>; .reg .b32 %r<16>; .reg .b64 %rd<10>;\n"
"  ld.param.u64 %rd1,[pC]; ld.param.u64 %rd2,[pA]; ld.param.u64 %rd3,[pB];\n"
"  ld.param.u32 %r1,[pN];\n"
"  cvta.to.global.u64 %rd1,%rd1; cvta.to.global.u64 %rd2,%rd2; cvta.to.global.u64 %rd3,%rd3;\n"
"  mov.u32 %r2,%ntid.x; mov.u32 %r3,%ctaid.x; mov.u32 %r4,%tid.x; mad.lo.s32 %r5,%r3,%r2,%r4;\n"
"  mov.u32 %r6,%ntid.y; mov.u32 %r7,%ctaid.y; mov.u32 %r8,%tid.y; mad.lo.s32 %r9,%r7,%r6,%r8;\n"
"  setp.ge.u32 %p1,%r5,%r1; setp.ge.u32 %p2,%r9,%r1; or.pred %p3,%p1,%p2; @%p3 bra $L_ret;\n"
"  mov.f32 %f1,0f00000000; mov.u32 %r10,0;\n"
"$L_loop:\n"
"  setp.ge.u32 %p1,%r10,%r1; @%p1 bra $L_done;\n"
"  mad.lo.s32 %r11,%r9,%r1,%r10;\n"
"  mul.wide.u32 %rd4,%r11,4; add.s64 %rd5,%rd2,%rd4; ld.global.f32 %f2,[%rd5];\n"
"  mad.lo.s32 %r12,%r10,%r1,%r5;\n"
"  mul.wide.u32 %rd6,%r12,4; add.s64 %rd7,%rd3,%rd6; ld.global.f32 %f3,[%rd7];\n"
"  fma.rn.f32 %f1,%f2,%f3,%f1;\n"
"  add.s32 %r10,%r10,1; bra $L_loop;\n"
"$L_done:\n"
"  mad.lo.s32 %r13,%r9,%r1,%r5;\n"
"  mul.wide.u32 %rd8,%r13,4; add.s64 %rd9,%rd1,%rd8; st.global.f32 [%rd9],%f1;\n"
"$L_ret:\n"
"  ret;\n}\n";

/* 0xDEADBEEF as fp32 is about -6.26e18 — never a legitimate C value, and DISTINCT from the
 * zero an unbacked/zeroed leaf would read back as. The two diagnoses stay separable. */
#define POISON 0xDEADBEEFu

static double now_ms(void){
    struct timespec ts; clock_gettime(CLOCK_MONOTONIC,&ts);
    return ts.tv_sec*1000.0 + ts.tv_nsec/1e6;
}
/* ★★★ w315 — THE SECOND CLOCK, AND IT IS HERE ONLY TO BE CORRELATED, NEVER TO TIME.
 * Every duration in this file is CLOCK_MONOTONIC. CLOCK_REALTIME is read exactly twice
 * (origin and end) so a reader can (a) place this run on a wall clock beside the host's
 * device log and (b) SEE the two clocks' relative rate rather than assume it. ⊘ A duration
 * is never computed from it: a realtime step would silently become a "measurement".
 * ⚠ The guest's realtime is not the host's; the correspondence this licenses is a SEQUENCE
 * alignment (launch i ↔ the doorbell cluster between t0_mono[i] and t0_mono[i]+launch_ms),
 * not an offset anyone may subtract. */
static double now_real_ms(void){
    struct timespec ts; clock_gettime(CLOCK_REALTIME,&ts);
    return ts.tv_sec*1000.0 + ts.tv_nsec/1e6;
}
/* ★★★★★ w320 — THE THREAD-STATE CLOCK. This is the whole sync instrument.
 *
 * `cuCtxSynchronize` takes ~23.5 ms and did NOT move when the submit side got 21x faster
 * (w318). A wait that is constant regardless of what preceded it is not waiting on work —
 * but "not work" has at least three shapes and a wall clock cannot tell them apart:
 *
 *   spinning in userspace   ⇒ cpu ≈ wall, utime ≈ wall,  nvcsw ≈ 0
 *   spinning in the kernel  ⇒ cpu ≈ wall, stime ≈ wall,  nvcsw ≈ 0
 *   blocked / sleeping      ⇒ cpu ≈ 0,                    nvcsw > 0   ★ off-CPU
 *
 * CLOCK_THREAD_CPUTIME_ID is the same thread's CPU time, in the same process, at ns
 * resolution. ⇒ `offcpu = wall - cpu` is EXACT and there is no third term: the breakdown
 * CLOSES BY CONSTRUCTION. That is deliberate — w315's host-side breakdown needed a derived
 * clock offset and an argued containment licence, and neither survives here because during a
 * sync THE GUEST IS NOT HALTED. Staying inside one thread's own two clocks is what removes
 * the correspondence problem instead of arguing about it.
 *
 * ⊘ WHAT IT CANNOT DO, stated where it is used: it says THAT the thread waits and how often
 *   it blocked. It does not say WHAT woke it. The wakeup source is a separate arm.
 * ⚠ getrusage's utime/stime are TICK-GRANULAR on a stock kernel (1-4 ms), so the user/kernel
 *   SPLIT is coarse; the cpu-vs-wall total from CLOCK_THREAD_CPUTIME_ID is not, and that is
 *   the number the finding rests on. Both are printed so nobody has to guess which is which.
 */
static double cpu_ms(void){
    struct timespec ts;
    if(clock_gettime(CLOCK_THREAD_CPUTIME_ID,&ts)!=0) return -1.0;
    return ts.tv_sec*1000.0 + ts.tv_nsec/1e6;
}
typedef struct { double cpu; double utime; double stime; long nvcsw; long nivcsw; } tstate;
static void tstate_read(tstate *s){
    struct rusage ru;
    s->cpu = cpu_ms();
    if(getrusage(RUSAGE_THREAD,&ru)==0){
        s->utime = ru.ru_utime.tv_sec*1000.0 + ru.ru_utime.tv_usec/1000.0;
        s->stime = ru.ru_stime.tv_sec*1000.0 + ru.ru_stime.tv_usec/1000.0;
        s->nvcsw = ru.ru_nvcsw; s->nivcsw = ru.ru_nivcsw;
    } else { s->utime=s->stime=-1.0; s->nvcsw=s->nivcsw=-1; }
}

static int cmp_d(const void *a,const void *b){
    double x=*(const double*)a, y=*(const double*)b;
    return (x<y)?-1:((x>y)?1:0);
}
/* median / percentile of a SORTED array */
static double pct(const double *s,int n,double p){
    if(n<=0) return -1.0;
    int i=(int)(p*(n-1)+0.5); if(i<0)i=0; if(i>=n)i=n-1; return s[i];
}

int main(void){
    const char *e;
    const char *sizes_s = (e=getenv("BENCH_SIZES")) ? e : "1024,2048";
    int ITERS  = (e=getenv("BENCH_ITERS"))  ? atoi(e) : 20;
    int BATCH  = (e=getenv("BENCH_BATCH"))  ? atoi(e) : 10;
    int VERIFY = (e=getenv("BENCH_VERIFY")) ? atoi(e) : 1;
    /* ★★★★★ THE KNOWN-POSITIVE. `bad=0` is worth nothing unless the verifier can be SHOWN to
     * fire, and a census zero with no known-positive is a class this tree has paid for
     * repeatedly. BENCH_NOLAUNCH=1 skips every cuLaunchKernel and changes NOTHING else: the
     * poison fill, the readback and the verify all still run. ⇒ it MUST report bad = N*N.
     * If it reports 0, the verifier is dead and every green from this program is vacuous. */
    int NOLAUNCH = (e=getenv("BENCH_NOLAUNCH")) ? atoi(e) : 0;
    /* ★★★ w320 — THE BATCH SWEEP. K launches per ONE cuCtxSynchronize, over a LIST of K,
     * repeated, medianed. `mm` is IDEMPOTENT over the same buffers, so K launches produce the
     * same verified answer as one ⇒ this single arm is BOTH the batching question ("does
     * amortising the sync help?") AND the duration discriminator ("does the wait scale with
     * the work behind it?"). Fitting total(K) = a*K + b separates a per-LAUNCH term from a
     * per-SYNC term with a residual left over to check.
     * ⊘ DEFAULT EMPTY. With BENCH_BATCH_SWEEP unset this program is byte-for-byte w318's in
     *   behaviour, so w315/w318 arms remain one-variable against it. */
    const char *sweep_s = (e=getenv("BENCH_BATCH_SWEEP")) ? e : "";
    int SWEEP_REPS = (e=getenv("BENCH_BATCH_REPS")) ? atoi(e) : 5;
    /* ★★ w320 — cuCtxCreate flags. CU_CTX_SCHED_AUTO=0 (the default this tree has always
     * run), SPIN=1, YIELD=2, BLOCKING_SYNC=4. If the sync wait is a BLOCK, SPIN collapses it;
     * if it is below CUDA's scheduling choice, all three read the same. ⊘ A separate arm, not
     * a default: it changes the workload. */
    unsigned CTXFLAGS = (e=getenv("BENCH_CTX_FLAGS")) ? (unsigned)strtoul(e,NULL,0) : 0u;
    /* ★★★ w320 — operands in PINNED HOST MEMORY rather than VRAM. See the block at the
     * allocation site for what this controls for and what it cannot prove. ⊘ Default OFF. */
    int HOSTMEM = (e=getenv("BENCH_HOSTMEM")) ? atoi(e) : 0;
    if(ITERS<2)  ITERS=2;
    if(BATCH<1)  BATCH=1;
    if(VERIFY<1) VERIFY=1;
    if(SWEEP_REPS<1) SWEEP_REPS=1;

    printf("BENCH_BUILD sizes=[%s] iters=%d batch=%d verify_every=%d nolaunch=%d\n",
           sizes_s,ITERS,BATCH,VERIFY,NOLAUNCH); fflush(stdout);
    printf("BENCH_W320 batch_sweep=[%s] batch_reps=%d ctx_flags=0x%x hostmem=%d\n",
           sweep_s,SWEEP_REPS,CTXFLAGS,HOSTMEM); fflush(stdout);
    { /* ⊘ An instrument that silently degrades is worse than one that is absent: if this
       * kernel has no per-thread CPU clock, the whole §3.1 breakdown is meaningless and must
       * say so HERE rather than print zeros that read as "the thread never ran". */
      tstate probe; tstate_read(&probe);
      printf("BENCH_TSTATE_AVAILABLE=%s cpu_ms=%.6f nvcsw=%ld\n",
             (probe.cpu>=0.0 && probe.nvcsw>=0) ? "yes" : "NO", probe.cpu, probe.nvcsw);
      fflush(stdout);
    }
    /* w315: the clock origin, printed BEFORE any work, so every `t0_mono_ms` below is
     * placeable on a wall clock without anyone having to guess the process start. */
    printf("BENCH_CLOCK_ORIGIN mono_ms=%.3f real_ms=%.3f\n", now_ms(), now_real_ms());
    fflush(stdout);
    if(NOLAUNCH)
        printf("BENCH_MODE=NOLAUNCH — ★ NEGATIVE CONTROL. Launches are SKIPPED; the verifier"
               " MUST report bad = N*N. A 0 here means the verifier is dead.\n");
    else
        printf("BENCH_MODE=MEASURE\n");
    fflush(stdout);

    double t;
    t=now_ms(); CK(cuInit(0));                 printf("BENCH_INIT_MS=%.2f\n", now_ms()-t);
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){ printf("no dev\n"); return 1; }
    CUdevice d; CK(cuDeviceGet(&d,0));
    char devname[128]; memset(devname,0,sizeof devname);
    if(cuDeviceGetName(devname,sizeof devname-1,d)!=CUDA_SUCCESS) strcpy(devname,"<unavailable>");
    printf("BENCH_DEVICE=[%s]\n",devname);
    CUcontext ctx;
    t=now_ms(); CK(cuCtxCreate_v2(&ctx,CTXFLAGS,d)); printf("BENCH_CTX_MS=%.2f\n", now_ms()-t);
    CUmodule mod;
    t=now_ms(); CK(cuModuleLoadData(&mod,PTX)); printf("BENCH_MODULE_MS=%.2f\n", now_ms()-t);
    CUfunction fn; CK(cuModuleGetFunction(&fn,mod,"mm"));
    fflush(stdout);

    long total_bad = 0;
    int  n_sizes = 0;

    char *sl = strdup(sizes_s), *save=NULL;
    for(char *tok=strtok_r(sl,",",&save); tok; tok=strtok_r(NULL,",",&save)){
        unsigned N=(unsigned)atoi(tok);
        N=(N+15u)&~15u; if(!N) N=16;
        size_t sz=(size_t)N*N*sizeof(float);
        n_sizes++;

        printf("\nBENCH_SIZE_BEGIN N=%u per_matrix_MiB=%zu device_MiB=%zu\n",
               N,sz>>20,(3*sz)>>20); fflush(stdout);

        float *hA=malloc(sz), *hB=malloc(sz), *hC=malloc(sz);
        if(!hA||!hB||!hC){ printf("OOM host N=%u\n",N); return 1; }
        for(unsigned i=0;i<N;i++) for(unsigned k=0;k<N;k++) hA[(size_t)i*N+k]=(float)((i&3u)+1u);
        for(unsigned k=0;k<N;k++) for(unsigned j=0;j<N;j++) hB[(size_t)k*N+j]=(float)((j&3u)+1u);

        CUdeviceptr dA,dB,dC;
        void *pA=NULL,*pB=NULL,*pC=NULL;
        t=now_ms();
        if(HOSTMEM){
            /* ★★★★★ w320 — THE MECHANISM CONTROL, AND IT RUNS NATIVELY WITH NO GUEST.
             *
             * w320 measured the guest's kernel running 22-81x slower than the IDENTICAL kernel
             * native on the same GA106, with the submit path already down to ~4 ms. The leading
             * candidate is placement: this tree records that 99.4 % of our address table is
             * GUEST RAM, so the operands may be reached over PCIe rather than out of VRAM, and
             * `mm` is a naive triple loop that is entirely global-load-bound.
             *
             * ⊘ A MAGNITUDE THAT MATCHES IS NOT A MECHANISM — this campaign has paid for that
             *   three times. So the hypothesis is not argued from the ratio; it is REPRODUCED
             *   BY CONSTRUCTION: same GPU, same kernel, same program, no hypervisor anywhere,
             *   with the ONLY variable being that the operands live in pinned host memory.
             *   If that alone reproduces the slowdown, placement is sufficient to explain it.
             *   If it does not, the hypothesis is refuted and the cost is somewhere else.
             * ⚠ Sufficiency is not identity: reproducing the magnitude here would NOT prove our
             *   guest's buffers are placed this way, only that this placement costs this much.
             *   Where our buffers actually are is a separate measurement.
             *   CU_MEMHOSTALLOC_DEVICEMAP = 0x02. */
            CQ(cuMemHostAlloc(&pA,sz,0x02)); CQ(cuMemHostAlloc(&pB,sz,0x02));
            CQ(cuMemHostAlloc(&pC,sz,0x02));
            CQ(cuMemHostGetDevicePointer_v2(&dA,pA,0));
            CQ(cuMemHostGetDevicePointer_v2(&dB,pB,0));
            CQ(cuMemHostGetDevicePointer_v2(&dC,pC,0));
        } else {
            CQ(cuMemAlloc_v2(&dA,sz)); CQ(cuMemAlloc_v2(&dB,sz)); CQ(cuMemAlloc_v2(&dC,sz));
        }
        double alloc_ms = now_ms()-t;
        printf("B%u_ALLOC_MS=%.2f\n",N,alloc_ms);
        printf("B%u_PTRS=A:0x%llx,B:0x%llx,C:0x%llx\n",N,
               (unsigned long long)dA,(unsigned long long)dB,(unsigned long long)dC);
        fflush(stdout);

        t=now_ms();
        CQ(cuMemcpyHtoD_v2(dA,hA,sz)); CQ(cuMemcpyHtoD_v2(dB,hB,sz));
        CQ(cuCtxSynchronize());
        double h2d_ms = now_ms()-t;
        printf("B%u_H2D_MS=%.2f  (2 x %zu MiB)\n",N,h2d_ms,sz>>20); fflush(stdout);

        unsigned Np=N; void *args[]={ &dC,&dA,&dB,&Np };
        unsigned g=(N+15u)/16u;
        printf("B%u_GRID=(%u,%u)x(16,16)\n",N,g,g); fflush(stdout);

        double *lat = malloc(sizeof(double)*(size_t)ITERS);
        double *d2h = malloc(sizeof(double)*(size_t)ITERS);
        /* w315: the submit/complete split, and the absolute monotonic origin of each timed
         * window — the latter is what a host-side doorbell log is aligned against. */
        double *sub = malloc(sizeof(double)*(size_t)ITERS);
        double *syn = malloc(sizeof(double)*(size_t)ITERS);
        double *abs0= malloc(sizeof(double)*(size_t)ITERS);
        /* w320: the thread-state split, one array per reported quantity */
        double *sub_cpu=malloc(sizeof(double)*(size_t)ITERS);
        double *syn_cpu=malloc(sizeof(double)*(size_t)ITERS);
        double *syn_off=malloc(sizeof(double)*(size_t)ITERS);
        double *syn_csw=malloc(sizeof(double)*(size_t)ITERS);
        if(!lat||!d2h||!sub||!syn||!abs0||!sub_cpu||!syn_cpu||!syn_off||!syn_csw){
            printf("OOM stats\n"); return 1; }
        int n_d2h=0;
        long size_bad=0; float size_maxerr=0.f; long firstbad_idx=-1; float firstbad_got=0.f;

        for(int it=0; it<ITERS; it++){
            /* POISON, then a full sync, so the fill cannot land inside the timed window and
             * a launch that does nothing reads back as poison rather than as last iteration's
             * correct answer. ⊘ This is the falsifier; do not "optimise" it out of the loop. */
            CQ(cuMemsetD32_v2(dC,POISON,(size_t)N*N));
            CQ(cuCtxSynchronize());

            /* ★★★★★ w315 — THE ONE GUEST-SIDE SEGMENT BOUNDARY THAT EXISTS AT ALL.
             * `cuLaunchKernel` returns once the work is ENQUEUED; `cuCtxSynchronize`
             * returns once it is COMPLETE. Splitting there is the only place a userspace
             * program can cut the ~100 ms floor without a debugger, and it separates the two
             * mechanisms the floor could have: SUBMIT (our MMIO trap, which stops this vCPU
             * inline) vs COMPLETION (the guest spinning on a semaphore nobody has written).
             * ⊘ It does NOT attribute across the boundary — a submit_ms of 100 says the time
             * is inside the launch call, not that it is inside OUR trap. The host log's
             * per-doorbell durations are the other half, and the sum check is the finding. */
            /* ★★★★★ w320 — the thread-state clock is read at the SAME three boundaries as the
             * wall clock, so `submit` and `sync` each get an exact cpu/off-cpu split with no
             * extra bracketing and no fourth timestamp to go stale. */
            tstate s0,s1,s2; tstate_read(&s0);
            double t0=now_ms();
            if(!NOLAUNCH) CQ(cuLaunchKernel(fn, g,g,1, 16,16,1, 0,0, args,0));
            double tsub=now_ms();
            tstate_read(&s1);
            CQ(cuCtxSynchronize());
            double t1=now_ms();
            tstate_read(&s2);
            lat[it]=t1-t0;
            sub[it]=tsub-t0;
            syn[it]=t1-tsub;
            abs0[it]=t0;
            sub_cpu[it]=s1.cpu-s0.cpu;
            syn_cpu[it]=s2.cpu-s1.cpu;
            syn_off[it]=syn[it]-syn_cpu[it];
            syn_csw[it]=(double)(s2.nvcsw-s1.nvcsw);

            long bad=0; float maxerr=0.f;
            if(it % VERIFY == 0){
                double t2=now_ms();
                CQ(cuMemcpyDtoH_v2(hC,dC,sz));
                d2h[n_d2h++]=now_ms()-t2;
                /* ⚠ NO early-exit guard: `bad` is a WHOLE-MATRIX total and `maxerr` a
                 *   whole-matrix maximum. cup8.c's `bad<8` guard made both partial. */
                for(unsigned i=0;i<N;i++) for(unsigned j=0;j<N;j++){
                    float ref=(float)N*(float)((i&3u)+1u)*(float)((j&3u)+1u);
                    float got=hC[(size_t)i*N+j];
                    float er=fabsf(got-ref); if(er>maxerr)maxerr=er;
                    if(er>1e-3f){ bad++;
                        if(firstbad_idx<0){ firstbad_idx=(long)((size_t)i*N+j); firstbad_got=got; } }
                }
                size_bad += bad; if(maxerr>size_maxerr) size_maxerr=maxerr;
            }
            /* ⚠ w320 fields are APPENDED AFTER `t0_mono_ms=`, never inserted before it.
             * w315_align.py / w315_attribute.py both match
             *   `... sync_ms=(...) t0_mono_ms=(...)`
             * as ADJACENT groups; inserting a field between them would silently stop those
             * two analysers matching any line — and a regex that matches nothing reports an
             * empty result, not an error. Appending keeps every existing reader green. */
            printf("ITER N=%u i=%d launch_ms=%.3f submit_ms=%.3f sync_ms=%.3f "
                   "t0_mono_ms=%.3f sub_cpu_ms=%.3f syn_cpu_ms=%.3f syn_offcpu_ms=%.3f "
                   "syn_nvcsw=%ld syn_nivcsw=%ld syn_utime_ms=%.3f syn_stime_ms=%.3f "
                   "verified=%s bad=%ld\n",
                   N,it,lat[it],sub[it],syn[it],abs0[it],
                   sub_cpu[it],syn_cpu[it],syn_off[it],
                   s2.nvcsw-s1.nvcsw, s2.nivcsw-s1.nivcsw,
                   s2.utime-s1.utime, s2.stime-s1.stime,
                   (it%VERIFY==0)?"yes":"no", bad);
            fflush(stdout);
        }

        /* ---- BATCH PHASE: K launches, ONE sync. Separates submit from completion. ---- */
        double batch_ms=-1.0; long batch_bad=-1;
        {
            CQ(cuMemsetD32_v2(dC,POISON,(size_t)N*N));
            CQ(cuCtxSynchronize());
            double t0=now_ms();
            for(int b=0;b<BATCH && !NOLAUNCH;b++)
                CQ(cuLaunchKernel(fn, g,g,1, 16,16,1, 0,0, args,0));
            CQ(cuCtxSynchronize());
            batch_ms=now_ms()-t0;
            CQ(cuMemcpyDtoH_v2(hC,dC,sz));
            long bb=0;
            for(unsigned i=0;i<N;i++) for(unsigned j=0;j<N;j++){
                float ref=(float)N*(float)((i&3u)+1u)*(float)((j&3u)+1u);
                if(fabsf(hC[(size_t)i*N+j]-ref)>1e-3f) bb++;
            }
            batch_bad=bb; size_bad+=bb;
        }

        /* ================================================================================
         * ★★★★★ w320 — THE BATCH SWEEP. K launches, ONE sync, over a LIST of K, repeated.
         *
         * WHY THIS ONE ARM ANSWERS TWO QUESTIONS. `mm` writes C = A*B and reads neither its
         * own output nor any state carried between launches ⇒ it is IDEMPOTENT over these
         * buffers. K launches therefore leave EXACTLY the answer one launch leaves, while
         * putting K times as much work behind the single `cuCtxSynchronize`. So:
         *
         *   fit  total(K) = a*K + b
         *        a  ->  the per-LAUNCH term (submit, plus any real per-kernel completion)
         *        b  ->  the per-SYNC term   (a fixed wait, if there is one)
         *
         * If b is large and a is ~the submit floor, the wait is per-sync and BATCHING
         * AMORTISES IT. If b ~ 0 and a ~ submit+sync, the wait is real per-kernel work and
         * batching moves nothing. ⊘ Neither outcome is assumed; both are printed.
         *
         * ⚠ CORRECTNESS IS NOT RELAXED HERE, and that is deliberate: a batched loop that
         *   silently skips work is the easiest possible false green. C is POISONED before
         *   every rep and VERIFIED after every rep — not once at the end — so a K that
         *   launched nothing reads back as poison and is counted. Under BENCH_NOLAUNCH the
         *   inner loop is skipped and every rep MUST report bad>0; that inversion is the
         *   known-positive and it is asserted per rep rather than per program.
         *
         * ⚠ ORDERING: the sweep runs LAST, after the per-iteration phase and the legacy
         *   BATCH phase, so a deadline hit costs the NEW measurement and never the ones
         *   w315/w318 are read against.
         * ================================================================================ */
        if(sweep_s[0]){
            printf("\nBSWEEP_BEGIN N=%u reps=%d list=[%s]\n",N,SWEEP_REPS,sweep_s);
            fflush(stdout);
            char *kl=strdup(sweep_s), *ksave=NULL;
            double *rt=malloc(sizeof(double)*(size_t)SWEEP_REPS);
            double *rc=malloc(sizeof(double)*(size_t)SWEEP_REPS);
            double *ro=malloc(sizeof(double)*(size_t)SWEEP_REPS);
            double *rw=malloc(sizeof(double)*(size_t)SWEEP_REPS);
            if(!kl||!rt||!rc||!ro||!rw){ printf("OOM sweep\n"); return 1; }
            for(char *kt=strtok_r(kl,",",&ksave); kt; kt=strtok_r(NULL,",",&ksave)){
                int K=atoi(kt); if(K<1) K=1;
                long kbad_total=0;
                for(int r=0;r<SWEEP_REPS;r++){
                    CQ(cuMemsetD32_v2(dC,POISON,(size_t)N*N));
                    CQ(cuCtxSynchronize());
                    tstate q0,q1; tstate_read(&q0);
                    double b0=now_ms();
                    for(int b=0;b<K && !NOLAUNCH;b++)
                        CQ(cuLaunchKernel(fn, g,g,1, 16,16,1, 0,0, args,0));
                    double bs=now_ms();
                    CQ(cuCtxSynchronize());
                    double b1=now_ms();
                    tstate_read(&q1);
                    rt[r]=b1-b0;
                    rc[r]=q1.cpu-q0.cpu;
                    ro[r]=rt[r]-rc[r];
                    rw[r]=(double)(q1.nvcsw-q0.nvcsw);
                    /* ★ VERIFY EVERY REP. The whole matrix, no early exit. */
                    CQ(cuMemcpyDtoH_v2(hC,dC,sz));
                    long kb=0;
                    for(unsigned i=0;i<N;i++) for(unsigned j=0;j<N;j++){
                        float ref=(float)N*(float)((i&3u)+1u)*(float)((j&3u)+1u);
                        if(fabsf(hC[(size_t)i*N+j]-ref)>1e-3f) kb++;
                    }
                    kbad_total+=kb;
                    printf("BSWEEP N=%u K=%d rep=%d total_ms=%.3f submit_ms=%.3f sync_ms=%.3f "
                           "cpu_ms=%.3f offcpu_ms=%.3f nvcsw=%ld bad=%ld\n",
                           N,K,r,rt[r],bs-b0,b1-bs,rc[r],ro[r],q1.nvcsw-q0.nvcsw,kb);
                    fflush(stdout);
                }
                size_bad += kbad_total;
                double *st=malloc(sizeof(double)*(size_t)SWEEP_REPS);
                double *sc=malloc(sizeof(double)*(size_t)SWEEP_REPS);
                double *so=malloc(sizeof(double)*(size_t)SWEEP_REPS);
                double *sw=malloc(sizeof(double)*(size_t)SWEEP_REPS);
                if(!st||!sc||!so||!sw){ printf("OOM sweep2\n"); return 1; }
                memcpy(st,rt,sizeof(double)*(size_t)SWEEP_REPS);
                memcpy(sc,rc,sizeof(double)*(size_t)SWEEP_REPS);
                memcpy(so,ro,sizeof(double)*(size_t)SWEEP_REPS);
                memcpy(sw,rw,sizeof(double)*(size_t)SWEEP_REPS);
                qsort(st,(size_t)SWEEP_REPS,sizeof(double),cmp_d);
                qsort(sc,(size_t)SWEEP_REPS,sizeof(double),cmp_d);
                qsort(so,(size_t)SWEEP_REPS,sizeof(double),cmp_d);
                qsort(sw,(size_t)SWEEP_REPS,sizeof(double),cmp_d);
                /* ⊘ min and max are printed beside the median because a per-K median over 5
                 * reps on a shared bench is an order statistic, not a measurement of one
                 * thing; a reader has to be able to see the spread the fit is drawn through. */
                printf("BSWEEPSUM N=%u K=%d reps=%d med_total_ms=%.3f per_launch_ms=%.3f "
                       "min_ms=%.3f max_ms=%.3f med_cpu_ms=%.3f med_offcpu_ms=%.3f "
                       "med_nvcsw=%.1f bad=%ld\n",
                       N,K,SWEEP_REPS,pct(st,SWEEP_REPS,0.5),pct(st,SWEEP_REPS,0.5)/K,
                       st[0],st[SWEEP_REPS-1],pct(sc,SWEEP_REPS,0.5),pct(so,SWEEP_REPS,0.5),
                       pct(sw,SWEEP_REPS,0.5),kbad_total);
                fflush(stdout);
                free(st);free(sc);free(so);free(sw);
            }
            free(kl);free(rt);free(rc);free(ro);free(rw);
            printf("BSWEEP_END N=%u\n",N); fflush(stdout);
        }

        /* ---- STATS. First launch reported SEPARATELY from steady state. ---- */
        double first_ms = lat[0];
        int    ns = ITERS-1;
        double *srt = malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        for(int i=0;i<ns;i++) srt[i]=lat[i+1];
        qsort(srt,(size_t)ns,sizeof(double),cmp_d);
        double med=pct(srt,ns,0.5), p90=pct(srt,ns,0.90);
        /* w315: the same first-launch-discarded statistic, on each half of the split. ⚠ The
         * two medians are taken INDEPENDENTLY, so submit_med + sync_med need not equal
         * med — they are three order statistics of three samples, not a decomposition of
         * one number. The per-ITER lines carry the paired values for anyone who needs the
         * decomposition to hold exactly. */
        double *ssub = malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        double *ssyn = malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        for(int i=0;i<ns;i++){ ssub[i]=sub[i+1]; ssyn[i]=syn[i+1]; }
        qsort(ssub,(size_t)ns,sizeof(double),cmp_d);
        qsort(ssyn,(size_t)ns,sizeof(double),cmp_d);
        double sub_med=pct(ssub,ns,0.5), syn_med=pct(ssyn,ns,0.5);
        /* ★★★ w320 — the SAME first-launch-discarded median, on the thread-state split.
         * ⚠ These are INDEPENDENT order statistics exactly as sub_med/syn_med are, so
         *   `syn_cpu_med + syn_offcpu_med` need not equal `syn_med`. The per-ITER lines carry
         *   the paired values, and the identity `offcpu = wall - cpu` holds THERE, per
         *   iteration, exactly. A reader wanting the identity must use the ITER lines. */
        double *scpu=malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        double *soff=malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        double *scsw=malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        double *sbcp=malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        if(!scpu||!soff||!scsw||!sbcp){ printf("OOM stats2\n"); return 1; }
        for(int i=0;i<ns;i++){ scpu[i]=syn_cpu[i+1]; soff[i]=syn_off[i+1];
                               scsw[i]=syn_csw[i+1]; sbcp[i]=sub_cpu[i+1]; }
        qsort(scpu,(size_t)ns,sizeof(double),cmp_d);
        qsort(soff,(size_t)ns,sizeof(double),cmp_d);
        qsort(scsw,(size_t)ns,sizeof(double),cmp_d);
        qsort(sbcp,(size_t)ns,sizeof(double),cmp_d);
        double syn_cpu_med=pct(scpu,ns,0.5), syn_off_med=pct(soff,ns,0.5);
        double syn_csw_med=pct(scsw,ns,0.5), sub_cpu_med=pct(sbcp,ns,0.5);
        double sub_max=ns>0?ssub[ns-1]:-1.0, syn_max=ns>0?ssyn[ns-1]:-1.0;
        double mn = ns>0?srt[0]:-1.0, mx = ns>0?srt[ns-1]:-1.0;
        double sum=0; for(int i=0;i<ns;i++) sum+=srt[i];
        double mean = ns>0? sum/ns : -1.0;

        double *ds = malloc(sizeof(double)*(size_t)(n_d2h>0?n_d2h:1));
        for(int i=0;i<n_d2h;i++) ds[i]=d2h[i];
        qsort(ds,(size_t)n_d2h,sizeof(double),cmp_d);
        double d2h_med = pct(ds,n_d2h,0.5);

        double flop = 2.0*(double)N*(double)N*(double)N;
        double gflops_med   = med>0     ? flop/(med/1000.0)/1e9 : -1.0;
        double gflops_batch = batch_ms>0? (flop*BATCH)/(batch_ms/1000.0)/1e9 : -1.0;

        printf("\n");
        printf("B%u_N=%u\n",N,N);
        printf("B%u_ITERS=%d\n",N,ITERS);
        printf("B%u_FIRST_MS=%.3f\n",N,first_ms);
        printf("B%u_MEDIAN_MS=%.3f\n",N,med);
        printf("B%u_P90_MS=%.3f\n",N,p90);
        printf("B%u_MIN_MS=%.3f\n",N,mn);
        printf("B%u_MAX_MS=%.3f\n",N,mx);
        printf("B%u_MEAN_MS=%.3f\n",N,mean);
        printf("B%u_GFLOPS=%.3f\n",N,gflops_med);
        printf("B%u_BATCH=%d\n",N,BATCH);
        printf("B%u_BATCH_TOTAL_MS=%.3f\n",N,batch_ms);
        printf("B%u_BATCH_PER_LAUNCH_MS=%.3f\n",N,BATCH>0?batch_ms/BATCH:-1.0);
        printf("B%u_BATCH_GFLOPS=%.3f\n",N,gflops_batch);
        printf("B%u_BATCH_BAD=%ld\n",N,batch_bad);
        printf("B%u_H2D_MS=%.3f\n",N,h2d_ms);
        printf("B%u_D2H_MED_MS=%.3f\n",N,d2h_med);
        printf("B%u_ALLOC_MS2=%.3f\n",N,alloc_ms);
        printf("B%u_SUBMIT_MED_MS=%.3f\n",N,sub_med);
        printf("B%u_SYNC_MED_MS=%.3f\n",N,syn_med);
        printf("B%u_SYNC_CPU_MED_MS=%.3f\n",N,syn_cpu_med);
        printf("B%u_SYNC_OFFCPU_MED_MS=%.3f\n",N,syn_off_med);
        printf("B%u_SYNC_NVCSW_MED=%.1f\n",N,syn_csw_med);
        printf("B%u_SUBMIT_CPU_MED_MS=%.3f\n",N,sub_cpu_med);
        printf("B%u_SUBMIT_MAX_MS=%.3f\n",N,sub_max);
        printf("B%u_SYNC_MAX_MS=%.3f\n",N,syn_max);
        printf("B%u_WINDOW_MONO_MS=%.3f..%.3f\n",N,abs0[0],abs0[ITERS-1]+lat[ITERS-1]);
        printf("B%u_BAD=%ld\n",N,size_bad);
        printf("B%u_MAXERR=%g\n",N,size_maxerr);
        if(firstbad_idx>=0) printf("B%u_FIRSTBAD=idx%ld got=%g\n",N,firstbad_idx,firstbad_got);
        printf("BSUM N=%u iters=%d first_ms=%.3f med_ms=%.3f p90_ms=%.3f min_ms=%.3f "
               "max_ms=%.3f gflops=%.3f batch_per_launch_ms=%.3f batch_gflops=%.3f "
               "h2d_ms=%.3f d2h_med_ms=%.3f submit_med_ms=%.3f sync_med_ms=%.3f "
               "sync_cpu_med_ms=%.3f sync_offcpu_med_ms=%.3f sync_nvcsw_med=%.1f "
               "submit_cpu_med_ms=%.3f bad=%ld maxerr=%g\n",
               N,ITERS,first_ms,med,p90,mn,mx,gflops_med,
               BATCH>0?batch_ms/BATCH:-1.0,gflops_batch,h2d_ms,d2h_med,
               sub_med,syn_med,syn_cpu_med,syn_off_med,syn_csw_med,sub_cpu_med,
               size_bad,size_maxerr);
        printf("BENCH_SIZE_END N=%u\n",N); fflush(stdout);

        total_bad += size_bad;
        free(srt); free(ds); free(lat); free(d2h);
        free(ssub); free(ssyn); free(sub); free(syn); free(abs0);
        free(scpu); free(soff); free(scsw); free(sbcp);
        free(sub_cpu); free(syn_cpu); free(syn_off); free(syn_csw);
        if(HOSTMEM){ cuMemFreeHost(pA); cuMemFreeHost(pB); cuMemFreeHost(pC); }
        else       { cuMemFree_v2(dA); cuMemFree_v2(dB); cuMemFree_v2(dC); }
        free(hA); free(hB); free(hC);
    }
    free(sl);

    printf("BENCH_CLOCK_END mono_ms=%.3f real_ms=%.3f\n", now_ms(), now_real_ms());
    printf("\nBENCH_SIZES_DONE=%d\n",n_sizes);
    printf("BENCH_TOTAL_BAD=%ld\n",total_bad);
    if(NOLAUNCH){
        /* ★ INVERTED GRADING, deliberately: in the negative control a ZERO is the failure. */
        printf("BENCH_NOLAUNCH_TOTAL_BAD=%ld\n",total_bad);
        printf("BENCH_VERDICT: %s\n", total_bad>0
               ? "PASS-NEGATIVE-CONTROL (the verifier FIRED with launches skipped)"
               : "FAIL-NEGATIVE-CONTROL (the verifier reported 0 with NO LAUNCHES — it is dead)");
        printf("DONE\n"); fflush(stdout);
        return total_bad>0?0:3;
    }
    printf("BENCH_VERDICT: %s\n", total_bad==0 ? "PASS (every timed iteration verified)"
                                               : "FAIL (a timed iteration produced wrong data)");
    printf("DONE\n"); fflush(stdout);
    return total_bad==0?0:2;
}
