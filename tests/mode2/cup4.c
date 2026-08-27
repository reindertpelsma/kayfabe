/* Mode-2 GR matmul proof (north star, step 2): a REAL NxN fp32 matmul kernel runs on the
 * host GR engine through the emulated GA106. N*N threads (1 block, NxN), each computes one
 * C[row,col] = sum_k A[row,k]*B[k,col]. Un-forgeable (the CE software path can't multiply/sum)
 * and exercises a larger working set + many threads (util>0) vs cup3's 1-thread kernel.
 * Verifies against a CPU reference with fp tolerance. */
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#define N 16
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)

/* mm(C,A,B,N): C[ty,tx] = sum_k A[ty,k]*B[k,tx]; launched block=(N,N), grid=(1,1). */
static const char *PTX =
".version 7.8\n.target sm_86\n.address_size 64\n"
".visible .entry mm(.param .u64 pC,.param .u64 pA,.param .u64 pB,.param .u32 pN){\n"
"  .reg .pred %p<2>; .reg .f32 %f<5>; .reg .b32 %r<9>; .reg .b64 %rd<10>;\n"
"  ld.param.u64 %rd1,[pC]; ld.param.u64 %rd2,[pA]; ld.param.u64 %rd3,[pB];\n"
"  ld.param.u32 %r1,[pN];\n"
"  cvta.to.global.u64 %rd1,%rd1; cvta.to.global.u64 %rd2,%rd2; cvta.to.global.u64 %rd3,%rd3;\n"
"  mov.u32 %r2,%tid.x;            // col\n"
"  mov.u32 %r3,%tid.y;            // row\n"
"  mov.f32 %f1,0f00000000;        // acc\n"
"  mov.u32 %r4,0;                 // k\n"
"$L_loop:\n"
"  setp.ge.u32 %p1,%r4,%r1;\n"
"  @%p1 bra $L_done;\n"
"  mul.lo.s32 %r5,%r3,%r1; add.s32 %r5,%r5,%r4;   // row*N+k\n"
"  mul.wide.u32 %rd4,%r5,4; add.s64 %rd5,%rd2,%rd4; ld.global.f32 %f2,[%rd5];\n"
"  mul.lo.s32 %r6,%r4,%r1; add.s32 %r6,%r6,%r2;   // k*N+col\n"
"  mul.wide.u32 %rd6,%r6,4; add.s64 %rd7,%rd3,%rd6; ld.global.f32 %f3,[%rd7];\n"
"  fma.rn.f32 %f1,%f2,%f3,%f1;\n"
"  add.s32 %r4,%r4,1; bra $L_loop;\n"
"$L_done:\n"
"  mul.lo.s32 %r7,%r3,%r1; add.s32 %r7,%r7,%r2;   // row*N+col\n"
"  mul.wide.u32 %rd8,%r7,4; add.s64 %rd9,%rd1,%rd8; st.global.f32 [%rd9],%f1;\n"
"  ret;\n}\n";

int main(void){
    CK(cuInit(0));
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){printf("no dev\n");return 1;}
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d)); printf("CTX OK\n"); fflush(stdout);
    CUmodule mod; CK(cuModuleLoadData(&mod,PTX)); printf("MODULE OK\n"); fflush(stdout);
    CUfunction fn; CK(cuModuleGetFunction(&fn,mod,"mm")); printf("FUNC OK\n"); fflush(stdout);

    const int n=N, sz=N*N*sizeof(float);
    float *hA=malloc(sz), *hB=malloc(sz), *hC=malloc(sz), *ref=malloc(sz);
    for(int i=0;i<n*n;i++){ hA[i]=(float)((i*7+3)%11); hB[i]=(float)((i*5+1)%13); }
    for(int r=0;r<n;r++)for(int c=0;c<n;c++){ float a=0; for(int k=0;k<n;k++) a+=hA[r*n+k]*hB[k*n+c]; ref[r*n+c]=a; }

    CUdeviceptr dA,dB,dC;
    CK(cuMemAlloc(&dA,sz)); CK(cuMemAlloc(&dB,sz)); CK(cuMemAlloc(&dC,sz));
    printf("MEMALLOC A=0x%llx B=0x%llx C=0x%llx\n",(unsigned long long)dA,(unsigned long long)dB,(unsigned long long)dC);
    fflush(stdout);
    CK(cuMemcpyHtoD(dA,hA,sz)); CK(cuMemcpyHtoD(dB,hB,sz)); CK(cuMemsetD32(dC,0,n*n));

    int Np=n; void *args[]={ &dC,&dA,&dB,&Np };
    CK(cuLaunchKernel(fn, 1,1,1, n,n,1, 0,0, args,0)); printf("LAUNCH OK\n"); fflush(stdout);
    CK(cuCtxSynchronize()); printf("SYNC OK\n"); fflush(stdout);
    CK(cuMemcpyDtoH(hC,dC,sz));

    int bad=0; float maxerr=0;
    for(int i=0;i<n*n;i++){ float e=fabsf(hC[i]-ref[i]); if(e>maxerr)maxerr=e; if(e>1e-3f)bad++; }
    printf("MATMUL N=%d bad=%d maxerr=%g  C[0]=%g ref[0]=%g  C[last]=%g ref[last]=%g -> %s\n",
           n,bad,maxerr,hC[0],ref[0],hC[n*n-1],ref[n*n-1], bad==0?"PASS":"MISMATCH");
    printf("DONE\n"); fflush(stdout);
    return bad==0?0:2;
}
