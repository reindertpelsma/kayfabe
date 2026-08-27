/* Mode-2 D1/D2 discriminator: does the host GR engine READ a LARGE user cuMemAlloc buffer
 * correctly? cup4 (N=16, 1 KB) is walk-caught and proves nothing at scale; cup5/cup6 are
 * HtoD->DtoH *copy* round-trips the emulated CE/CPU path can satisfy without the host GPU ever
 * seeing the bytes. This test forces a GR (shader) READ of a multi-MB buffer:
 *   out[i] = in[i] + 1.0f   over an N-float buffer (default 64 MB), 1-D grid.
 * `in` is filled HtoD with a non-zero pattern; `out` is pre-set to a sentinel. After launch we
 * byte-verify and classify the FIRST wrong element into a tri-state that names the failure mode:
 *   out[i] == in[i]+1   -> CORRECT (host GR read real bytes)
 *   out[i] == SENTINEL  -> kernel never wrote it (launch/coverage gap, not a backing bug)
 *   out[i] == 1.0f      -> in[i] read as 0  => the `in` buffer leaf is UNBACKED/fake (D2!)
 *   else                -> in[i] read as some other garbage (D2, wrong backing)
 * The bad index * 4 + dIn gives the GPU VA of the offending leaf to grep in the QEMU log
 * (M7 R2 gpga_obj / M5.51 gpga_obj FAILED). PASS (rc=0) => D1 (correctness holds at scale).
 *
 * Env: CUP7_MB = buffer size in MiB (default 64). Build: gcc -O0 -o cup7 cup7.c -lcuda. */
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)

/* va(out,in,N): i = ctaid.x*ntid.x + tid.x; if(i<N) out[i] = in[i] + 1.0f; */
static const char *PTX =
".version 7.8\n.target sm_86\n.address_size 64\n"
".visible .entry va(.param .u64 pOut,.param .u64 pIn,.param .u32 pN){\n"
"  .reg .pred %p<2>; .reg .f32 %f<3>; .reg .b32 %r<6>; .reg .b64 %rd<7>;\n"
"  ld.param.u64 %rd1,[pOut]; ld.param.u64 %rd2,[pIn]; ld.param.u32 %r1,[pN];\n"
"  mov.u32 %r2,%ctaid.x; mov.u32 %r3,%ntid.x; mov.u32 %r4,%tid.x;\n"
"  mad.lo.s32 %r5,%r2,%r3,%r4;            // i = ctaid.x*ntid.x + tid.x\n"
"  setp.ge.u32 %p1,%r5,%r1; @%p1 bra $L_done;\n"
"  cvta.to.global.u64 %rd1,%rd1; cvta.to.global.u64 %rd2,%rd2;\n"
"  mul.wide.u32 %rd3,%r5,4;\n"
"  add.s64 %rd4,%rd2,%rd3; ld.global.f32 %f1,[%rd4];   // in[i]\n"
"  add.f32 %f2,%f1,0f3F800000;                          // + 1.0f\n"
"  add.s64 %rd5,%rd1,%rd3; st.global.f32 [%rd5],%f2;   // out[i]\n"
"$L_done:\n  ret;\n}\n";

/* deterministic, never 0.0f (so a zero read-back = unbacked, not a valid value) */
static float patt(unsigned i){ return (float)((i * 2654435761u) & 0xfffff) + 1.0f; }

int main(void){
    unsigned mb = 64; const char *e = getenv("CUP7_MB"); if(e&&*e) mb=(unsigned)atoi(e);
    size_t bytes = (size_t)mb << 20; unsigned n = (unsigned)(bytes/4); bytes = (size_t)n*4;
    const float SENT = -7777.0f;
    printf("CUP7 buffer=%u MiB  n=%u floats\n", mb, n); fflush(stdout);

    CK(cuInit(0));
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){printf("no dev\n");return 1;}
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d)); printf("CTX OK\n"); fflush(stdout);
    CUmodule mod; CK(cuModuleLoadData(&mod,PTX)); printf("MODULE OK\n"); fflush(stdout);
    CUfunction fn; CK(cuModuleGetFunction(&fn,mod,"va")); printf("FUNC OK\n"); fflush(stdout);

    float *hIn=malloc(bytes), *hOut=malloc(bytes);
    if(!hIn||!hOut){printf("OOM host\n");return 1;}
    for(unsigned i=0;i<n;i++){ hIn[i]=patt(i); }

    CUdeviceptr dIn,dOut;
    CK(cuMemAlloc(&dIn,bytes)); CK(cuMemAlloc(&dOut,bytes));
    printf("MEMALLOC in=0x%llx out=0x%llx bytes=%zu\n",
           (unsigned long long)dIn,(unsigned long long)dOut,bytes); fflush(stdout);
    CK(cuMemcpyHtoD(dIn,hIn,bytes));
    /* sentinel into out so an un-launched element is distinguishable from in-read-as-0 */
    { union{float f;unsigned u;} s; s.f=SENT; CK(cuMemsetD32(dOut,s.u,n)); }

    unsigned Np=n; void *args[]={ &dOut,&dIn,&Np };
    unsigned blk=256, grid=(n+blk-1)/blk;
    printf("LAUNCH grid=%u block=%u (n=%u)\n",grid,blk,n); fflush(stdout);
    CK(cuLaunchKernel(fn, grid,1,1, blk,1,1, 0,0, args,0));
    CK(cuCtxSynchronize()); printf("SYNC OK\n"); fflush(stdout);
    CK(cuMemcpyDtoH(hOut,dOut,bytes));

    /* classify */
    long bad=0, read_zero=0, sentinel=0, other=0; long first=-1; float fv=0, fexp=0;
    for(unsigned i=0;i<n;i++){
        float exp = patt(i)+1.0f;
        if(hOut[i]==exp) continue;
        bad++;
        if(first<0){ first=i; fv=hOut[i]; fexp=exp; }
        if(hOut[i]==SENT) sentinel++;
        else if(hOut[i]==1.0f) read_zero++;   /* in[i] read as 0.0 => unbacked leaf */
        else other++;
    }
    printf("CUP7 RESULT n=%u bad=%ld (read0=%ld sentinel=%ld other=%ld)\n",
           n,bad,read_zero,sentinel,other);
    if(first>=0){
        unsigned long va = (unsigned long)dIn + (unsigned long)first*4;
        printf("  FIRST bad i=%ld got=%g exp=%g  in-VA=0x%lx (grep this leaf in QEMU log)\n",
               first,fv,fexp,va);
    }
    const char *verdict = bad==0 ? "PASS=D1 (host GR read large buffer byte-exact)"
        : read_zero>other && read_zero>sentinel ? "FAIL=D2 (in read as 0 -> UNBACKED leaf)"
        : sentinel>=bad ? "INCONCLUSIVE (kernel coverage gap, not backing)"
        : "FAIL=D2 (in read as garbage -> wrong backing)";
    printf("CUP7 VERDICT: %s\n", verdict);
    printf("DONE\n"); fflush(stdout);
    return bad==0?0:2;
}
