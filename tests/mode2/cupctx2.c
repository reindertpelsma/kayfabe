/* Mode-2 #12 repro (2026-06-17): a 2nd CUDA context after the 1st tears down.
 * Same NxN fp32 grid-matmul as cup8.c, but factored into run_ctx() which creates a
 * FRESH context, does the work, verifies, and DESTROYS the context — called TWICE.
 *
 * #12 symptom: ctx1 runs+verifies+destroys cleanly; ctx1's teardown fires the
 * CeUtils CE-scrubber destruct (scrubberDestruct waits on a vidmem finishPayload
 * sema we never write) → wedges shared driver state → ctx2 hangs at its FIRST
 * cuCtxSynchronize. Per-op markers (flushed) pinpoint exactly where ctx2 stalls;
 * NEVER SIGKILL mid-CUDA-op (wedges the driver) — let it time out.
 *
 * Small N (default 256) keeps each context fast; #12 is about context lifecycle,
 * not scale. CUPCTX2_N overrides; CUPCTX2_ITERS (default 2) sets context count. */
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return -1;} }while(0)
#define MK(...) do{ printf(__VA_ARGS__); fflush(stdout); }while(0)

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
"$L_ret:\n  ret;\n}\n";

/* One full context lifecycle: create -> matmul -> verify -> destroy. Returns
 * bad-count (>=0) or -1 on CUDA error. `tag` labels the markers (CTX1/CTX2). */
static long run_ctx(CUdevice d, unsigned N, const char *tag)
{
    MK("[%s] cuCtxCreate...\n", tag);
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d));
    MK("[%s] CTX OK\n", tag);
    CUmodule mod; CK(cuModuleLoadData(&mod,PTX));
    CUfunction fn; CK(cuModuleGetFunction(&fn,mod,"mm"));
    MK("[%s] MODULE OK\n", tag);

    size_t sz=(size_t)N*N*sizeof(float);
    float *hA=malloc(sz),*hB=malloc(sz),*hC=malloc(sz);
    if(!hA||!hB||!hC){ MK("[%s] OOM host\n",tag); return -1; }
    for(unsigned i=0;i<N;i++) for(unsigned k=0;k<N;k++) hA[(size_t)i*N+k]=(float)((i&3u)+1u);
    for(unsigned k=0;k<N;k++) for(unsigned j=0;j<N;j++) hB[(size_t)k*N+j]=(float)((j&3u)+1u);

    CUdeviceptr dA,dB,dC;
    CK(cuMemAlloc(&dA,sz)); CK(cuMemAlloc(&dB,sz)); CK(cuMemAlloc(&dC,sz));
    CK(cuMemcpyHtoD(dA,hA,sz)); CK(cuMemcpyHtoD(dB,hB,sz)); CK(cuMemsetD32(dC,0,(size_t)N*N));
    MK("[%s] H2D OK\n", tag);

    unsigned Np=N; void *args[]={ &dC,&dA,&dB,&Np }; unsigned g=(N+15u)/16u;
    CK(cuLaunchKernel(fn, g,g,1, 16,16,1, 0,0, args,0));
    MK("[%s] LAUNCH OK -> cuCtxSynchronize (THE #12 hang point for CTX2)...\n", tag);
    CK(cuCtxSynchronize());
    MK("[%s] SYNC OK\n", tag);
    CK(cuMemcpyDtoH(hC,dC,sz));

    long bad=0; float maxerr=0;
    for(unsigned i=0;i<N && bad<8;i++) for(unsigned j=0;j<N;j++){
        float ref=(float)N*(float)((i&3u)+1u)*(float)((j&3u)+1u);
        float er=fabsf(hC[(size_t)i*N+j]-ref); if(er>maxerr)maxerr=er;
        if(er>1e-3f) bad++;
    }
    MK("[%s] RESULT bad=%ld maxerr=%g C[0]=%g -> %s\n", tag,bad,maxerr,hC[0],
       bad==0?"PASS":"MISMATCH");
    CK(cuMemFree(dA)); CK(cuMemFree(dB)); CK(cuMemFree(dC));
    free(hA); free(hB); free(hC);
    MK("[%s] cuCtxDestroy (fires CeUtils scrubberDestruct)...\n", tag);
    CK(cuCtxDestroy(ctx));
    MK("[%s] CTX DESTROY OK\n", tag);
    return bad;
}

int main(void){
    unsigned N=256; const char *e=getenv("CUPCTX2_N"); if(e){ N=(unsigned)atoi(e); }
    N=(N+15u)&~15u; if(!N) N=16;
    int iters=2; const char *it=getenv("CUPCTX2_ITERS"); if(it){ iters=atoi(it); if(iters<1)iters=1; }
    CK(cuInit(0));
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){MK("no dev\n");return 1;}
    CUdevice d; CK(cuDeviceGet(&d,0));
    MK("CUPCTX2 N=%u iters=%d (per-matrix=%zu KiB)\n", N, iters, ((size_t)N*N*4)>>10);

    long total_bad=0;
    for(int i=0;i<iters;i++){
        char tag[16]; snprintf(tag,sizeof tag,"CTX%d",i+1);
        long b=run_ctx(d,N,tag);
        if(b<0){ MK("CUPCTX2 VERDICT: FAIL (CUDA error in %s)\n",tag); return 3; }
        total_bad+=b;
    }
    MK("CUPCTX2 VERDICT: %s (%d contexts, total bad=%ld)\n",
       total_bad==0?"PASS":"MISMATCH", iters, total_bad);
    MK("DONE\n");
    return total_bad==0?0:2;
}
