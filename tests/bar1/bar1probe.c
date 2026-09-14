/* bar1probe.c — how much of ONE reserved vidmem object can be CPU-mapped at once?
 *
 * Deliberately independent of the kayfabe stack: raw RM ioctls, no shared bookkeeping.
 * That independence IS the instrument — if both agree on a ceiling, the ceiling is the
 * driver's and not ours.
 *
 * ABI transcribed from kayfabe-abi (which cites ogkm-580 headers):
 *   NV_IOCTL_MAGIC 'F'; NV_ESC_RM_FREE 0x29, NV_ESC_RM_ALLOC 0x2b,
 *   NV_ESC_RM_MAP_MEMORY 0x4e, NV_ESC_RM_UNMAP_MEMORY 0x4f, NV_ESC_REGISTER_FD 201
 *   NVOS21 (32B), NVOS00 (16B), NVOS33-with-fd (56B), NV_MEMORY_ALLOCATION_PARAMS (128B)
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <stdint.h>

#define NVM 'F'
#define ESC_FREE       0x29
#define ESC_ALLOC      0x2b
#define ESC_MAP_MEM    0x4e
#define ESC_UNMAP_MEM  0x4f
#define ESC_REGISTER_FD 201

#define NV01_ROOT_CLIENT      0x41
#define NV01_DEVICE_0         0x80
#define NV20_SUBDEVICE_0      0x2080
#define NV01_MEMORY_LOCAL_USER 0x0040
#define ATTR_NONCONTIG_VIDMEM (1u << 27)
#define ATTR_CONTIG_VIDMEM    (2u << 27)

#define IOWR(nr, sz) _IOC(_IOC_READ|_IOC_WRITE, NVM, (nr), (sz))

static int ctlfd = -1, gpufd = -1;
static uint32_t hClient, hDevice, hSubdev;
static const char *DEVDIR = "/dev";
static int gpu_index = 0;

/* ---- NVOS21: hRoot,hParent,hNew,hClass,pParms(8),paramsSize,status = 32B ---- */
static int rm_alloc(uint32_t root, uint32_t parent, uint32_t want, uint32_t cls,
                    void *parms, uint32_t psize, uint32_t *out_handle, uint32_t *out_status)
{
    uint8_t a[32]; memset(a,0,sizeof a);
    *(uint32_t*)(a+0)=root; *(uint32_t*)(a+4)=parent; *(uint32_t*)(a+8)=want;
    *(uint32_t*)(a+12)=cls; *(uint64_t*)(a+16)=(uint64_t)(uintptr_t)parms;
    *(uint32_t*)(a+24)=psize;
    int rc = ioctl(ctlfd, IOWR(ESC_ALLOC, sizeof a), a);
    if (out_status) *out_status = *(uint32_t*)(a+28);
    if (out_handle) *out_handle = *(uint32_t*)(a+8);
    if (rc < 0) return -1;
    return *(uint32_t*)(a+28) == 0 ? 0 : -2;
}

static int rm_free(uint32_t root, uint32_t parent, uint32_t obj)
{
    uint8_t a[16]; memset(a,0,sizeof a);
    *(uint32_t*)(a+0)=root; *(uint32_t*)(a+4)=parent; *(uint32_t*)(a+8)=obj;
    int rc = ioctl(ctlfd, IOWR(ESC_FREE, sizeof a), a);
    if (rc < 0) return -1;
    return *(uint32_t*)(a+12) == 0 ? 0 : -2;
}

