// engsw_shim.c — BENCH-ONLY guest LD_PRELOAD shim for the x11-dispsw classID probe
// (docs/design/V3_DISPLAY.md, the x11-dispsw "review fixes" block; hook.sh step 4c,
// DISPLAY_X11_ENGSW=1). Built and used inside the bench guest only; never shipped, and the
// built .so never leaves the guest.
//
// What it does: before every GF100_DISP_SW (0x9072) alloc the process makes, it allocates a
// GF100_TIMED_SEMAPHORE_SW (0x9074) under the SAME channel. The guest's CPU-RM gives that object
// the channel's next FIFO software classID (kchannelRegisterChild, ogkm-580
// kernel_channel.c:3408-3453) before it RPCs the alloc; kayfabe's boundary refuses the class, so the
// guest's numbering moves one ahead of anything a host twin numbered — the review's case (b). The
// display-SW object that follows must then carry the guest's number on its twin, or the guest's
// SET_OBJECT names a number the twin lacks.
//
// ENGSW_MODE=badhead instead allocates, before each display-SW object, a GF100_DISP_SW with
// logicalHeadId 0x7f: the guest's CPU-RM numbers it, asks for the active displays, then refuses the
// head (disp_sw.c:83-88) and never sends the alloc — the review's case (a), the one slip no
// physical RM can see (kf-rm counts it: `query was not followed by its alloc`).
//
// Output: one stderr line per alloc, `ENGSW_SHIM ...`, with both statuses.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <linux/ioctl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define NV_IOCTL_MAGIC 'F'
#define NV_ESC_RM_ALLOC 0x2b
#define GF100_DISP_SW 0x9072u
#define GF100_TIMED_SEMAPHORE_SW 0x9074u

static int (*real_ioctl)(int, unsigned long, ...);
static uint32_t next_handle = 0xc0de9000u;

static uint32_t rd32(const uint8_t *p, unsigned at) {
    uint32_t v;
    memcpy(&v, p + at, 4);
    return v;
}

static void wr32(uint8_t *p, unsigned at, uint32_t v) { memcpy(p + at, &v, 4); }

int ioctl(int fd, unsigned long req, ...) {
    va_list ap;
    va_start(ap, req);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    if (!real_ioctl)
        real_ioctl = (int (*)(int, unsigned long, ...))dlsym(RTLD_NEXT, "ioctl");
    unsigned sz = _IOC_SIZE(req);
    // NVOS21_PARAMETERS (32 bytes) or NVOS64_PARAMETERS (48 bytes), ogkm-580 nvos.h:
    // hRoot@0 hObjectParent@4 hObjectNew@8 hClass@12 pAllocParms@16; NVOS21 paramsSize@24
    // status@28; NVOS64 pRightsRequested@24 paramsSize@32 flags@36 status@40.
    if (_IOC_TYPE(req) != NV_IOCTL_MAGIC || _IOC_NR(req) != NV_ESC_RM_ALLOC || !arg ||
        (sz != 32 && sz != 48) || rd32(arg, 12) != GF100_DISP_SW)
        return real_ioctl(fd, req, arg);
    uint8_t *p = arg;
    unsigned size_at = sz == 32 ? 24 : 32, status_at = sz == 32 ? 28 : 40;
    const char *mode = getenv("ENGSW_MODE");
    int badhead = mode && !strcmp(mode, "badhead");
    uint8_t pre[48];
    uint32_t head[3] = {0x7f, 0, 0}; // NV9072_ALLOCATION_PARAMETERS {logicalHeadId, displayMask, caps}
    memcpy(pre, p, sz);
    uint32_t h = __atomic_fetch_add(&next_handle, 1, __ATOMIC_RELAXED);
    wr32(pre, 8, h);
    if (badhead) {
        uint64_t at = (uint64_t)(uintptr_t)head;
        memcpy(pre + 16, &at, 8);
        wr32(pre, size_at, sizeof head);
    } else {
        wr32(pre, 12, GF100_TIMED_SEMAPHORE_SW);
        memset(pre + 16, 0, 8);
        wr32(pre, size_at, 0);
    }
    if (sz == 48) {
        memset(pre + 24, 0, 8);
        wr32(pre, 36, 0);
    }
    wr32(pre, status_at, 0);
    int r1 = real_ioctl(fd, req, pre);
    fprintf(stderr, "ENGSW_SHIM pre %s under %#x handle %#x: ioctl=%d status=%#x\n",
            badhead ? "0x9072-badhead" : "0x9074", rd32(p, 4), h, r1, rd32(pre, status_at));
    int r2 = real_ioctl(fd, req, arg);
    fprintf(stderr, "ENGSW_SHIM 0x9072 under %#x handle %#x: ioctl=%d status=%#x\n", rd32(p, 4),
            rd32(p, 8), r2, rd32(p, status_at));
    return r2;
}
