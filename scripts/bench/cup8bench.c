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
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <time.h>

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
    if(ITERS<2)  ITERS=2;
    if(BATCH<1)  BATCH=1;
    if(VERIFY<1) VERIFY=1;

    printf("BENCH_BUILD sizes=[%s] iters=%d batch=%d verify_every=%d nolaunch=%d\n",
           sizes_s,ITERS,BATCH,VERIFY,NOLAUNCH); fflush(stdout);
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
    t=now_ms(); CK(cuCtxCreate_v2(&ctx,0,d)); printf("BENCH_CTX_MS=%.2f\n", now_ms()-t);
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
        t=now_ms();
        CQ(cuMemAlloc_v2(&dA,sz)); CQ(cuMemAlloc_v2(&dB,sz)); CQ(cuMemAlloc_v2(&dC,sz));
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
        if(!lat||!d2h){ printf("OOM stats\n"); return 1; }
        int n_d2h=0;
        long size_bad=0; float size_maxerr=0.f; long firstbad_idx=-1; float firstbad_got=0.f;

        for(int it=0; it<ITERS; it++){
            /* POISON, then a full sync, so the fill cannot land inside the timed window and
             * a launch that does nothing reads back as poison rather than as last iteration's
             * correct answer. ⊘ This is the falsifier; do not "optimise" it out of the loop. */
            CQ(cuMemsetD32_v2(dC,POISON,(size_t)N*N));
            CQ(cuCtxSynchronize());

            double t0=now_ms();
            if(!NOLAUNCH) CQ(cuLaunchKernel(fn, g,g,1, 16,16,1, 0,0, args,0));
            CQ(cuCtxSynchronize());
            double t1=now_ms();
            lat[it]=t1-t0;

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
            printf("ITER N=%u i=%d launch_ms=%.3f verified=%s bad=%ld\n",
                   N,it,lat[it], (it%VERIFY==0)?"yes":"no", bad);
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

        /* ---- STATS. First launch reported SEPARATELY from steady state. ---- */
        double first_ms = lat[0];
        int    ns = ITERS-1;
        double *srt = malloc(sizeof(double)*(size_t)(ns>0?ns:1));
        for(int i=0;i<ns;i++) srt[i]=lat[i+1];
        qsort(srt,(size_t)ns,sizeof(double),cmp_d);
        double med=pct(srt,ns,0.5), p90=pct(srt,ns,0.90);
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
        printf("B%u_BAD=%ld\n",N,size_bad);
        printf("B%u_MAXERR=%g\n",N,size_maxerr);
        if(firstbad_idx>=0) printf("B%u_FIRSTBAD=idx%ld got=%g\n",N,firstbad_idx,firstbad_got);
        printf("BSUM N=%u iters=%d first_ms=%.3f med_ms=%.3f p90_ms=%.3f min_ms=%.3f "
               "max_ms=%.3f gflops=%.3f batch_per_launch_ms=%.3f batch_gflops=%.3f "
               "h2d_ms=%.3f d2h_med_ms=%.3f bad=%ld maxerr=%g\n",
               N,ITERS,first_ms,med,p90,mn,mx,gflops_med,
               BATCH>0?batch_ms/BATCH:-1.0,gflops_batch,h2d_ms,d2h_med,size_bad,size_maxerr);
        printf("BENCH_SIZE_END N=%u\n",N); fflush(stdout);

        total_bad += size_bad;
        free(srt); free(ds); free(lat); free(d2h);
        cuMemFree_v2(dA); cuMemFree_v2(dB); cuMemFree_v2(dC);
        free(hA); free(hB); free(hC);
    }
    free(sl);

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