/* NVOS33 w/ fd: hClient,hDevice,hMemory,pad,offset,length,pLinear,status,flags,fd,pad=56 */
static int rm_map_memory(uint32_t hMem, uint64_t off, uint64_t len, uint32_t flags,
                         int fd, uint32_t *out_status, int *out_errno, uint64_t *out_cookie)
{
    uint8_t a[56]; memset(a,0,sizeof a);
    *(uint32_t*)(a+0)=hClient; *(uint32_t*)(a+4)=hDevice; *(uint32_t*)(a+8)=hMem;
    *(uint64_t*)(a+16)=off; *(uint64_t*)(a+24)=len;
    *(uint32_t*)(a+44)=flags; *(int32_t*)(a+48)=fd;
    errno = 0;
    int rc = ioctl(ctlfd, IOWR(ESC_MAP_MEM, sizeof a), a);
    *out_errno = errno;
    *out_status = *(uint32_t*)(a+40);
    if (out_cookie) *out_cookie = *(uint64_t*)(a+32);   /* pLinearAddress [OUT] cookie */
    if (rc < 0) return -1;
    return *out_status == 0 ? 0 : -2;
}

/* NVOS34: hClient,hDevice,hMemory,pad,pLinearAddress,status,flags = 32B */
static int rm_unmap_memory(uint32_t hMem, uint64_t cookie, uint32_t *out_status, int *out_errno)
{
    uint8_t a[32]; memset(a,0,sizeof a);
    *(uint32_t*)(a+0)=hClient; *(uint32_t*)(a+4)=hDevice; *(uint32_t*)(a+8)=hMem;
    *(uint64_t*)(a+16)=cookie;
    errno = 0;
    int rc = ioctl(ctlfd, IOWR(ESC_UNMAP_MEM, sizeof a), a);
    *out_errno = errno;
    *out_status = *(uint32_t*)(a+24);
    if (rc < 0) return -1;
    return *out_status == 0 ? 0 : -2;
}

static const char *nvstatus(uint32_t s) {
    switch (s) {
    case 0x00: return "NV_OK";
    case 0x1e: return "NV_ERR_INSUFFICIENT_RESOURCES";
    case 0x23: return "NV_ERR_INVALID_CLIENT";
    case 0x2b: return "NV_ERR_INVALID_LIMIT";
    case 0x30: return "NV_ERR_INVALID_OBJECT";
    case 0x33: return "NV_ERR_INVALID_OBJECT_HANDLE";
    case 0x38: return "NV_ERR_INVALID_PARAMETER";
    case 0x3b: return "NV_ERR_INVALID_STATE";
    case 0x42: return "NV_ERR_NOT_SUPPORTED";
    case 0x51: return "NV_ERR_NO_MEMORY";
    case 0x56: return "NV_ERR_OBJECT_NOT_FOUND";
    case 0x57: return "NV_ERR_OPERATING_SYSTEM";
    default: return "NV_ERR_<unmapped>";
    }
}

static void smi_bar1(const char *tag) {
    FILE *p = popen("nvidia-smi -q 2>/dev/null | awk '/BAR1 Memory Usage/{f=1;next} f&&/Total|Used|Free/{printf \"%s \", $(NF-1); c++} c>=3{exit}'", "r");
    char buf[128] = {0};
    if (p) { if (fgets(buf, sizeof buf, p) == NULL) buf[0]=0; pclose(p); }
    printf("  [SMI %-14s BAR1 total/used/free MiB = %s]\n", tag, buf[0]?buf:"(unavailable)");
    fflush(stdout);
}

static int open_gpu_node(void) {
    char path[64];
    snprintf(path, sizeof path, "%s/nvidia%d", DEVDIR, gpu_index);
    return open(path, O_RDWR | O_CLOEXEC);
}

