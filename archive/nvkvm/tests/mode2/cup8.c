/* Mode-2 GR matmul SCALE proof (map-on-touch generalization, 2026-06-15): a REAL NxN fp32
 * matmul on the host GR engine through the emulated GA106, at a size whose A/B/C buffers
 * exceed what the host's 256 MiB BAR1 can CPU-map alongside the ctx — the regime where the
 * D2 wall hit. Unlike cup4 (N=16, one block), this is a 2D GRID (16x16 blocks) so the kernel
 * genuinely reads ALL of A and B and writes ALL of C; a backing hole anywhere -> wrong C.
 *
 * Closed-form inputs so we verify in O(N^2) (no slow O(N^3) CPU reference): A[i][k]=(i&3)+1,
 * B[k][j]=(j&3)+1 (each independent of k) -> C[i][j] = N*((i&3)+1)*((j&3)+1), fp32-exact for
 * N up to ~1M. The kernel STILL loads every A[i*N+k] / B[k*N+j] across the full buffers, so
 * an unbacked/garbage leaf shows up as a value mismatch. Default N=2048 (16 MiB per matrix,
 * 48 MiB device); CUP8_N overrides. PASS => map-on-touch holds for general compute. */
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)

/* mm(C,A,B,N): C[row,col]=sum_k A[row,k]*B[k,col]; 2D grid, block=(16,16). */
static const char *PTX =
".version 7.8\n.target sm_86\n.address_size 64\n"
".visible .entry mm(.param .u64 pC,.param .u64 pA,.param .u64 pB,.param .u32 pN){\n"
"  .reg .pred %p<4>; .reg .f32 %f<5>; .reg .b32 %r<16>; .reg .b64 %rd<10>;\n"
"  ld.param.u64 %rd1,[pC]; ld.param.u64 %rd2,[pA]; ld.param.u64 %rd3,[pB];\n"
"  ld.param.u32 %r1,[pN];\n"
"  cvta.to.global.u64 %rd1,%rd1; cvta.to.global.u64 %rd2,%rd2; cvta.to.global.u64 %rd3,%rd3;\n"
"  mov.u32 %r2,%ntid.x; mov.u32 %r3,%ctaid.x; mov.u32 %r4,%tid.x; mad.lo.s32 %r5,%r3,%r2,%r4;  // col\n"
"  mov.u32 %r6,%ntid.y; mov.u32 %r7,%ctaid.y; mov.u32 %r8,%tid.y; mad.lo.s32 %r9,%r7,%r6,%r8;  // row\n"
"  setp.ge.u32 %p1,%r5,%r1; setp.ge.u32 %p2,%r9,%r1; or.pred %p3,%p1,%p2; @%p3 bra $L_ret;\n"
"  mov.f32 %f1,0f00000000; mov.u32 %r10,0;\n"
"$L_loop:\n"
"  setp.ge.u32 %p1,%r10,%r1; @%p1 bra $L_done;\n"
"  mad.lo.s32 %r11,%r9,%r1,%r10;   // row*N+k\n"
"  mul.wide.u32 %rd4,%r11,4; add.s64 %rd5,%rd2,%rd4; ld.global.f32 %f2,[%rd5];\n"
"  mad.lo.s32 %r12,%r10,%r1,%r5;   // k*N+col\n"
"  mul.wide.u32 %rd6,%r12,4; add.s64 %rd7,%rd3,%rd6; ld.global.f32 %f3,[%rd7];\n"
"  fma.rn.f32 %f1,%f2,%f3,%f1;\n"
"  add.s32 %r10,%r10,1; bra $L_loop;\n"
"$L_done:\n"
"  mad.lo.s32 %r13,%r9,%r1,%r5;    // row*N+col\n"
"  mul.wide.u32 %rd8,%r13,4; add.s64 %rd9,%rd1,%rd8; st.global.f32 [%rd9],%f1;\n"
"$L_ret:\n"
"  ret;\n}\n";

int main(void){
    unsigned N = 2048; const char *e = getenv("CUP8_N"); if(e){ N=(unsigned)atoi(e); }
    N = (N + 15u) & ~15u; if(!N) N=16;
    CK(cuInit(0));
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){printf("no dev\n");return 1;}
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d)); printf("CTX OK\n"); fflush(stdout);
    CUmodule mod; CK(cuModuleLoadData(&mod,PTX)); printf("MODULE OK\n"); fflush(stdout);
    CUfunction fn; CK(cuModuleGetFunction(&fn,mod,"mm")); printf("FUNC OK\n"); fflush(stdout);

    size_t sz = (size_t)N*N*sizeof(float);
    printf("CUP8 matmul N=%u  per-matrix=%zu MiB  device=%zu MiB\n",
           N, sz>>20, (3*sz)>>20); fflush(stdout);
    float *hA=malloc(sz), *hB=malloc(sz), *hC=malloc(sz);
    if(!hA||!hB||!hC){ printf("OOM host\n"); return 1; }
    for(unsigned i=0;i<N;i++) for(unsigned k=0;k<N;k++) hA[(size_t)i*N+k]=(float)((i&3u)+1u);
    for(unsigned k=0;k<N;k++) for(unsigned j=0;j<N;j++) hB[(size_t)k*N+j]=(float)((j&3u)+1u);

    CUdeviceptr dA,dB,dC;
    CK(cuMemAlloc(&dA,sz)); CK(cuMemAlloc(&dB,sz)); CK(cuMemAlloc(&dC,sz));
    printf("MEMALLOC A=0x%llx B=0x%llx C=0x%llx\n",(unsigned long long)dA,
           (unsigned long long)dB,(unsigned long long)dC); fflush(stdout);
    CK(cuMemcpyHtoD(dA,hA,sz)); CK(cuMemcpyHtoD(dB,hB,sz)); CK(cuMemsetD32(dC,0,(size_t)N*N));

    unsigned Np=N; void *args[]={ &dC,&dA,&dB,&Np };
    unsigned g=(N+15u)/16u;
    printf("LAUNCH grid=(%u,%u) block=(16,16)\n",g,g); fflush(stdout);
    CK(cuLaunchKernel(fn, g,g,1, 16,16,1, 0,0, args,0)); printf("LAUNCH OK\n"); fflush(stdout);
    CK(cuCtxSynchronize()); printf("SYNC OK\n"); fflush(stdout);
    CK(cuMemcpyDtoH(hC,dC,sz));

    long bad=0; float maxerr=0; long firsti=-1;
    for(unsigned i=0;i<N && bad<8;i++) for(unsigned j=0;j<N;j++){
        float ref=(float)N*(float)((i&3u)+1u)*(float)((j&3u)+1u);
        float er=fabsf(hC[(size_t)i*N+j]-ref); if(er>maxerr)maxerr=er;
        if(er>1e-3f){ if(firsti<0){ firsti=(long)((size_t)i*N+j);
            printf("  FIRST bad i=%u j=%u got=%g exp=%g\n",i,j,hC[(size_t)i*N+j],ref); } bad++; }
    }
    printf("CUP8 RESULT N=%u bad=%ld maxerr=%g C[0]=%g(exp %g) -> %s\n",
           N,bad,maxerr,hC[0],(float)N*1.0f*1.0f, bad==0?"PASS":"MISMATCH");
    printf("CUP8 VERDICT: %s\n", bad==0 ? "PASS (host GR matmul byte-exact at scale)"
                                        : "FAIL (matmul wrong -> backing hole / map-on-touch gap)");
    printf("DONE\n"); fflush(stdout);
    return bad==0?0:2;
}
