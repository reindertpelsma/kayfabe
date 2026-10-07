/* cupctx2_min.c — MINIMAL #12 repro: create -> destroy -> create, NO compute.
 * Strips cupctx2 down to just the context lifecycle so we can tell whether the
 * 2nd-context hang requires CTX1 to have done compute (which drives the singleton
 * CeUtils scrub finishPayload up via the usermode doorbell) or reproduces from a
 * bare create/destroy cycle. No module load, no H2D, no launch, no sync.
 *   ITERS (default 2) = number of create/destroy cycles.
 * Expected if #12 is the create-lifecycle scrub wait: CTX2 cuCtxCreate hangs
 *   (last line "[CTX2] cuCtxCreate..."), rc=124 under `timeout`.
 * If it PASSES: the hang needs CTX1 compute first (singleton advanced past CTX2's
 *   reachable target) — a different, more specific confirmation.
 */
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>

#define CK(x) do{ CUresult r=(x); if(r!=CUDA_SUCCESS){ const char *s=0; \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 3; } }while(0)
#define MK(...) do{ printf(__VA_ARGS__); fflush(stdout); }while(0)

int main(void){
    int iters=2; const char *it=getenv("ITERS"); if(it){ iters=atoi(it); if(iters<1) iters=1; }
    CK(cuInit(0));
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){ MK("no dev\n"); return 1; }
    CUdevice d; CK(cuDeviceGet(&d,0));
    MK("CUPCTX2_MIN iters=%d (create->destroy only, NO compute)\n", iters);

    for(int i=0;i<iters;i++){
        char tag[16]; snprintf(tag,sizeof tag,"CTX%d",i+1);
        MK("[%s] cuCtxCreate...\n", tag);
        CUcontext c; CK(cuCtxCreate(&c,0,d));
        MK("[%s] CTX OK\n", tag);
        MK("[%s] cuCtxDestroy...\n", tag);
        CK(cuCtxDestroy(c));
        MK("[%s] CTX DESTROY OK\n", tag);
    }
    MK("CUPCTX2_MIN VERDICT: PASS (%d contexts, no compute)\n", iters);
    MK("DONE\n");
    return 0;
}