static int bringup(void) {
    char path[64];
    ctlfd = open("/dev/nvidiactl", O_RDWR | O_CLOEXEC);
    if (ctlfd < 0) { perror("open /dev/nvidiactl"); return -1; }
    snprintf(path, sizeof path, "%s/nvidia%d", DEVDIR, gpu_index);
    gpufd = open(path, O_RDWR | O_CLOEXEC);
    if (gpufd < 0) { perror("open /dev/nvidiaN"); return -1; }
    int reg = ctlfd;
    if (ioctl(gpufd, IOWR(ESC_REGISTER_FD, sizeof(int)), &reg) < 0) {
        perror("REGISTER_FD"); return -1;
    }
    uint32_t st;
    if (rm_alloc(0, 0, 0xc1d00001, NV01_ROOT_CLIENT, NULL, 0, &hClient, &st) != 0) {
        printf("FAIL root client status=%#x (%s) errno=%d\n", st, nvstatus(st), errno); return -1;
    }
    uint8_t dev[56]; memset(dev,0,sizeof dev);
    *(uint32_t*)(dev+0) = gpu_index;               /* deviceId */
    if (rm_alloc(hClient, hClient, 0xc1d00002, NV01_DEVICE_0, dev, sizeof dev, &hDevice, &st) != 0) {
        printf("FAIL device status=%#x (%s)\n", st, nvstatus(st)); return -1;
    }
    uint8_t sub[4]; memset(sub,0,sizeof sub);
    if (rm_alloc(hClient, hDevice, 0xc1d00003, NV20_SUBDEVICE_0, sub, sizeof sub, &hSubdev, &st) != 0) {
        printf("FAIL subdevice status=%#x (%s)\n", st, nvstatus(st)); return -1;
    }
    printf("bringup OK: client=%#x device=%#x subdev=%#x\n", hClient, hDevice, hSubdev);
    return 0;
}

static uint32_t reserve(uint64_t bytes, int contig, uint32_t *st) {
    uint8_t p[128]; memset(p,0,sizeof p);
    *(uint32_t*)(p+0) = hClient;                                   /* owner */
    *(uint32_t*)(p+24) = contig ? ATTR_CONTIG_VIDMEM : ATTR_NONCONTIG_VIDMEM;
    *(uint64_t*)(p+64) = bytes;                                    /* size  */
    *(uint64_t*)(p+72) = 4096;                                     /* align */
    uint32_t h = 0;
    static uint32_t next = 0xc1d01000;
    if (rm_alloc(hClient, hDevice, next++, NV01_MEMORY_LOCAL_USER, p, sizeof p, &h, st) != 0)
        return 0;
    return h;
}

#define MAXV 20000
struct view { int fd; void *va; uint64_t len; uint64_t cookie; uint32_t hmem; };
static struct view V[MAXV];
static int nv_ = 0;

/* map one slice; returns 0 ok, -1 RM refused, -2 mmap refused */
static int map_slice(uint32_t hMem, uint64_t off, uint64_t len,
                     uint32_t *st, int *er, const char **stage) {
    int fd = open_gpu_node();
    if (fd < 0) { *stage="open(/dev/nvidiaN)"; *er=errno; *st=0; return -3; }
    uint64_t cookie = 0;
    int rc = rm_map_memory(hMem, off, len, 0, fd, st, er, &cookie);
    if (rc != 0) { *stage="NV_ESC_RM_MAP_MEMORY"; close(fd); return -1; }
    void *va = mmap(NULL, (size_t)len, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);
    if (va == MAP_FAILED) { *stage="mmap"; *er=errno; close(fd); return -2; }
    V[nv_].fd = fd; V[nv_].va = va; V[nv_].len = len;
    V[nv_].cookie = cookie; V[nv_].hmem = hMem; nv_++;
    *stage="ok";
    return 0;
}

static int use_rm_unmap = 1;      /* 0 => munmap+close only (the w722 false positive) */
static int unmap_fail = 0; static uint32_t unmap_last_st = 0; static int unmap_last_er = 0;

static void drop_view(int i) {
    if (V[i].va) munmap(V[i].va, (size_t)V[i].len);
    if (use_rm_unmap && V[i].cookie) {
        uint32_t st2 = 0; int er2 = 0;
        if (rm_unmap_memory(V[i].hmem, V[i].cookie, &st2, &er2) != 0) {
            unmap_fail++; unmap_last_st = st2; unmap_last_er = er2;
        }
    }
    if (V[i].fd >= 0) close(V[i].fd);
    V[i].va = NULL; V[i].fd = -1; V[i].len = 0; V[i].cookie = 0;
}

static double MiB(uint64_t b) { return (double)b / (1024.0*1024.0); }

