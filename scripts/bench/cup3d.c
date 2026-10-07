/* ★★★★★ w305 ITEM A — cup3 WITH AN EXPLICIT `cuCtxDestroy`, so the PREEMPT path is REACHED.
 *
 * ## Why this file exists, and why it is cup3 plus a tail rather than a new program
 *
 * `w303` replaced an unconditional forged `NV_OK` for `NVA06C_CTRL_CMD_PREEMPT` (`0xa06c0105`)
 * with a decision: `NV_OK` **only** when the named channel group provably has no host twin,
 * `NV_ERR_INVALID_STATE` otherwise. Its own report named the gap it could not close:
 *
 *   "all ~15 committed boots that refused it were boots where `cuCtxCreate` had already
 *    FAILED. A refusal on the successful destroy path has never been observed."
 *
 * and it named the cause: `[measured 2026-08-14, w303]` `0xa06c0105` appears **nowhere** in
 * either crossing boot, because **`scripts/bench/cup3.c` calls `cuCtxCreate` and never
 * `cuCtxDestroy`**. The host reference trace puts the id inside an unbroken `RM_FREE`
 * cascade (record 457 of 608 in `ctx_r1`) with payload `bWait = 1` — i.e. it is a
 * **teardown** control asking *"is the engine still reading the pages I am about to free?"*.
 *
 * ⇒ The shortest path to a measurement is the crossing workload plus the call that is
 *   missing. ⊘ Deliberately NOT a fresh program: `cup3` is the only workload that has ever
 *   crossed (`CUP3_VAL=43`, w297), so anything that fails BEFORE the teardown here is
 *   attributable to the change of workload rather than to the destroy path — and keeping the
 *   compute leg byte-identical is what makes that attribution possible.
 *
 * ## ★★★ THE STAGE MARKERS ARE BYTE-IDENTICAL TO cup3.c, ON PURPOSE
 *
 * `cup3_hook.sh`'s ladder greps `^CTX OK`, `^MODULE OK`, `^FUNC OK`, `^MEMALLOC`,
 * `^LAUNCH OK`, `^SYNC OK`, `^KERNEL rv=`, `^DONE`, and its metric is `^KERNEL rv=`. Every
 * one of those is emitted here, unchanged and in the same order, so this program is graded by
 * the **proven** hook rather than by a second copy of it that could drift.
 * ⊘ The teardown lines are ADDED AFTER `DONE`, never interleaved, so the existing ladder
 *   cannot change meaning because of them.
 *
 * ## ⊘ THE TEARDOWN IS NOT WRAPPED IN `CK()`, AND THAT IS THE POINT
 *
 * `CK()` returns on the first failure. A teardown that aborts on its first non-OK result
 * would hide every later call — and the pre-registered outcome (c) of this rung is precisely
 * *"the guest FAILS where it did not before"*, which needs each teardown call's status
 * reported **individually and unconditionally**. So each one prints its own
 * `DESTROY_<stage>_RC=` line whatever it answers, and the program keeps going.
 *
 * ⚠ `cuCtxDestroy`'s own result is printed as `CUP3D_CTXDESTROY_RC=` on its own anchored
 *   line: it is the single number that says whether our PREEMPT answer was ACCEPTED by the
 *   guest's CUDA runtime, and it must never have to be inferred from an exit code that also
 *   carries the compute verdict.
 */
#include <cuda.h>
#include <stdio.h>
#define CK(x) do{ CUresult r=(x); const char*s=0; if(r!=CUDA_SUCCESS){ \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 1;} else { printf("ok   %s\n",#x); fflush(stdout);} }while(0)

/* ⊘ UNCONDITIONAL: reports, never returns. See the header note on why the teardown must not
 * short-circuit — the whole question is what EVERY call on this path answers. */
#define TD(tag,x) do{ CUresult r=(x); const char*s=0; cuGetErrorString(r,&s); \
    printf("DESTROY_%s_RC=%d (%s)\n",tag,(int)r,s?s:"?"); fflush(stdout); }while(0)

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

    /* ------------------------------------------------------------------------------------
     * ★★★★★ THE RUNG. Everything above this line is cup3, byte for byte.
     *
     * The order is the ordinary CUDA teardown order — frees, then the module, then the
     * context — because the question is what a NORMAL destroy path does, not what an
     * artificial one can be made to do. `cuCtxDestroy` is the call the `RM_FREE` cascade
     * carrying `0xa06c0105` hangs off in the host reference trace.
     * ---------------------------------------------------------------------------------- */
    printf("TEARDOWN BEGIN\n"); fflush(stdout);
    TD("MEMFREE_IN",  cuMemFree(d_in));
    TD("MEMFREE_OUT", cuMemFree(d_out));
    TD("MODUNLOAD",   cuModuleUnload(mod));

    /* ★★★★★ THE ONE THAT MATTERS. Its own anchored line, because a runner must be able to
     * read "did the guest accept our PREEMPT answer" without decoding an exit status that
     * also carries the compute verdict. */
    {
        CUresult r = cuCtxDestroy(ctx); const char *s = 0; cuGetErrorString(r,&s);
        printf("CUP3D_CTXDESTROY_RC=%d\n", (int)r);
        printf("CUP3D_CTXDESTROY_STR=%s\n", s?s:"?");
        fflush(stdout);
    }
    printf("TEARDOWN DONE\n"); fflush(stdout);

    /* ⊘ The exit code still carries the COMPUTE verdict and only that, exactly as cup3's
     * does, so `CUP3_RC` remains comparable across the two workloads. The teardown's verdict
     * travels on its own lines above; folding it in here would make one number mean two
     * things and neither readable. */
    return rv==want?0:2;
}
