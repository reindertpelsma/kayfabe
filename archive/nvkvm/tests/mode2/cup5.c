// cup5.c — Mode-2 BULK data-plane forcing function (driver API; the deterministic,
// llama-free reproducer for the LLM model-load hang). cup2 only moves 4 KB so it
// passes via QEMU's CE emulation; the LLM moves ~469 MB of weights HtoD and stalls
// (host util ~0% = the copy is CPU-emulated, not run on the host CE engine — the
// M5.49b "CE data plane is QEMU-emulated" gap). cup5 does a LARGE cuMemcpyHtoD +
// cuCtxSynchronize + DtoH byte-verify (size = $CUP5_MB, default 64) and TIMES it, so
// the outcome cleanly classifies the blocker:
//   PASS fast + host util>0  -> host CE genuinely moved the data; LLM hang is cudart-specific.
//   PASS but glacial, util~0 -> QEMU CPU-emulated copy; fix = forward the copy to host CE.
//   HANG / FAULT             -> bulk residency/completion broken.
// Build: gcc -O2 -o cup5 cup5.c -lcuda
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#define CK(x) do{ CUresult r=(x); if(r){ const char*s=0; cuGetErrorString(r,&s); \
  printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",(int)r); fflush(stdout); return 1; } }while(0)
static double now(void){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t);
  return (double)t.tv_sec + (double)t.tv_nsec*1e-9; }
int main(void)
{
    size_t MB = getenv("CUP5_MB") ? (size_t)atoi(getenv("CUP5_MB")) : 64;
    size_t N = MB << 20;
    CK(cuInit(0));
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext c; CK(cuCtxCreate(&c,0,d));
    CUdeviceptr dp; CK(cuMemAlloc(&dp,N));
    printf("alloc %zuMB dp=0x%llx OK\n", MB, (unsigned long long)dp); fflush(stdout);
    unsigned char *h = malloc(N), *o = malloc(N);
    if(!h||!o){ printf("FAIL host malloc\n"); return 1; }
    for(size_t i=0;i<N;i++) h[i]=(unsigned char)((i*131u+7u)&0xff);
    memset(o,0,N);
    printf("HtoD %zuMB ...\n",MB); fflush(stdout);
    double t0=now(); CK(cuMemcpyHtoD(dp,h,N)); CK(cuCtxSynchronize()); double t1=now();
    printf("HtoD done %.2fs (%.1f MB/s)\n", t1-t0, (double)MB/(t1-t0)); fflush(stdout);
    double t2=now(); CK(cuMemcpyDtoH(o,dp,N)); CK(cuCtxSynchronize()); double t3=now();
    printf("DtoH done %.2fs (%.1f MB/s)\n", t3-t2, (double)MB/(t3-t2)); fflush(stdout);
    size_t bad=0;
    for(size_t i=0;i<N;i++) if(h[i]!=o[i]){ bad++; if(bad<5) printf("  mismatch @%zu %02x!=%02x\n",i,h[i],o[i]); }
    printf("VERIFY %zuMB bad=%zu -> %s\n", MB, bad, bad?"FAIL":"PASS"); fflush(stdout);
    return bad?1:0;
}