int main(int argc, char **argv) {
    uint64_t slice_mb   = argc > 1 ? strtoull(argv[1], NULL, 0) : 32;
    uint64_t reserve_mb = argc > 2 ? strtoull(argv[2], NULL, 0) : 8192;
    const char *mode    = argc > 3 ? argv[3] : "ceiling";

    if (getenv("NO_RM_UNMAP")) use_rm_unmap = 0;
    struct rlimit rl = { .rlim_cur = 65536, .rlim_max = 65536 };
    setrlimit(RLIMIT_NOFILE, &rl);
    struct rlimit got; getrlimit(RLIMIT_NOFILE, &got);

    printf("=== bar1probe slice=%llu MiB reserve=%llu MiB mode=%s nofile=%llu ===\n",
           (unsigned long long)slice_mb, (unsigned long long)reserve_mb, mode,
           (unsigned long long)got.rlim_cur);
    printf("release method: %s\n", use_rm_unmap ? "munmap + NV_ESC_RM_UNMAP_MEMORY + close"
                                                : "munmap + close ONLY (no RM unmap)");
    if (bringup() != 0) return 2;
    smi_bar1("before-reserve");

    uint32_t st = 0;
    uint64_t rbytes = reserve_mb << 20;
    uint32_t hMem = reserve(rbytes, 0, &st);
    if (!hMem) {
        printf("RESERVE_REFUSED %llu MiB status=%#x (%s)\n",
               (unsigned long long)reserve_mb, st, nvstatus(st));
        return 3;
    }
    printf("RESERVE_OK %llu MiB  handle=%#x\n", (unsigned long long)reserve_mb, hMem);
    smi_bar1("after-reserve");

    uint64_t slice = slice_mb << 20;

    if (!strcmp(mode, "churn")) {
        /* ★ THE STRICT Q3. The earlier remap test re-used the SAME object offsets it had
         * just released, which RM could satisfy from a cached per-(object,offset) mapping
         * without ever recycling aperture. This maps offsets that have NEVER been mapped,
         * so a success cannot be a cache hit. Repeated rounds also expose a leak: if
         * release is lazy, round N+1 gets less than round N. */
        int rounds = (argc > 4 ? atoi(argv[4]) : 5);
        uint64_t off2 = 0;
        int first = -1;
        for (int r = 0; r < rounds; r++) {
            int got = 0;
            int er3 = 0; const char *stg3 = "";
            while (nv_ < MAXV && off2 + slice <= rbytes) {
                if (map_slice(hMem, off2, slice, &st, &er3, &stg3) != 0) break;
                off2 += slice; got++;
            }
            printf("CHURN round %d: mapped %d views (%.0f MiB) at FRESH offsets "
                   "[%.0f..%.0f MiB of the object]  stop=%s status=%#x (%s)\n",
                   r, got, MiB((uint64_t)got*slice),
                   MiB(off2 - (uint64_t)got*slice), MiB(off2), stg3, st, nvstatus(st));
            fflush(stdout);
            if (first < 0) first = got;
            for (int i = 0; i < nv_; i++) drop_view(i);
            nv_ = 0;
        }
        printf("CHURN_VERDICT first_round=%d unmap_failures=%d (last status=%#x (%s) errno=%d)"
               "  (a later round smaller than the first => release is lazy or leaks)\n",
               first, unmap_fail, unmap_last_st, nvstatus(unmap_last_st), unmap_last_er);
        smi_bar1("after-churn");
        return 0;
    }

    if (!strcmp(mode, "bisect")) {
        /* largest SINGLE mapping that succeeds — the bookkeeping control for the
         * accumulate ceiling. If one N-MiB map succeeds where N 1-MiB maps failed,
         * the wall is our per-view overhead, not the aperture. */
        uint64_t lo = 0, hi = (argc > 4 ? strtoull(argv[4], NULL, 0) : 512) << 20;
        uint64_t grain = 1 << 20;
        int er2 = 0; const char *stg = "";
        while (hi - lo > grain) {
            uint64_t mid = lo + (hi - lo) / 2;
            mid &= ~(grain - 1);
            if (mid == lo) break;
            int r = map_slice(hMem, 0, mid, &st, &er2, &stg);
            if (r == 0) { lo = mid; drop_view(nv_-1); nv_--; }
            else hi = mid;
        }
        printf("LARGEST_SINGLE_MAP=%.0f MiB (last refusal status=%#x (%s) at %s)\n",
               MiB(lo), st, nvstatus(st), stg);
        smi_bar1("after-bisect");
        return 0;
    }

    if (!strcmp(mode, "single")) {
        /* ---- the bookkeeping control: ONE mapping of the whole accumulated total ---- */
        uint64_t want = (argc > 4 ? strtoull(argv[4], NULL, 0) : 256) << 20;
        int er = 0; const char *stage = "";
        int rc = map_slice(hMem, 0, want, &st, &er, &stage);
        printf("SINGLE_MAP %.0f MiB -> %s status=%#x (%s) errno=%d (%s)\n",
               MiB(want), rc==0?"OK":"REFUSED", st, nvstatus(st), er,
               rc==0?"-":stage);
        smi_bar1("after-single");
        return rc == 0 ? 0 : 1;
    }

    /* ---- Q2: accumulate slices until refusal ---- */
    uint64_t off = 0, total = 0;
    int er = 0; const char *stage = "";
    int rc = 0;
    while (nv_ < MAXV && off + slice <= rbytes) {
        rc = map_slice(hMem, off, slice, &st, &er, &stage);
        if (rc != 0) break;
        off += slice; total += slice;
        /* prove the mapping is LIVE, not just accepted */
        volatile uint32_t *w = (volatile uint32_t *)V[nv_-1].va;
        w[0] = 0xA5A50000u | (uint32_t)nv_;
        if (w[0] != (0xA5A50000u | (uint32_t)nv_)) {
            printf("READBACK_MISMATCH at view %d: %#x\n", nv_, w[0]);
        }
        if ((nv_ % 16) == 0) { printf("  ... %d views, %.0f MiB mapped\n", nv_, MiB(total)); fflush(stdout); }
    }
    printf("CEILING views=%d total_mapped=%.1f MiB slice=%llu MiB stopped_at=%s "
           "status=%#x (%s) errno=%d (%s)\n",
           nv_, MiB(total), (unsigned long long)slice_mb,
           rc==0?"(ran out of object/room)":stage, st, nvstatus(st), er, strerror(er));
    smi_bar1("at-ceiling");

    if (!strcmp(mode, "hold")) {
        printf("HOLDING for 60s (pid %d) — a second process can now compete\n", getpid());
        fflush(stdout);
        sleep(60);
        printf("HOLD_DONE\n");
        return 0;
    }

    /* ---- Q3: does unmapping return aperture? ---- */
    if (nv_ > 0) {
        int half = nv_ / 2;
        if (half < 1) half = 1;
        for (int i = 0; i < half; i++) drop_view(i);
        printf("RELEASED %d of %d views (%.0f MiB) via munmap+close\n", half, nv_, MiB((uint64_t)half*slice));
        smi_bar1("after-release");

        int regained = 0;
        uint64_t roff = 0;
        for (int i = 0; i < half && roff + slice <= rbytes; i++) {
            /* reuse the low offsets we just freed */
            int save = nv_;
            if (nv_ >= MAXV) break;
            rc = map_slice(hMem, roff, slice, &st, &er, &stage);
            roff += slice;
            if (rc != 0) { nv_ = save; break; }
            regained++;
        }
        printf("REMAP_AFTER_RELEASE regained=%d of %d released  %s "
               "(last status=%#x (%s) errno=%d)\n",
               regained, half,
               regained >= half ? "=> APERTURE IS FULLY RECLAIMED"
                                : (regained > 0 ? "=> PARTIALLY reclaimed" : "=> NOT reclaimed"),
               st, nvstatus(st), er);
        smi_bar1("after-remap");
    }

    for (int i = 0; i < nv_; i++) drop_view(i);
    smi_bar1("after-drop-all");
    printf("PROBE_EXIT=0\n");
    return 0;
}
