/* cupctx2_min_kmsg.c — #12 diag variant of tests/mode2/cupctx2_min.c.
 * Identical lifecycle (create -> destroy -> create, NO compute) but writes a
 * phase marker into /dev/kmsg before+after every CUDA call so the guest dmesg
 * interleaves the UVM MAX_JUMP / completed_value asserts with the exact CUDA
 * phase (CTX1-destroy vs CTX2-create) — that ordering decides fix (a) vs (b)
 * for the UVM tracking-semaphore pool staleness (cont.32/33). */
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>

static int kfd = -1;
static void kmsg(const char *m){
    char b[128]; int n = snprintf(b, sizeof b, "CUPCTX2MIN: %s\n", m);
    if (kfd >= 0) { ssize_t w = write(kfd, b, n); (void)w; }
    printf("%s", b); fflush(stdout);
}
#define CK(x) do{ CUresult r=(x); if(r!=CUDA_SUCCESS){ const char *s=0; \
    cuGetErrorString(r,&s); printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r); \
    fflush(stdout); return 3; } }while(0)

int main(void){
    int iters=2; const char *it=getenv("ITERS"); if(it){ iters=atoi(it); if(iters<1) iters=1; }
    kfd = open("/dev/kmsg", O_WRONLY);
    kmsg("cuInit begin");
    CK(cuInit(0));
    kmsg("cuInit done");
    int nd=0; CK(cuDeviceGetCount(&nd)); if(nd<1){ kmsg("no dev"); return 1; }
    CUdevice d; CK(cuDeviceGet(&d,0));
    for(int i=0;i<iters;i++){
        char tag[64];
        snprintf(tag,sizeof tag,"CTX%d cuCtxCreate begin",i+1); kmsg(tag);
        CUcontext c; CK(cuCtxCreate(&c,0,d));
        snprintf(tag,sizeof tag,"CTX%d cuCtxCreate done",i+1); kmsg(tag);
        snprintf(tag,sizeof tag,"CTX%d cuCtxDestroy begin",i+1); kmsg(tag);
        CK(cuCtxDestroy(c));
        snprintf(tag,sizeof tag,"CTX%d cuCtxDestroy done",i+1); kmsg(tag);
    }
    kmsg("VERDICT PASS");
    printf("CUPCTX2_MIN VERDICT: PASS (%d contexts, no compute)\nDONE\n", iters);
    return 0;
}
