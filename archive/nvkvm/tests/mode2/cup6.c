// cup6.c — DISCRIMINATOR for the cup5 bulk-copy slowness: UC cacheability vs
// per-page demand-fault/residency overhead. Same alloc as cup5, but:
//   - times HtoD TWICE to the SAME buffer. UC writes stay slow forever; a
//     first-touch fault/residency cost makes copy #2 jump to memory speed.
//   - holds the buffer + sleeps so the harness can read the LITERAL memtype
//     from /sys/kernel/debug/x86/pat_memtype_list (write-back vs uncached-minus
//     vs write-combining) while the mapping is live.
// Build: gcc -O2 -o cup6 cup6.c -lcuda
#include <cuda.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <stdint.h>
#include <inttypes.h>
#include <fcntl.h>
#define CK(x) do{ CUresult r=(x); if(r){ const char*s=0; cuGetErrorString(r,&s); \
  printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",(int)r); fflush(stdout); return 1; } }while(0)
static double now(void){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t);
  return (double)t.tv_sec + (double)t.tv_nsec*1e-9; }

/* Self-introspection: dump this process's /dev/nvidia* + large mappings, resolve
 * each to a guest-physical address via /proc/self/pagemap, and flag the buffer
 * sized == want_n (dp). Answers: is dp's CPU mapping backed by guest RAM (GPA in
 * the 0x1_0000_0000 window) and what physical range to match in pat_memtype_list.
 * Pure guest userspace — no GPU/QEMU change. */
static uint64_t va_to_phys(int pmfd, uint64_t va, int *present)
{
    uint64_t e = 0; *present = 0;
    if (pread(pmfd, &e, 8, (off_t)((va >> 12) * 8)) != 8) return 0;
    *present = (int)((e >> 63) & 1);
    if (!*present) return 0;
    return ((e & ((1ULL << 55) - 1)) << 12) | (va & 0xfff);
}
static void dump_nvidia_maps(size_t want_n)
{
    FILE *m = fopen("/proc/self/maps", "r");
    int pmfd = open("/proc/self/pagemap", O_RDONLY);
    if (!m || pmfd < 0) { printf("MAPS: open failed (need root for pagemap PFNs)\n");
                          if (m) fclose(m); if (pmfd >= 0) close(pmfd); return; }
    char line[512];
    printf("==== cup6 mapping introspection (dp size=0x%zx) ====\n", want_n);
    while (fgets(line, sizeof line, m)) {
        uint64_t a, b; char perms[8]; char path[256] = "";
        int n = sscanf(line, "%" SCNx64 "-%" SCNx64 " %7s %*s %*s %*s %255[^\n]",
                       &a, &b, perms, path);
        if (n < 3) continue;
        size_t sz = (size_t)(b - a);
        int is_nv  = strstr(path, "nvidia") != NULL;
        int is_big = sz >= (want_n / 2);
        if (!is_nv && !is_big) continue;
        int pr0 = 0, prm = 0;
        uint64_t p0 = va_to_phys(pmfd, a, &pr0);
        uint64_t pm = va_to_phys(pmfd, a + sz / 2, &prm);
        int guestram = (p0 >= 0x100000000ULL && p0 < 0x180000000ULL);
        const char *tag = (sz == want_n) ? "  <== DP-SIZED" :
                          (is_nv ? "  (nvidia)" : "  (big)");
        printf("MAP va=0x%" PRIx64 "-0x%" PRIx64 " sz=0x%zx perms=%s "
               "phys0=0x%" PRIx64 "(pr=%d) physmid=0x%" PRIx64 "(pr=%d) "
               "guestRAMwin=%d path=%s%s\n",
               a, b, sz, perms, p0, pr0, pm, prm, guestram,
               path[0] ? path : "(anon)", tag);
    }
    printf("==== end introspection ====\n");
    fclose(m); close(pmfd); fflush(stdout);
}
int main(void)
{
    size_t MB = getenv("CUP6_MB") ? (size_t)atoi(getenv("CUP6_MB")) : 64;
    size_t N = MB << 20;
    CK(cuInit(0));
    CUdevice d; CK(cuDeviceGet(&d,0));
    CUcontext c; CK(cuCtxCreate(&c,0,d));
    CUdeviceptr dp; CK(cuMemAlloc(&dp,N));
    printf("alloc %zuMB dp=0x%llx pid=%d OK\n", MB, (unsigned long long)dp, (int)getpid());
    fflush(stdout);
    unsigned char *h = malloc(N);
    if(!h){ printf("FAIL host malloc\n"); return 1; }
    for(size_t i=0;i<N;i++) h[i]=(unsigned char)((i*131u+7u)&0xff);

    // copy #1 (first touch — pays any fault/residency cost)
    double t0=now(); CK(cuMemcpyHtoD(dp,h,N)); CK(cuCtxSynchronize()); double t1=now();
    printf("HtoD#1 %.3fs (%.1f MB/s)\n", t1-t0, (double)MB/(t1-t0)); fflush(stdout);
    // copy #2 (same buffer — already resident if it was a first-touch cost)
    double t2=now(); CK(cuMemcpyHtoD(dp,h,N)); CK(cuCtxSynchronize()); double t3=now();
    printf("HtoD#2 %.3fs (%.1f MB/s)\n", t3-t2, (double)MB/(t3-t2)); fflush(stdout);
    // copy #3 (third, confirm steady state)
    double t4=now(); CK(cuMemcpyHtoD(dp,h,N)); CK(cuCtxSynchronize()); double t5=now();
    printf("HtoD#3 %.3fs (%.1f MB/s)\n", t5-t4, (double)MB/(t5-t4)); fflush(stdout);
    printf("INTERPRET: #2/#3 >> #1 => per-page first-touch fault/residency; "
           "#1~#2~#3 (all slow) => persistent (UC / per-copy overhead)\n");
    fflush(stdout);
    dump_nvidia_maps(N);
    printf("HOLDING buffer 25s — read pat_memtype_list now\n"); fflush(stdout);
    sleep(25);
    return 0;
}
