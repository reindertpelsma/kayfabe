// chshare.c — Mode-2 channel backing isolation + sharing test (the #12 FB-phys-collision probe).
//
// Two CUDA streams == two RM channels (sharing one context VAS). We test, with NO PTX (pure
// CE memset/copy so it exercises the channel data path the scrubber/UVM channels use):
//
//   ISOLATION (the #12 false-collision case): each channel fills its OWN 64 KiB buffer with a
//   distinct pattern concurrently; after sync each buffer must read back ONLY its own pattern.
//   If our emulation collides two channels' backings onto one emulated-FB page, one buffer is
//   contaminated by the other's pattern -> FAIL (reproduces #12 in miniature).
//
//   COHERENCE (the legitimate-sharing primitive): channel 1 writes a SHARED buffer, channel 2
//   (ordered after, via event) copies it out; channel 2 must observe channel 1's bytes.
//
// Build on guest: gcc -O0 -g -o /tmp/chshare /tmp/chshare.c -lcuda
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CK(call) do { CUresult _r=(call); if(_r!=CUDA_SUCCESS){ const char*_s=0; \
  cuGetErrorString(_r,&_s); fprintf(stderr,"ERR %s -> %d %s\n",#call,_r,_s?_s:"?"); \
  exit(2);} else { printf("ok   %s\n",#call);} } while(0)

#define N    (16*1024)            // 16K u32 = 64 KiB
#define SZ   ((size_t)N*4)

static int check(const char *tag, unsigned *h, unsigned want) {
    for (int i=0;i<N;i++) if (h[i]!=want) {
        printf("  %s MISMATCH at [%d]: got 0x%08x want 0x%08x\n", tag, i, h[i], want);
        return 1;
    }
    printf("  %s OK (all 0x%08x)\n", tag, want);
    return 0;
}

int main(void){
    CK(cuInit(0));
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext ctx; CK(cuCtxCreate(&ctx,0,d));

    CUstream s1,s2; CK(cuStreamCreate(&s1,0)); CK(cuStreamCreate(&s2,0));
    CUevent e1,e2;  CK(cuEventCreate(&e1,0));  CK(cuEventCreate(&e2,0));

    CUdeviceptr a,b,sh,outc; // a=chan1 private, b=chan2 private, sh=shared, outc=coherence out
    CK(cuMemAlloc(&a,SZ)); CK(cuMemAlloc(&b,SZ)); CK(cuMemAlloc(&sh,SZ)); CK(cuMemAlloc(&outc,SZ));
    printf("ALLOC a=0x%llx b=0x%llx sh=0x%llx outc=0x%llx\n",
           (unsigned long long)a,(unsigned long long)b,
           (unsigned long long)sh,(unsigned long long)outc);

    unsigned *ha=malloc(SZ),*hb=malloc(SZ),*hc=malloc(SZ);

    // ---- ISOLATION: two channels fill their own buffers concurrently with distinct patterns.
    printf("== ISOLATION: chan1 fills a=0xA5A5A5A5, chan2 fills b=0x5B5B5B5B (concurrent) ==\n");
    CK(cuMemsetD32Async(a,0xA5A5A5A5u,N,s1));
    CK(cuMemsetD32Async(b,0x5B5B5B5Bu,N,s2));
    CK(cuStreamSynchronize(s1)); CK(cuStreamSynchronize(s2));
    CK(cuMemcpyDtoH(ha,a,SZ)); CK(cuMemcpyDtoH(hb,b,SZ));
    int fail=0;
    fail |= check("chan1 buf a", ha, 0xA5A5A5A5u);
    fail |= check("chan2 buf b", hb, 0x5B5B5B5Bu);

    // ---- COHERENCE: chan1 writes shared; chan2 (ordered after) copies it out and reads it.
    printf("== COHERENCE: chan1 writes sh=0xC0FFEE11, chan2 copies sh->outc (ordered) ==\n");
    CK(cuMemsetD32Async(sh,0xC0FFEE11u,N,s1));
    CK(cuEventRecord(e1,s1));
    CK(cuStreamWaitEvent(s2,e1,0));
    CK(cuMemcpyDtoDAsync(outc,sh,SZ,s2));
    CK(cuStreamSynchronize(s2));
    CK(cuMemcpyDtoH(hc,outc,SZ));
    fail |= check("chan2 read of chan1's shared", hc, 0xC0FFEE11u);

    printf("CHSHARE VERDICT: %s\n", fail? "FAIL (backing collision/incoherence)":"PASS (isolated + coherent)");
    CK(cuCtxDestroy(ctx));
    return fail?1:0;
}
