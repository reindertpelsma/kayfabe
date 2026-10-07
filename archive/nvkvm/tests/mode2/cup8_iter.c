/* cup8_iter.c — scenario 2: multiple matmul kernels in ONE process/context.
 * Runs ITERS matmuls (varying N each iter) reusing the same context+module, verifying
 * byte-exact each time. Stresses repeated launch/sync/copy + repeated cuMemAlloc/free
 * within a single CUDA context (multiple kernels, one process). rc=0 on all-pass. */
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} }while(0)

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

static int run_one(CUfunction fn, unsigned N, int iter){
    N=(N+15u)&~15u; if(!N)N=16;
    size_t sz=(size_t)N*N*sizeof(float);
    float *hA=malloc(sz),*hB=malloc(sz),*hC=malloc(sz);
    if(!hA||!hB||!hC){printf("OOM host iter %d\n",iter);return 1;}
    for(unsigned i=0;i<N;i++)for(unsigned k=0;k<N;k++)hA[(size_t)i*N+k]=(float)((i&3u)+1u);
    for(unsigned k=0;k<N;k++)for(unsigned j=0;j<N;j++)hB[(size_t)k*N+j]=(float)((j&3u)+1u);
    CUdeviceptr dA,dB,dC;
    CK(cuMemAlloc(&dA,sz)); CK(cuMemAlloc(&dB,sz)); CK(cuMemAlloc(&dC,sz));
    CK(cuMemcpyHtoD(dA,hA,sz)); CK(cuMemcpyHtoD(dB,hB,sz)); CK(cuMemsetD32(dC,0,(size_t)N*N));
    unsigned Np=N; void*args[]={&dC,&dA,&dB,&Np}; unsigned g=(N+15u)/16u;
    CK(cuLaunchKernel(fn,g,g,1,16,16,1,0,0,args,0));
    CK(cuCtxSynchronize());
    CK(cuMemcpyDtoH(hC,dC,sz));
    long bad=0; float maxerr=0;
    for(unsigned i=0;i<N&&bad<8;i++)for(unsigned j=0;j<N;j++){
        float ref=(float)N*(float)((i&3u)+1u)*(float)((j&3u)+1u);
        float er=fabsf(hC[(size_t)i*N+j]-ref); if(er>maxerr)maxerr=er;
        if(er>1e-3f)bad++;
    }
    printf("ITER %d N=%u bad=%ld maxerr=%g -> %s\n",iter,N,bad,maxerr,bad==0?"PASS":"MISMATCH");
    fflush(stdout);
    cuMemFree(dA); cuMemFree(dB); cuMemFree(dC);
    free(hA);free(hB);free(hC);
    return bad==0?0:2;
}

int main(void){
    int ITERS=5; const char*e=getenv("ITERS"); if(e)ITERS=atoi(e);
    unsigned sizes[]={512,1024,1536,2048,768,1280,2048,512};
    CK(cuInit(0));
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){printf("no dev\n");return 1;}
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d)); printf("CTX OK\n"); fflush(stdout);
    CUmodule mod; CK(cuModuleLoadData(&mod,PTX)); printf("MODULE OK\n"); fflush(stdout);
    CUfunction fn; CK(cuModuleGetFunction(&fn,mod,"mm")); printf("FUNC OK\n"); fflush(stdout);
    int fails=0;
    for(int it=0; it<ITERS; it++){
        unsigned N=sizes[it % (int)(sizeof(sizes)/sizeof(sizes[0]))];
        if(run_one(fn,N,it)!=0){ fails++; printf("ITER %d FAILED\n",it); break; }
    }
    printf("CUP8_ITER VERDICT: %s (iters=%d fails=%d)\n", fails==0?"PASS":"FAIL", ITERS, fails);
    printf("DONE\n"); fflush(stdout);
    return fails==0?0:2;
}
