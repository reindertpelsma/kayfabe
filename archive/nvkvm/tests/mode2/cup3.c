/* Mode-2 GR KERNEL-LAUNCH proof (north star, step 1): cuCtxCreate -> module load ->
 * cuLaunchKernel -> DtoH. The kernel computes out[0] = in[0]*3 + 1 ON THE GPU. The
 * QEMU CE emulator can only copy/memset (no arithmetic), so a correct out=43 from in=14
 * is UN-FORGEABLE proof the host GR engine genuinely ran the shader. Each call wrapped. */
#include <cuda.h>
#include <stdio.h>
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)

/* PTX for sm_86: k(out,in){ *out = *in*3 + 1; } — JITed by libnvidia-ptxjitcompiler. */
static const char *PTX =
".version 7.8\n.target sm_86\n.address_size 64\n"
".visible .entry k(.param .u64 p_out, .param .u64 p_in){\n"
"  .reg .u64 %rd<3>; .reg .u32 %r<3>;\n"
"  ld.param.u64 %rd1, [p_out];\n"
"  ld.param.u64 %rd2, [p_in];\n"
"  cvta.to.global.u64 %rd1, %rd1;\n"
"  cvta.to.global.u64 %rd2, %rd2;\n"
"  ld.global.u32 %r1, [%rd2];\n"
"  mul.lo.s32 %r2, %r1, 3;\n"
"  add.s32 %r2, %r2, 1;\n"
"  st.global.u32 [%rd1], %r2;\n"
"  ret;\n}\n";

int main(void){
    CK(cuInit(0));
    int n=0; CK(cuDeviceGetCount(&n)); if(n<1){printf("no dev\n");return 1;}
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d));
    printf("CTX OK\n"); fflush(stdout);

    CUmodule mod; CK(cuModuleLoadData(&mod,PTX));
    printf("MODULE OK\n"); fflush(stdout);
    CUfunction fn; CK(cuModuleGetFunction(&fn,mod,"k"));
    printf("FUNC OK\n"); fflush(stdout);

    CUdeviceptr d_in, d_out;
    CK(cuMemAlloc(&d_in,4)); CK(cuMemAlloc(&d_out,4));
    printf("MEMALLOC in=0x%llx out=0x%llx\n",(unsigned long long)d_in,(unsigned long long)d_out);
    fflush(stdout);
    unsigned hv=14, rv=0xeeee;
    CK(cuMemcpyHtoD(d_in,&hv,4));
    CK(cuMemsetD32(d_out,0,1));                 /* clear out so a stale copy can't fake it */

    void *args[] = { &d_out, &d_in };
    CK(cuLaunchKernel(fn, 1,1,1, 1,1,1, 0, 0, args, 0));
    printf("LAUNCH OK\n"); fflush(stdout);
    CK(cuCtxSynchronize());
    printf("SYNC OK\n"); fflush(stdout);

    CK(cuMemcpyDtoH(&rv,d_out,4));
    unsigned want = hv*3+1;                      /* 43 */
    printf("KERNEL rv=%u want=%u -> %s\n", rv, want, rv==want?"PASS":"MISMATCH");
    printf("DONE\n"); fflush(stdout);
    return rv==want?0:2;
}
