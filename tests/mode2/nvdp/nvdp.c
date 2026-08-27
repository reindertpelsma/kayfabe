/*
 * nvdp.c -- NATIVE DATA-PLANE reference capture for the cup2 workload.
 *
 * WHY THIS EXISTS
 * ---------------
 * Every oracle this project owns is CONTROL PLANE: the 56-row RM control table,
 * the nvdiff ioctl differential, the ogkm-compiled parsers. The C artifact's
 * green runs were CPU copies with EMULATOR-WRITTEN completions (CLAUDE.md,
 * "SCOPE THE ORACLE", 2026-08-12). Nobody has ever recorded what a native,
 * unvirtualised cup2 actually does on the ring / pushbuffer / semaphore.
 *
 * THE INSIGHT THAT MAKES IT CHEAP
 * -------------------------------
 * Run natively and the ring, the pushbuffer, USERD and the semaphore are all in
 * THIS PROCESS'S OWN ADDRESS SPACE. No emulator, no BAR window, no page-table
 * descent. A plain userspace program can read them.
 *
 * WHAT IT MEASURES
 * ----------------
 *   1. the ring      -- GPU VA (gpFifoOffset), entry count, live GPFIFO entries
 *   2. the pushbuffer-- the decoded method stream each entry points at
 *   3. the semaphore -- exact VA, page offset, APERTURE, declaring channel/engine
 *   4. WHO WRITES IT -- see the note below; POLL, do not watch.
 *   5. GP_GET vs GP_PUT -- sampled over time by a poller thread, each sample
 *                       carrying its OWN CLOCK_MONOTONIC timestamp (so a
 *                       buffered dump at teardown still reports time correctly).
 *
 * ///// AUTHORSHIP: WHY A WATCHPOINT CANNOT ANSWER IT /////
 * A GPU semaphore release is a DMA write. It does not go through the CPU MMU,
 * takes no page fault, and x86 debug registers watch CPU accesses ONLY -- so a
 * watchpoint CANNOT see it. Its silence is the expected behaviour of ANY DMA and
 * therefore proves nothing on its own.
 *   => the watchpoint here is a NEGATIVE CONTROL. If it FIRES, libcuda stored the
 *      value with a CPU instruction and GPU authorship is REFUTED. Silence is
 *      necessary, not sufficient.
 * Primary evidence, strongest first:
 *   (a) THE GPU TIMESTAMP INSIDE THE REPORT. A 4-word release is
 *       [payload, 0, ts_lo, ts_hi] and the timestamp comes from the GPU's own
 *       clock -- nothing CPU-side has access to it. We record the RAW WORDS.
 *   (b) THE PAYLOAD MATCHING WHAT THE PUSHBUFFER DECLARED. We decode the
 *       SET_SEMAPHORE_PAYLOAD / SET_REPORT_SEMAPHORE operand out of the method
 *       stream and compare it against the landed value; agreement ties the write
 *       to THAT submission.
 *   (c) ORDERING AGAINST GP_GET. The release should follow the cursor advancing
 *       past the entry that carried it. The semaphore words and GP_GET are read
 *       in the SAME loop iteration, so their order is measured, not inferred.
 *
 * ///// APERTURE MATTERS /////
 * sysmem: the DMA write is snooped and an ordinary load sees it -- but the read
 *   MUST be volatile or the compiler hoists it and you sample a cached register
 *   and conclude nothing changed.
 * vidmem/BAR: CPU reads cross PCIe, uncached/WC semantics apply, and a stale or
 *   reordered read is a real hazard.
 * Which one it actually is, is itself one of the five things we want, so we
 * classify the semaphore's mapping (anon/sysmem vs a /dev/nvidia* BAR window)
 * and print the evidence rather than assuming.
 *
 * HOW IT FINDS THEM
 * -----------------
 * ioctl()/mmap() are interposed *by the executable itself* (executable symbols
 * win over libc for libcuda's PLT calls -- no LD_PRELOAD needed). We decode:
 *   NV_ESC_RM_ALLOC (0x2B) with a channel class -> NV_CHANNEL_ALLOC_PARAMS
 *       => gpFifoOffset, gpFifoEntries, engineType, userdMem{base,addressSpace}
 *   NV_ESC_RM_MAP_MEMORY (0x4E) -> NVOS33 => hMemory, offset, length, pLinearAddress
 *   mmap on /dev/nvidia* -> the CPU windows
 * and then read the ring straight out of our own VA space.
 *
 * Build:  cc -O2 -g -o nvdp nvdp.c -ldl -lpthread
 * Run:    ./nvdp            (needs a real GPU + driver; NO CUDA toolkit needed --
 *                            libcuda.so.1 is dlopen'd, cuda.h is not required)
 * Env:    NVDP_OUT=<path>   text log (default ./nvdp.log)
 *         NVDP_RAW=<dir>    directory for raw binary dumps (default ./nvdp_raw)
 *         NVDP_SCANCAP=<MB> cap on bytes scanned (default 512)
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <stdint.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <dlfcn.h>
#include <dirent.h>
#include <pthread.h>
#include <time.h>
#include <sys/uio.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <linux/perf_event.h>
#include <linux/hw_breakpoint.h>
#include <signal.h>
#include <setjmp.h>

/* ------------------------------------------------------------------ output */

static FILE *g_log;
static char  g_rawdir[256] = "./nvdp_raw";
static double g_t0;

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

/* Every line carries the time it was EMITTED. A recorder that buffers and dumps
 * at teardown reports order correctly and time not at all -- measured here. */
static void L(const char *fmt, ...)
{
    va_list ap;
    if (!g_log) g_log = stdout;
    fprintf(g_log, "[%9.6f] ", now_s() - g_t0);
    va_start(ap, fmt); vfprintf(g_log, fmt, ap); va_end(ap);
    fputc('\n', g_log);
    fflush(g_log);
}

/* ------------------------------------------------------- safe self-reading */

static pid_t g_pid;

/*
 * ! process_vm_readv CANNOT read a VM_PFNMAP/VM_IO mapping.
 * Measured 2026-08-12: it goes through get_user_pages_remote, which refuses
 * device mappings, so EVERY /dev/nvidia0 BAR window -- which is where the ring
 * and USERD live -- read back as "unreadable" and the first two runs concluded
 * the ring was not in the address space. It was; the instrument could not see
 * it. The CPU itself can read those pages perfectly well, so for anything the
 * safe path refuses we fall back to a DIRECT volatile load fenced by a
 * SIGSEGV/SIGBUS handler.  (jmp buf is per-thread: the poller reads too.)
 */
static __thread sigjmp_buf g_jb;
static __thread volatile int g_faulting;
static __thread volatile size_t g_dread_done;

static void fault_h(int sig)
{
    (void)sig;
    if (g_faulting) siglongjmp(g_jb, 1);
    _exit(139);
}

static void install_fault_handlers(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = fault_h;
    sa.sa_flags = SA_NODEFER;
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL);
}

/* Direct load, fenced. Returns bytes successfully copied. */
static size_t dread(uint64_t a, void *dst, size_t len)
{
    g_dread_done = 0;
    g_faulting = 1;
    if (sigsetjmp(g_jb, 1) == 0) {
        volatile const uint32_t *sp = (volatile const uint32_t *)(uintptr_t)a;
        uint32_t *d = (uint32_t *)dst;
        size_t n = len / 4, i;
        for (i = 0; i < n; i++) { d[i] = sp[i]; g_dread_done = (i + 1) * 4; }
    }
    g_faulting = 0;
    return g_dread_done;
}

static int in_nvidia_window(uint64_t a);

static size_t sread(const void *a, void *dst, size_t len)
{
    struct iovec l = { dst, len }, r = { (void *)a, len };
    ssize_t n;
    if (!a || !len) return 0;
    n = process_vm_readv(g_pid, &l, 1, &r, 1, 0);
    if (n == (ssize_t)len) return (size_t)len;
    if (in_nvidia_window((uint64_t)(uintptr_t)a)) return dread((uint64_t)(uintptr_t)a, dst, len);
    return n < 0 ? 0 : (size_t)n;
}
static int rd32(uint64_t a, uint32_t *v)  { return sread((void *)a, v, 4) == 4; }
static int rd64(uint64_t a, uint64_t *v)  { return sread((void *)a, v, 8) == 8; }
static int readable(uint64_t a)           { uint32_t t; return rd32(a, &t); }

/* --------------------------------------------------------- /proc/self/maps */

#define MAXMAP 4096
struct maprec { uint64_t lo, hi; char perm[8]; char path[160]; };
static struct maprec g_map[MAXMAP];
static int g_nmap;

static void maps_reload(void)
{
    FILE *f = fopen("/proc/self/maps", "r");
    char line[512];
    g_nmap = 0;
    if (!f) return;
    while (fgets(line, sizeof line, f) && g_nmap < MAXMAP) {
        struct maprec *m = &g_map[g_nmap];
        unsigned long long lo, hi;
        char perm[8], path[300];
        int n;
        path[0] = 0;
        n = sscanf(line, "%llx-%llx %7s %*s %*s %*s %299[^\n]", &lo, &hi, perm, path);
        if (n < 3) continue;
        m->lo = lo; m->hi = hi;
        snprintf(m->perm, sizeof m->perm, "%s", perm);
        snprintf(m->path, sizeof m->path, "%s", path[0] ? path : "");
        g_nmap++;
    }
    fclose(f);
}

static const struct maprec *map_of(uint64_t a)
{
    int i;
    for (i = 0; i < g_nmap; i++)
        if (a >= g_map[i].lo && a < g_map[i].hi) return &g_map[i];
    return NULL;
}

static const char *map_desc(uint64_t a, char *buf, size_t n)
{
    const struct maprec *m = map_of(a);
    if (!m) { snprintf(buf, n, "<UNMAPPED>"); return buf; }
    snprintf(buf, n, "%016llx-%016llx %s %s +0x%llx",
             (unsigned long long)m->lo, (unsigned long long)m->hi, m->perm,
             m->path[0] ? m->path : "[anon]", (unsigned long long)(a - m->lo));
    return buf;
}

/* --------------------------------------------------------------- pagemap */
/* Only scan pages that are PRESENT, so a scan never faults in a reservation.  */

static int g_pagemap = -1;
static long g_pgsz;

/* ! A /dev/nvidia* mapping is VM_PFNMAP: pagemap reports no PFN and the page
 * reads as "not present", so a presence-gated scan SKIPS the ring and the
 * USERD entirely -- measured 2026-08-12, it is why the first ring hunt found
 * nothing while the sysmem pushbuffer was found immediately. Device mappings
 * are always scanned; process_vm_readv cannot fault us, so the cost of being
 * wrong is a short read. */
static int scan_ok(const struct maprec *m, uint64_t va);

static int page_present(uint64_t va)
{
    uint64_t e;
    off_t off = (off_t)(va / (uint64_t)g_pgsz) * 8;
    if (g_pagemap < 0) return 1;
    if (pread(g_pagemap, &e, 8, off) != 8) return 0;
    return (e >> 63) & 1;
}

static int scan_ok(const struct maprec *m, uint64_t va)
{
    if (m && strncmp(m->path, "/dev/nvidia", 11) == 0) return 1;
    return page_present(va);
}

/* ------------------------------------------- interposed ioctl / mmap records */

#define NV_IOCTL_MAGIC 'F'
#define NV_ESC_RM_ALLOC_MEMORY 0x27
#define NV_ESC_RM_ALLOC        0x2B
#define NV_ESC_RM_MAP_MEMORY   0x4E
#define NV_ESC_RM_MAP_MEMORY_DMA 0x57

/* Channel classes we care about (Kepler..Hopper GPFIFO). GA106 uses 0xC56F. */
static int is_channel_class(uint32_t c)
{
    switch (c) {
    case 0xA06F: case 0xB06F: case 0xC06F: case 0xC36F:
    case 0xC46F: case 0xC56F: case 0xC66F: case 0xC76F: case 0xC86F:
        return 1;
    default: return 0;
    }
}

#define MAXCH 64
struct chan {
    double   t;
    uint32_t hClass, hObjectNew, hObjectParent;
    uint64_t gpFifoOffset;
    uint32_t gpFifoEntries, flags, engineType, cid;
    uint32_t hUserdMemory0;
    uint64_t userdOffset0;
    uint64_t instBase, userdBase, ramfcBase;
    uint64_t ring_cpu, userd_cpu;
    uint32_t instAS,  userdAS,  ramfcAS;
    int      status;
};
static struct chan g_ch[MAXCH];
static int g_nch;

struct chdiag { uint32_t hClass, iocsize, psz, got; uint64_t pptr; uint8_t head[64]; };
static struct chdiag g_chdiag[MAXCH];
static int g_nchdiag;

#define MAXMAPMEM 512
struct mapmem { double t; uint32_t hMemory; uint64_t offset, length, lin; uint32_t status, flags; };
static struct mapmem g_mm[MAXMAPMEM];
static int g_nmm;

#define MAXMMAP 512
struct mmaprec { double t; char dev[24]; uint64_t off, len, ret; int prot; };
static struct mmaprec g_mp[MAXMMAP];
static int g_nmp;

#define MAXOBJ 4096
struct objrec { uint32_t h; uint32_t cls; };
static struct objrec g_obj[MAXOBJ];
static int g_nobj;

static int in_nvidia_window(uint64_t a)
{
    int i;
    for (i = 0; i < g_nmp; i++)
        if (a >= g_mp[i].ret && a < g_mp[i].ret + g_mp[i].len) return 1;
    return 0;
}

static int (*r_ioctl)(int, unsigned long, void *);
static void *(*r_mmap)(void *, size_t, int, int, int, off_t);

#define FDCACHE 4096
static char g_fdname[FDCACHE][24];
static char g_fdknown[FDCACHE];

static int fd_is_nvidia(int fd, const char **name)
{
    char link[64], path[256];
    ssize_t n;
    if (fd < 0) return 0;
    if (fd < FDCACHE && g_fdknown[fd]) { *name = g_fdname[fd]; return g_fdknown[fd] == 1; }
    snprintf(link, sizeof link, "/proc/self/fd/%d", fd);
    n = readlink(link, path, sizeof path - 1);
    if (n < 0) return 0;
    path[n] = 0;
    if (strncmp(path, "/dev/nvidia", 11) != 0) { if (fd < FDCACHE) g_fdknown[fd] = 2; return 0; }
    if (fd < FDCACHE) {
        snprintf(g_fdname[fd], sizeof g_fdname[fd], "%s", path + 5);
        g_fdknown[fd] = 1; *name = g_fdname[fd];
    } else { static __thread char t[24]; snprintf(t, sizeof t, "%s", path + 5); *name = t; }
    return 1;
}

int close(int fd)
{
    static int (*r_close)(int);
    if (!r_close) r_close = dlsym(RTLD_NEXT, "close");
    if (fd >= 0 && fd < FDCACHE) g_fdknown[fd] = 0;
    return r_close(fd);
}

/* NV_MEMORY_DESC_PARAMS { NvU64 base; NvU64 size; NvU32 addressSpace; NvU32 cacheAttrib; } */
static void memdesc(const uint8_t *p, uint64_t *base, uint32_t *as)
{
    memcpy(base, p + 0, 8);
    memcpy(as,   p + 16, 4);
}

/*
 * ! The RM_ALLOC header must be sampled BEFORE the call.
 * Measured 2026-08-12: read AFTER the ioctl, paramsSize comes back as 0 on all
 * 16 channel allocs -- RM clobbers the header -- so a post-call read reports
 * "no parameters" for a call that plainly had them. The pointer and the size
 * come from the PRE image; the handle comes from the POST image because
 * hObjectNew is [OUT].
 */
static void note_alloc(void *arg, size_t hlen, const uint8_t *pre, size_t pregot)
{
    uint8_t h[64];
    uint32_t hClass, psz = 0;
    uint64_t pptr = 0;
    uint8_t pb[512];
    size_t got;

    if (sread(arg, h, hlen < sizeof h ? hlen : sizeof h) < 16) return;
    memcpy(&hClass, h + 12, 4);
    if (pregot >= 16) memcpy(&hClass, pre + 12, 4);

    if (g_nobj < MAXOBJ) { uint32_t hn; memcpy(&hn, h + 8, 4);
                           g_obj[g_nobj].h = hn; g_obj[g_nobj].cls = hClass; g_nobj++; }

    if (!is_channel_class(hClass)) return;
    {
        const uint8_t *src = (pregot >= hlen) ? pre : h;
        if (hlen >= 48) { memcpy(&pptr, src + 16, 8); memcpy(&psz, src + 32, 4); }
        else            { memcpy(&pptr, src + 16, 8); memcpy(&psz, src + 24, 4); }
    }
    if (!psz || psz > 4096) psz = 512;   /* fall back to a full struct read */
    got = sread((void *)pptr, pb, psz > sizeof pb ? sizeof pb : psz);
    if (g_nch >= MAXCH) return;
    /* Record WHY a channel alloc was not decoded. "0 channels" must be
     * distinguishable from "the recogniser dropped it". */
    g_chdiag[g_nchdiag].hClass = hClass;
    g_chdiag[g_nchdiag].iocsize = hlen;
    g_chdiag[g_nchdiag].pptr = pptr;
    g_chdiag[g_nchdiag].psz = psz;
    g_chdiag[g_nchdiag].got = (uint32_t)got;
    memcpy(g_chdiag[g_nchdiag].head, pb, got < 64 ? got : 64);
    if (g_nchdiag < MAXCH - 1) g_nchdiag++;
    if (got < 200) return;

    {
        struct chan *c = &g_ch[g_nch++];
        memset(c, 0, sizeof *c);
        c->t = now_s() - g_t0;
        c->hClass = hClass;
        memcpy(&c->hObjectParent, h + 4, 4);
        memcpy(&c->hObjectNew,    h + 8, 4);
        memcpy(&c->gpFifoOffset,  pb + 8,  8);
        memcpy(&c->gpFifoEntries, pb + 16, 4);
        memcpy(&c->flags,         pb + 20, 4);
        memcpy(&c->hUserdMemory0, pb + 32, 4);
        memcpy(&c->userdOffset0,  pb + 64, 8);
        memcpy(&c->engineType,    pb + 128, 4);
        memcpy(&c->cid,           pb + 132, 4);
        memdesc(pb + 144, &c->instBase,  &c->instAS);
        memdesc(pb + 168, &c->userdBase, &c->userdAS);
        memdesc(pb + 192, &c->ramfcBase, &c->ramfcAS);
    }
}

int ioctl(int fd, unsigned long req, ...)
{
    va_list ap; void *arg; const char *dev = NULL;
    unsigned nr, ty, iocsize; int rc, se;
    uint8_t pre_hdr[64]; size_t pre_got = 0;

    va_start(ap, req); arg = va_arg(ap, void *); va_end(ap);
    if (!r_ioctl) r_ioctl = dlsym(RTLD_NEXT, "ioctl");
    if (!fd_is_nvidia(fd, &dev)) return r_ioctl(fd, req, arg);

    ty = (unsigned)((req >> 8) & 0xff);
    nr = (unsigned)(req & 0xff);
    iocsize = (unsigned)_IOC_SIZE(req);

    if (ty == NV_IOCTL_MAGIC && nr == NV_ESC_RM_ALLOC && strncmp(dev, "nvidia-uvm", 10) != 0)
        pre_got = sread(arg, pre_hdr, iocsize < sizeof pre_hdr ? iocsize : sizeof pre_hdr);

    rc = r_ioctl(fd, req, arg); se = errno;

    if (ty == NV_IOCTL_MAGIC && strncmp(dev, "nvidia-uvm", 10) != 0) {
        if (nr == NV_ESC_RM_ALLOC) {
            note_alloc(arg, iocsize, pre_hdr, pre_got);
        } else if (nr == NV_ESC_RM_MAP_MEMORY && iocsize >= 48 && g_nmm < MAXMAPMEM) {
            uint8_t p[64];
            if (sread(arg, p, 48) >= 48) {
                struct mapmem *m = &g_mm[g_nmm++];
                m->t = now_s() - g_t0;
                memcpy(&m->hMemory, p + 8, 4);
                memcpy(&m->offset,  p + 16, 8);
                memcpy(&m->length,  p + 24, 8);
                memcpy(&m->lin,     p + 32, 8);
                memcpy(&m->status,  p + 40, 4);
                memcpy(&m->flags,   p + 44, 4);
            }
        }
    }
    errno = se;
    return rc;
}

void *mmap(void *addr, size_t len, int prot, int flags, int fd, off_t off)
{
    const char *dev = NULL; void *r;
    if (!r_mmap) r_mmap = dlsym(RTLD_NEXT, "mmap");
    r = r_mmap(addr, len, prot, flags, fd, off);
    if (fd_is_nvidia(fd, &dev) && g_nmp < MAXMMAP) {
        struct mmaprec *m = &g_mp[g_nmp++];
        m->t = now_s() - g_t0;
        snprintf(m->dev, sizeof m->dev, "%s", dev);
        m->off = (uint64_t)off; m->len = len; m->ret = (uint64_t)(uintptr_t)r; m->prot = prot;
    }
    return r;
}
void *mmap64(void *a, size_t l, int p, int f, int fd, off_t o)
    __attribute__((alias("mmap")));

/* ----------------------------------------------------- GPFIFO / method decode */
/* NVC36F_GP_ENTRY0_GET 31:2 ; ENTRY1_GET_HI 7:0 ; PRIV 8 ; LEVEL 9 ;
 * LENGTH 30:10 (in dwords) ; SYNC 31.                     (ogkm clc36f.h:263) */
static uint64_t gpe_addr(uint32_t e0, uint32_t e1) { return ((uint64_t)(e1 & 0xff) << 32) | (e0 & 0xfffffffcu); }
static uint32_t gpe_len(uint32_t e1)               { return (e1 >> 10) & 0x1fffff; }

/* USERD: NV_RAMUSERD_GP_GET = dword 34 (0x88), GP_PUT = dword 35 (0x8c).
 * (ogkm src/common/inc/swref/published/ampere/ga100/dev_ram.h:37) */
#define USERD_GP_GET 0x88
#define USERD_GP_PUT 0x8c

static const char *ce_method(uint32_t m)
{
    switch (m) {
    case 0x0240: return "NVC7B5_SET_SEMAPHORE_A(upper)";
    case 0x0244: return "NVC7B5_SET_SEMAPHORE_B(lower)";
    case 0x0248: return "NVC7B5_SET_SEMAPHORE_PAYLOAD";
    case 0x024C: return "NVC7B5_SET_SEMAPHORE_PAYLOAD_UPPER";
    case 0x0300: return "NVC7B5_LAUNCH_DMA";
    case 0x0400: return "NVC7B5_OFFSET_IN_UPPER";
    case 0x0404: return "NVC7B5_OFFSET_IN_LOWER";
    case 0x0408: return "NVC7B5_OFFSET_OUT_UPPER";
    case 0x040C: return "NVC7B5_OFFSET_OUT_LOWER";
    case 0x0410: return "NVC7B5_PITCH_IN";
    case 0x0414: return "NVC7B5_PITCH_OUT";
    case 0x0418: return "NVC7B5_LINE_LENGTH_IN";
    case 0x041C: return "NVC7B5_LINE_COUNT";
    case 0x0700: return "NVC7B5_LAUNCH_DMA_DATA(inline)";
    default: return NULL;
    }
}
static const char *host_method(uint32_t m)
{
    switch (m) {
    case 0x0000: return "NVC36F_SET_OBJECT";
    case 0x0010: return "NVC36F_ILLEGAL";
    case 0x0050: return "NVC36F_NON_STALL_INTERRUPT";
    case 0x0054: return "NVC36F_FB_FLUSH";
    case 0x005c: return "NVC36F_SEM_ADDR_LO";
    case 0x0060: return "NVC36F_SEM_ADDR_HI";
    case 0x0064: return "NVC36F_SEM_PAYLOAD_LO";
    case 0x0068: return "NVC36F_SEM_PAYLOAD_HI";
    case 0x006c: return "NVC36F_SEM_EXECUTE";
    case 0x0070: return "NVC36F_WFI";
    case 0x0078: return "NVC36F_MEM_OP_A";
    case 0x007c: return "NVC36F_MEM_OP_B";
    case 0x0080: return "NVC36F_MEM_OP_C";
    case 0x0084: return "NVC36F_MEM_OP_D";
    default: return NULL;
    }
}
static const char *gr_method(uint32_t m)
{
    switch (m) {
    case 0x0000: return "NVC7C0_SET_OBJECT";
    case 0x0100: return "NVC7C0_NO_OPERATION";
    case 0x0180: return "NVC7C0_LINE_LENGTH_IN";
    case 0x0184: return "NVC7C0_LINE_COUNT";
    case 0x0188: return "NVC7C0_OFFSET_OUT_UPPER";
    case 0x018c: return "NVC7C0_OFFSET_OUT";
    case 0x01b0: return "NVC7C0_LAUNCH_DMA(I2M)";
    case 0x01b4: return "NVC7C0_LOAD_INLINE_DATA <== PAYLOAD IS A LITERAL";
    case 0x0510: return "NVC7C0_SET_FALCON04";
    case 0x1698: return "NVC7C0_INVALIDATE_SHADER_CACHES_NO_WFI";
    case 0x3400: case 0x3404: case 0x3408: case 0x340c:
                 return "NVC7C0_SET_MME_SHADOW_SCRATCH";
    case 0x0158: return "NVC7C0_SET_REPORT_SEMAPHORE_PAYLOAD_LOWER";
    case 0x015c: return "NVC7C0_SET_REPORT_SEMAPHORE_PAYLOAD_UPPER";
    case 0x0160: return "NVC7C0_SET_REPORT_SEMAPHORE_ADDRESS_LOWER";
    case 0x0164: return "NVC7C0_SET_REPORT_SEMAPHORE_ADDRESS_UPPER";
    case 0x1b00: return "NVC7C0_SET_REPORT_SEMAPHORE_A(upper)";
    case 0x1b04: return "NVC7C0_SET_REPORT_SEMAPHORE_B(lower)";
    case 0x1b08: return "NVC7C0_SET_REPORT_SEMAPHORE_C(payload)";
    case 0x1b0c: return "NVC7C0_SET_REPORT_SEMAPHORE_D(operation)";
    case 0x0af4: return "NVC7C0_LAUNCH_DMA";
    case 0x0ad0: return "NVC7C0_SEND_SIGNALING_PCAS_B";
    case 0x0ad4: return "NVC7C0_SEND_SIGNALING_PCAS2_B";
    case 0x0268: return "NVC7C0_SET_INLINE_QMD_ADDRESS_A";
    case 0x026c: return "NVC7C0_SET_INLINE_QMD_ADDRESS_B";
    /* ★ w274 -- the CONTEXT-INIT segment (gpe[0]) names these and nothing else did.
     * ⚠ Note the field names in clc7c0.h, because they are the whole point:
     *   SET_SHADER_LOCAL_MEMORY_A/B      -> ADDRESS_UPPER / ADDRESS_LOWER   (a POINTER)
     *   SET_SHADER_LOCAL_MEMORY_WINDOW_* -> BASE_ADDRESS_UPPER / BASE_ADDRESS (an APERTURE)
     *   SET_SHADER_SHARED_MEMORY_WINDOW_*-> BASE_ADDRESS_UPPER / BASE_ADDRESS (an APERTURE)
     * and there is NO SET_SHADER_SHARED_MEMORY_A/B at all -- shared memory is on-chip
     * SRAM, so there is nothing in memory for a pointer to point at. */
    case 0x0114: return "NVC7C0_LOAD_MME_INSTRUCTION_RAM_POINTER";
    case 0x0118: return "NVC7C0_LOAD_MME_INSTRUCTION_RAM <== MME MICROCODE";
    case 0x011c: return "NVC7C0_LOAD_MME_START_ADDRESS_RAM_POINTER/RAM";
    case 0x0200: return "NVC7C0_SET_VALID_SPAN_OVERFLOW_AREA_A";
    case 0x0204: return "NVC7C0_SET_VALID_SPAN_OVERFLOW_AREA_B";
    case 0x0208: return "NVC7C0_SET_VALID_SPAN_OVERFLOW_AREA_C(size)";
    case 0x023c: return "NVC7C0_SET_SPA_VERSION/INVALIDATE";
    case 0x0248: return "NVC7C0_(0x0248 -- repeated 64x in context init)";
    case 0x02a0: return "NVC7C0_SET_SHADER_SHARED_MEMORY_WINDOW_A <== APERTURE BASE, NOT A POINTER";
    case 0x02a4: return "NVC7C0_SET_SHADER_SHARED_MEMORY_WINDOW_B <== APERTURE BASE, NOT A POINTER";
    case 0x02e4: return "NVC7C0_SET_SHADER_LOCAL_MEMORY_NON_THROTTLED_A(size)";
    case 0x02e8: return "NVC7C0_SET_SHADER_LOCAL_MEMORY_NON_THROTTLED_B(size)";
    case 0x02ec: return "NVC7C0_SET_SHADER_LOCAL_MEMORY_NON_THROTTLED_C(max_sm)";
    case 0x0310: return "NVC7C0_(0x0310 -- context init)";
    case 0x0790: return "NVC7C0_SET_SHADER_LOCAL_MEMORY_A(ADDRESS_UPPER) <== a real POINTER";
    case 0x0794: return "NVC7C0_SET_SHADER_LOCAL_MEMORY_B(ADDRESS_LOWER) <== a real POINTER";
    case 0x07b0: return "NVC7C0_SET_SHADER_LOCAL_MEMORY_WINDOW_A <== APERTURE BASE";
    case 0x07b4: return "NVC7C0_SET_SHADER_LOCAL_MEMORY_WINDOW_B <== APERTURE BASE";
    case 0x155c: return "NVC7C0_SET_TEX_SAMPLER_POOL_A";
    case 0x1560: return "NVC7C0_SET_TEX_SAMPLER_POOL_B";
    case 0x1564: return "NVC7C0_SET_TEX_SAMPLER_POOL_C(max_index)";
    case 0x1574: return "NVC7C0_SET_TEX_HEADER_POOL_A";
    case 0x1578: return "NVC7C0_SET_TEX_HEADER_POOL_B";
    case 0x157c: return "NVC7C0_SET_TEX_HEADER_POOL_C(max_index)";
    default: return NULL;
    }
}

/* Recovered semaphore/launch facts from one decoded pushbuffer segment. */
struct pbfacts {
    int      n_launch_dma;      /* CE LAUNCH_DMA seen                        */
    int      n_report_sem;      /* GR SET_REPORT_SEMAPHORE seen              */
    int      n_ce_setsem;       /* CE SET_SEMAPHORE_A/B seen                 */
    int      n_host_sem;        /* host SEM_ADDR/SEM_EXECUTE seen            */
    int      n_qmd;             /* compute launch (QMD / SEND_PCAS)          */
    int      n_i2m_launch;      /* NVC7C0_LAUNCH_DMA -- the compute-class I2M */
    int      n_inline_data;     /* NVC7C0_LOAD_INLINE_DATA payload dwords     */
    uint32_t inline_first;      /* the first inline literal seen              */
    uint64_t i2m_off_out;       /* NVC7C0_OFFSET_OUT_UPPER / OFFSET_OUT       */
    uint32_t i2m_line_len, i2m_line_cnt, i2m_launch;
    uint64_t ce_sem_va, host_sem_va, gr_sem_va;
    uint32_t ce_sem_payload, host_sem_payload_lo, gr_sem_payload;
    uint64_t off_in, off_out;
    uint32_t line_len, launch_dma;
    /* ★ w274 -- the context-init segment's own operands. `have_*` is separate from the
     * value because 0 is a legal window base and "absent" must not decode to it. */
    uint64_t shared_win, local_win, local_mem, tex_hdr, tex_smp, span_ovf;
    int have_shared_win, have_local_win, have_local_mem, have_tex_hdr, have_tex_smp,
        have_span_ovf;
    int n_mme_dw;               /* dwords written to LOAD_MME_INSTRUCTION_RAM */
};

/* Decode a Pascal+/Ampere pushbuffer segment. dwords at cpu_va, n dwords. */
static void decode_pb(uint64_t cpu_va, uint32_t ndw, struct pbfacts *f, int verbose)
{
    uint32_t *b = malloc((size_t)ndw * 4);
    uint32_t i = 0;
    uint32_t pend_ce_a = 0, pend_ce_b = 0, have_ce_a = 0, have_ce_b = 0;
    uint32_t pend_h_lo = 0, pend_h_hi = 0, have_h_lo = 0, have_h_hi = 0;
    uint32_t pend_g_lo = 0, pend_g_hi = 0, have_g_lo = 0, have_g_hi = 0;
    uint32_t subch_class[8];
    memset(subch_class, 0, sizeof subch_class);
    if (!b) return;
    if (sread((void *)cpu_va, b, (size_t)ndw * 4) != (size_t)ndw * 4) {
        if (verbose) L("      <pushbuffer NOT READABLE at 0x%llx>", (unsigned long long)cpu_va);
        free(b); return;
    }
    while (i < ndw) {
        uint32_t d = b[i];
        uint32_t sec = d >> 29;
        /* ogkm clb06f.h:203 -- NVB06F_DMA_INCR_ADDRESS is 11:0 and holds
         * (address >> 2). The Kepler-era 12:2 encoding is _ADDRESS_OLD and is
         * NOT what 580.159.04 emits; decoding with it silently mis-names every
         * method above 0x1FFC (which is most of the GR class). */
        uint32_t addr = (d & 0xfff) << 2;
        uint32_t subch = (d >> 13) & 7;
        uint32_t cnt = (d >> 16) & 0x1fff;
        uint32_t imm = (d >> 16) & 0x1fff;
        const char *inc = "?";
        uint32_t j;

        switch (sec) {
        case 1: inc = "INC";      break;
        case 3: inc = "NONINC";   break;
        case 4: inc = "IMMD";     break;
        case 5: inc = "ONEINC";   break;
        case 7: inc = "END_SEG";  break;
        case 0: inc = "GRP0";     break;
        default: inc = "SEC?";    break;
        }
        if (sec == 7) { if (verbose) L("      +%-4u %08x  END_PB_SEGMENT", i * 4, d); break; }

        if (sec == 4) {
            const char *nm = ce_method(addr); if (!nm) nm = host_method(addr);
            if (!nm) nm = gr_method(addr);
            if (verbose) L("      +%-4u %08x  IMMD  sub=%u mth=0x%04x data=0x%x  %s",
                           i * 4, d, subch, addr, imm, nm ? nm : "");
            if (addr == 0x0000) subch_class[subch] = imm;
            i++;
            continue;
        }
        if (verbose) L("      +%-4u %08x  %-6s sub=%u mth=0x%04x cnt=%u", i * 4, d, inc, subch, addr, cnt);
        for (j = 0; j < cnt && i + 1 + j < ndw; j++) {
            /* NON_INC keeps the address; ONE_INC bumps once then holds. */
            uint32_t a = (sec == 3) ? addr : (sec == 5 ? addr + (j ? 4 : 0) : addr + 4 * j);
            uint32_t v = b[i + 1 + j];
            const char *nm = ce_method(a); const char *hm = host_method(a); const char *gm = gr_method(a);
            if (verbose)
                L("           [%2u] mth=0x%04x val=0x%08x  %s", j, a, v,
                  nm ? nm : (hm ? hm : (gm ? gm : "")));
            if (a == 0x0000) subch_class[subch] = v;
            /* CE */
            if (a == 0x0240) { pend_ce_a = v; have_ce_a = 1; f->n_ce_setsem++; }
            if (a == 0x0244) { pend_ce_b = v; have_ce_b = 1; f->n_ce_setsem++; }
            if (a == 0x0248) f->ce_sem_payload = v;
            if (a == 0x0400) f->off_in  = ((uint64_t)v << 32) | (f->off_in  & 0xffffffffu);
            if (a == 0x0404) f->off_in  = (f->off_in  & ~0xffffffffull) | v;
            if (a == 0x0408) f->off_out = ((uint64_t)v << 32) | (f->off_out & 0xffffffffu);
            if (a == 0x040C) f->off_out = (f->off_out & ~0xffffffffull) | v;
            if (a == 0x0418) f->line_len = v;
            if (a == 0x0300) { f->n_launch_dma++; f->launch_dma = v; }
            /* host */
            if (a == 0x005c) { pend_h_lo = v; have_h_lo = 1; }
            if (a == 0x0060) { pend_h_hi = v; have_h_hi = 1; }
            if (a == 0x0064) f->host_sem_payload_lo = v;
            if (a == 0x006c) f->n_host_sem++;
            /* GR */
            if (a == 0x1b00) { pend_g_hi = v; have_g_hi = 1; }
            if (a == 0x1b04) { pend_g_lo = v; have_g_lo = 1; }
            if (a == 0x1b08) f->gr_sem_payload = v;
            if (a == 0x1b0c) f->n_report_sem++;
            if (a == 0x0160) { pend_g_lo = v; have_g_lo = 1; }
            if (a == 0x0164) { pend_g_hi = v; have_g_hi = 1; }
            if (a == 0x0ad0 || a == 0x0ad4 || a == 0x0268 || a == 0x026c) f->n_qmd++;
            if (a == 0x0180) f->i2m_line_len = v;
            if (a == 0x0184) f->i2m_line_cnt = v;
            if (a == 0x0188) f->i2m_off_out = ((uint64_t)v << 32) | (f->i2m_off_out & 0xffffffffu);
            if (a == 0x018c) f->i2m_off_out = (f->i2m_off_out & ~0xffffffffull) | v;
            if (a == 0x01b0) { f->n_i2m_launch++; f->i2m_launch = v; }
            if (a == 0x01b4) { if (!f->n_inline_data) f->inline_first = v; f->n_inline_data++; }
            /* ★ w274 -- context-init operands. ⚠ the `_A` field width is NOT constant:
             * these five are all `16:0` per clc7c0.h, unlike SET_REPORT_SEMAPHORE_A's
             * `7:0`. Masking them all with 0xff reports 0 for a real address. */
            if (a == 0x02a0) { f->shared_win = ((uint64_t)(v & 0x1ffff) << 32) | (f->shared_win & 0xffffffffu); f->have_shared_win = 1; }
            if (a == 0x02a4)   f->shared_win = (f->shared_win & ~0xffffffffull) | v;
            if (a == 0x07b0) { f->local_win  = ((uint64_t)(v & 0x1ffff) << 32) | (f->local_win  & 0xffffffffu); f->have_local_win = 1; }
            if (a == 0x07b4)   f->local_win  = (f->local_win  & ~0xffffffffull) | v;
            if (a == 0x0790) { f->local_mem  = ((uint64_t)(v & 0x1ffff) << 32) | (f->local_mem  & 0xffffffffu); f->have_local_mem = 1; }
            if (a == 0x0794)   f->local_mem  = (f->local_mem  & ~0xffffffffull) | v;
            if (a == 0x1574) { f->tex_hdr    = ((uint64_t)(v & 0x1ffff) << 32) | (f->tex_hdr    & 0xffffffffu); f->have_tex_hdr = 1; }
            if (a == 0x1578)   f->tex_hdr    = (f->tex_hdr    & ~0xffffffffull) | v;
            if (a == 0x155c) { f->tex_smp    = ((uint64_t)(v & 0x1ffff) << 32) | (f->tex_smp    & 0xffffffffu); f->have_tex_smp = 1; }
            if (a == 0x1560)   f->tex_smp    = (f->tex_smp    & ~0xffffffffull) | v;
            if (a == 0x0200) { f->span_ovf   = ((uint64_t)(v & 0xff)    << 32) | (f->span_ovf   & 0xffffffffu); f->have_span_ovf = 1; }
            if (a == 0x0204)   f->span_ovf   = (f->span_ovf   & ~0xffffffffull) | v;
            if (a == 0x0118)   f->n_mme_dw++;
        }
        i += 1 + cnt;
    }
    if (have_ce_a && have_ce_b) f->ce_sem_va   = ((uint64_t)pend_ce_a << 32) | pend_ce_b;
    if (have_h_lo && have_h_hi) f->host_sem_va = ((uint64_t)(pend_h_hi & 0xff) << 32) | (pend_h_lo & 0xfffffffcu);
    if (have_g_lo && have_g_hi) f->gr_sem_va   = ((uint64_t)(pend_g_hi & 0xff) << 32) | pend_g_lo;
    free(b);
}

/* ---------------------------------------------------------- perf breakpoints */

struct bp { const char *name; uint64_t addr; int len; int *fds; int nfds; };

static int perf_bp_open(pid_t tid, uint64_t addr, int len)
{
    struct perf_event_attr a;
    memset(&a, 0, sizeof a);
    a.type = PERF_TYPE_BREAKPOINT;
    a.size = sizeof a;
    a.bp_type = HW_BREAKPOINT_W;
    a.bp_addr = addr;
    a.bp_len = len;
    a.sample_period = 0;
    a.disabled = 1;
    a.exclude_kernel = 0;
    a.exclude_hv = 1;
    return (int)syscall(__NR_perf_event_open, &a, tid, -1, -1, 0);
}

/* Arm one write-breakpoint on EVERY thread of this process. perf_event_open
 * with pid=<tid> attaches to that thread only, so a per-thread sweep is the
 * only way to cover libcuda's worker threads (created before we got here). */
static int bp_arm(struct bp *b, const char *name, uint64_t addr, int len)
{
    DIR *d = opendir("/proc/self/task");
    struct dirent *e;
    int cap = 64, n = 0;
    b->name = name; b->addr = addr; b->len = len;
    b->fds = calloc(cap, sizeof(int)); b->nfds = 0;
    if (!d || !b->fds) { if (d) closedir(d); return -1; }
    while ((e = readdir(d))) {
        pid_t tid;
        int fd;
        if (e->d_name[0] < '0' || e->d_name[0] > '9') continue;
        tid = (pid_t)atoi(e->d_name);
        fd = perf_bp_open(tid, addr, len);
        if (fd < 0) { L("  bp_arm(%s) tid=%d FAILED: %s", name, (int)tid, strerror(errno)); continue; }
        if (n >= cap) { cap *= 2; b->fds = realloc(b->fds, cap * sizeof(int)); }
        b->fds[n++] = fd;
    }
    closedir(d);
    b->nfds = n;
    return n;
}
static void bp_enable(struct bp *b)  { int i; for (i = 0; i < b->nfds; i++) ioctl(b->fds[i], PERF_EVENT_IOC_RESET, 0), ioctl(b->fds[i], PERF_EVENT_IOC_ENABLE, 0); }
static void bp_disable(struct bp *b) { int i; for (i = 0; i < b->nfds; i++) ioctl(b->fds[i], PERF_EVENT_IOC_DISABLE, 0); }
static uint64_t bp_count(struct bp *b)
{
    uint64_t tot = 0, v; int i;
    for (i = 0; i < b->nfds; i++) if (read(b->fds[i], &v, 8) == 8) tot += v;
    return tot;
}

/* --------------------------------------------------------------- the poller */

/* A 4-word semaphore report is [payload, pad, ts_lo, ts_hi]; the timestamp comes
 * from the GPU's own clock. We sample all four, ALWAYS through a volatile
 * pointer so the compiler cannot hoist the load out of the loop. */
struct semsample { uint32_t w[4]; };
/* Per sample we take EVERY channel's cursor pair and EVERY 16-byte slot in the
 * semaphore page. Polling only the channel we expect would make a submission on
 * any other channel invisible, and "we saw nothing" would then be a property of
 * the instrument. */
#define NCHMAX_POLL 16
#define NSLOT 16
struct sample {
    double t;
    uint32_t gpget, gpput;                    /* the identified channel */
    uint32_t chget[NCHMAX_POLL], chput[NCHMAX_POLL];
    uint32_t slot[NSLOT];                     /* payload word of each slot */
    struct semsample ce, gr, host;
};
#define MAXSAMP 60000
static struct sample g_samp[MAXSAMP];
static volatile int g_nsamp, g_poll_run;
static uint64_t g_userd_va, g_sem_va, g_sem_va2, g_semhost_va;
static int g_userd_ch = -1;
static uint64_t g_poll_userd[NCHMAX_POLL];
static int      g_poll_nuserd;
static uint64_t g_slotbase;                   /* semaphore page + 0xf00 */

static inline void read4(uint64_t va, struct semsample *s)
{
    if (!va) { s->w[0] = s->w[1] = s->w[2] = s->w[3] = 0; return; }
    s->w[0] = ((volatile uint32_t *)va)[0];
    s->w[1] = ((volatile uint32_t *)va)[1];
    s->w[2] = ((volatile uint32_t *)va)[2];
    s->w[3] = ((volatile uint32_t *)va)[3];
}

static void *poller(void *unused)
{
    (void)unused;
    /* sigjmp_buf is __thread; the disposition is process-wide but the buffer
     * is not, so the poller must arm its own before any direct device read. */
    g_faulting = 0;
    while (g_poll_run) {
        int i = g_nsamp;
        if (i >= MAXSAMP) break;
        g_samp[i].t = now_s() - g_t0;
        g_samp[i].gpget = g_samp[i].gpput = 0xffffffffu;
        if (g_userd_va) {
            volatile uint32_t *u = (volatile uint32_t *)(g_userd_va);
            g_samp[i].gpget = u[USERD_GP_GET / 4];
            g_samp[i].gpput = u[USERD_GP_PUT / 4];
        }
        /* GP_GET and the semaphore words are read in the SAME iteration, so
         * their relative order is measured rather than inferred. */
        {
            int c;
            for (c = 0; c < g_poll_nuserd; c++) {
                volatile uint32_t *u = (volatile uint32_t *)g_poll_userd[c];
                g_samp[i].chget[c] = u[USERD_GP_GET / 4];
                g_samp[i].chput[c] = u[USERD_GP_PUT / 4];
            }
            if (g_slotbase) {
                volatile uint32_t *sl = (volatile uint32_t *)g_slotbase;
                for (c = 0; c < NSLOT; c++) g_samp[i].slot[c] = sl[c * 4];
            }
        }
        read4(g_sem_va,     &g_samp[i].ce);
        read4(g_sem_va2,    &g_samp[i].gr);
        read4(g_semhost_va, &g_samp[i].host);
        __atomic_store_n(&g_nsamp, i + 1, __ATOMIC_RELEASE);
    }
    return NULL;
}

/* ------------------------------------------------------------- memory scan */

struct hit { uint64_t addr; };
#define MAXHIT 256
static struct hit g_hit[MAXHIT];
static int g_nhit;

static uint64_t g_scan_cap = 512ull << 20;

/* Scan every PRESENT page of every readable mapping for a 64-bit pattern. */
static int scan_pattern(uint64_t pat, const char *what)
{
    int i;
    uint64_t scanned = 0;
    uint8_t *buf = malloc(1u << 20);
    g_nhit = 0;
    if (!buf) return 0;
    for (i = 0; i < g_nmap && scanned < g_scan_cap; i++) {
        uint64_t a;
        if (g_map[i].perm[0] != 'r') continue;
        if (strstr(g_map[i].path, "[vvar]") || strstr(g_map[i].path, "[vsyscall]")) continue;
        for (a = g_map[i].lo; a < g_map[i].hi && scanned < g_scan_cap; a += (uint64_t)g_pgsz) {
            size_t got, k;
            if (!scan_ok(&g_map[i], a)) continue;
            got = sread((void *)a, buf, (size_t)g_pgsz);
            scanned += got;
            for (k = 0; k + 8 <= got; k += 4) {
                uint64_t v; memcpy(&v, buf + k, 8);
                if (v == pat && g_nhit < MAXHIT) g_hit[g_nhit++].addr = a + k;
            }
        }
    }
    free(buf);
    L("  scan(%s pat=0x%016llx): %d hit(s), %llu MiB present-scanned",
      what, (unsigned long long)pat, g_nhit, (unsigned long long)(scanned >> 20));
    return g_nhit;
}

static void hexdump_to(const char *fname, uint64_t va, size_t len)
{
    char p[512]; int fd; void *b = malloc(len);
    if (!b) return;
    if (sread((void *)va, b, len) != len) { free(b); return; }
    snprintf(p, sizeof p, "%s/%s", g_rawdir, fname);
    fd = open(p, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) { if (write(fd, b, len) < 0) {} close(fd); L("  raw -> %s (%zu bytes from 0x%llx)", p, len, (unsigned long long)va); }
    free(b);
}

static void hexlog(uint64_t va, size_t len)
{
    uint8_t b[1024]; size_t i;
    if (len > sizeof b) len = sizeof b;
    if (sread((void *)va, b, len) != len) { L("      <unreadable 0x%llx>", (unsigned long long)va); return; }
    for (i = 0; i < len; i += 16) {
        char line[160]; int o = 0; size_t j;
        o += snprintf(line + o, sizeof line - o, "      %016llx:", (unsigned long long)(va + i));
        for (j = 0; j < 16 && i + j < len; j++)
            o += snprintf(line + o, sizeof line - o, " %02x", b[i + j]);
        L("%s", line);
    }
}

/* ------------------------------------------------------------ aperture ---- */
/*
 * Which aperture does this VA actually live in? We do not assume: we classify
 * from the mapping that backs it and cross-check against the nvidia mmap
 * windows we recorded. The distinction changes how the word may legally be
 * read (snooped ordinary load vs. an uncached PCIe read).
 */
static const char *aperture_of(uint64_t va, char *why, size_t n)
{
    const struct maprec *m = map_of(va);
    int i;
    if (!m) { snprintf(why, n, "no mapping"); return "UNMAPPED"; }
    for (i = 0; i < g_nmp; i++) {
        if (va >= g_mp[i].ret && va < g_mp[i].ret + g_mp[i].len) {
            snprintf(why, n, "inside nvidia mmap window #%d dev=%s devoff=0x%llx len=0x%llx",
                     i, g_mp[i].dev, (unsigned long long)g_mp[i].off,
                     (unsigned long long)g_mp[i].len);
            return "DEVICE-WINDOW (BAR / vidmem or driver-managed sysmem)";
        }
    }
    if (m->path[0] == 0 || strcmp(m->path, "[heap]") == 0 || strcmp(m->path, "[stack]") == 0) {
        snprintf(why, n, "anonymous mapping %s, no nvidia mmap window covers it", m->path[0] ? m->path : "[anon]");
        return "SYSMEM (anonymous, host RAM)";
    }
    snprintf(why, n, "file-backed mapping %s", m->path);
    return strncmp(m->path, "/dev/nvidia", 11) == 0 ? "DEVICE-FILE mapping" : "FILE-BACKED";
}

/* ------------------------------------------------------------------- CUDA */

typedef int CUresult_t;
static CUresult_t (*p_cuInit)(unsigned);
static CUresult_t (*p_cuDeviceGetCount)(int *);
static CUresult_t (*p_cuDeviceGet)(int *, int);
static CUresult_t (*p_cuDeviceGetName)(char *, int, int);
static CUresult_t (*p_cuCtxCreate)(void **, unsigned, int);
static CUresult_t (*p_cuMemAlloc)(uint64_t *, size_t);
static CUresult_t (*p_cuMemcpyHtoD)(uint64_t, const void *, size_t);
static CUresult_t (*p_cuMemcpyDtoH)(void *, uint64_t, size_t);
static CUresult_t (*p_cuCtxSynchronize)(void);

#define CK(x) do { int _r = (x); L("  %-28s -> %d", #x, _r); if (_r) { L("FAIL at %s", #x); goto done; } } while (0)

/* ================= physical address / PCI BAR: the APERTURE test ==========
 * We do not infer the aperture from a flag word. We take the semaphore's
 * PHYSICAL page out of /proc/self/pagemap and compare it against the GPU's own
 * PCI BAR ranges from sysfs. Inside a BAR => vidmem/BAR (a CPU read crosses
 * PCIe, uncached/WC, stale-read hazards are real). Outside => host RAM
 * (the DMA write is snooped and an ordinary volatile load sees it).
 */
#define MAXBAR 8
static struct { uint64_t start, end, flags; int idx; } g_bar[MAXBAR];
static int g_nbar;
static char g_pcidev[64];

static void pci_bars_load(void)
{
    DIR *d = opendir("/sys/bus/pci/drivers/nvidia");
    struct dirent *e;
    char p[256];
    FILE *f;
    int i = 0;
    if (!d) return;
    while ((e = readdir(d))) {
        if (strlen(e->d_name) == 12 && e->d_name[4] == ':') {
            snprintf(g_pcidev, sizeof g_pcidev, "%s", e->d_name);
            break;
        }
    }
    closedir(d);
    if (!g_pcidev[0]) return;
    snprintf(p, sizeof p, "/sys/bus/pci/devices/%s/resource", g_pcidev);
    f = fopen(p, "r");
    if (!f) return;
    while (i < MAXBAR) {
        unsigned long long a, b, c;
        if (fscanf(f, "%llx %llx %llx", &a, &b, &c) != 3) break;
        if (b > a) { g_bar[g_nbar].start = a; g_bar[g_nbar].end = b;
                     g_bar[g_nbar].flags = c; g_bar[g_nbar].idx = i; g_nbar++; }
        i++;
    }
    fclose(f);
}

static int phys_of(uint64_t va, uint64_t *phys)
{
    uint64_t ent;
    off_t off = (off_t)(va / (uint64_t)g_pgsz) * 8;
    if (g_pagemap < 0) return 0;
    if (pread(g_pagemap, &ent, 8, off) != 8) return 0;
    if (!((ent >> 63) & 1)) return 0;
    *phys = ((ent & ((1ull << 55) - 1)) * (uint64_t)g_pgsz) + (va & (uint64_t)(g_pgsz - 1));
    return 1;
}

static const char *aperture_phys(uint64_t va, char *why, size_t n)
{
    uint64_t ph;
    int i;
    if (!phys_of(va, &ph)) { snprintf(why, n, "pagemap gave no PFN (not present, or no privilege)"); return "UNDETERMINED"; }
    for (i = 0; i < g_nbar; i++)
        if (ph >= g_bar[i].start && ph <= g_bar[i].end) {
            snprintf(why, n, "phys 0x%llx is inside GPU %s BAR%d [0x%llx-0x%llx]",
                     (unsigned long long)ph, g_pcidev, g_bar[i].idx,
                     (unsigned long long)g_bar[i].start, (unsigned long long)g_bar[i].end);
            return "VIDMEM/BAR (CPU read crosses PCIe)";
        }
    snprintf(why, n, "phys 0x%llx is in NO GPU BAR -- host RAM", (unsigned long long)ph);
    return "SYSMEM (host RAM, snooped)";
}

/* ================= structural GPFIFO ring finder ==========================
 * We do not need the channel-alloc params to find the ring: a ring entry is a
 * dword pair that decodes to a readable pushbuffer address with a sane length.
 * Given a KNOWN pushbuffer address we search every present page for the pair
 * that points AT it (or just below it, since the entry names the segment start).
 */
static int find_ring_for(uint64_t pb_target, uint64_t *ring_out, uint32_t *idx_out,
                         uint64_t *pb_out, uint32_t *len_out)
{
    int i, found = 0;
    uint64_t scanned = 0;
    uint8_t *buf = malloc(1u << 20);
    if (!buf) return 0;
    for (i = 0; i < g_nmap && scanned < g_scan_cap; i++) {
        uint64_t a;
        if (g_map[i].perm[0] != 'r' || g_map[i].perm[1] != 'w') continue;
        if (strstr(g_map[i].path, "[vvar]") || strstr(g_map[i].path, "[vsyscall]")) continue;
        for (a = g_map[i].lo; a < g_map[i].hi && scanned < g_scan_cap; a += (uint64_t)g_pgsz) {
            size_t got, k;
            if (!scan_ok(&g_map[i], a)) continue;
            got = sread((void *)a, buf, (size_t)g_pgsz);
            scanned += got;
            for (k = 0; k + 8 <= got; k += 8) {
                uint32_t e0, e1, ln;
                uint64_t pb;
                memcpy(&e0, buf + k, 4); memcpy(&e1, buf + k + 4, 4);
                if (!e0 && !e1) continue;
                pb = gpe_addr(e0, e1);
                ln = gpe_len(e1);
                if (!ln || ln > 8192) continue;
                if (pb > pb_target || pb + 4ull * ln <= pb_target) continue;
                /* This entry's segment CONTAINS the address we found. */
                L("  RING HIT: entry at 0x%016llx = %08x %08x -> pbuf 0x%010llx len=%u dw"
                  "  (contains 0x%llx)", (unsigned long long)(a + k), e0, e1,
                  (unsigned long long)pb, ln, (unsigned long long)pb_target);
                if (!found) {
                    *pb_out = pb; *len_out = ln;
                    /* Walk back to the start of the ring: entries are 8 bytes and a
                     * ring is page-aligned in practice; report both the entry VA and
                     * the containing page. */
                    *ring_out = a; *idx_out = (uint32_t)(k / 8);
                }
                found++;
            }
        }
    }
    free(buf);
    L("  find_ring_for(0x%llx): %d candidate entr(ies), %llu MiB present-scanned",
      (unsigned long long)pb_target, found, (unsigned long long)(scanned >> 20));
    return found;
}

/* Widened variant: any entry naming an address in [target-slack, target]. */
static int find_ring_near(uint64_t target, uint64_t slack, uint64_t *ring_out,
                          uint32_t *idx_out, uint64_t *pb_out, uint32_t *len_out)
{
    int i, found = 0;
    uint64_t scanned = 0;
    uint8_t *buf = malloc(1u << 20);
    if (!buf) return 0;
    for (i = 0; i < g_nmap && scanned < g_scan_cap; i++) {
        uint64_t a;
        if (g_map[i].perm[0] != 'r') continue;
        if (strstr(g_map[i].path, "[vvar]") || strstr(g_map[i].path, "[vsyscall]")) continue;
        for (a = g_map[i].lo; a < g_map[i].hi && scanned < g_scan_cap; a += (uint64_t)g_pgsz) {
            size_t got, k;
            if (!scan_ok(&g_map[i], a)) continue;
            got = sread((void *)a, buf, (size_t)g_pgsz);
            scanned += got;
            for (k = 0; k + 8 <= got; k += 8) {
                uint32_t e0, e1, ln;
                uint64_t pb;
                memcpy(&e0, buf + k, 4); memcpy(&e1, buf + k + 4, 4);
                if (!e0 && !e1) continue;
                pb = gpe_addr(e0, e1); ln = gpe_len(e1);
                if (pb > target || target - pb > slack) continue;
                if (!in_nvidia_window(a + k)) continue;   /* stack/heap copies are not rings */
                L("  NEAR-RING: entry @0x%016llx = %08x %08x -> 0x%010llx len=%u dw "
                  "(anchor is +%llu bytes in)", (unsigned long long)(a + k), e0, e1,
                  (unsigned long long)pb, ln, (unsigned long long)(target - pb));
                if (!found) { *ring_out = a + k; *idx_out = (uint32_t)(k / 8);
                              *pb_out = pb; *len_out = ln ? ln : 256; }
                found++;
            }
        }
    }
    free(buf);
    L("  find_ring_near(0x%llx, %llu): %d candidate(s), %llu MiB scanned",
      (unsigned long long)target, (unsigned long long)slack, found,
      (unsigned long long)(scanned >> 20));
    return found;
}

/* ================= USERD by differential ==================================
 * Every 4 KiB /dev/nvidia* window is a USERD candidate. We snapshot GP_GET and
 * GP_PUT in all of them, do one copy, and snapshot again. The window whose
 * GP_PUT MOVED is the channel that ran the copy -- measured, not guessed.
 */
struct userdcand { uint64_t va; int ch; uint32_t get0, put0, get1, put1; };
#define MAXUC 128
static struct userdcand g_uc[MAXUC];
static int g_nuc;

static void userd_snapshot(int which)
{
    int i;
    if (which == 0) {
        g_nuc = 0;
        /* The channel table is authoritative: one USERD per channel. */
        for (i = 0; i < g_nch && g_nuc < MAXUC; i++) {
            if (!g_ch[i].userd_cpu) continue;
            g_uc[g_nuc].va = g_ch[i].userd_cpu;
            g_uc[g_nuc].ch = i;
            rd32(g_uc[g_nuc].va + USERD_GP_GET, &g_uc[g_nuc].get0);
            rd32(g_uc[g_nuc].va + USERD_GP_PUT, &g_uc[g_nuc].put0);
            g_nuc++;
        }
        /* plus every 4 KiB nvidia window, as a corroborating superset */
        for (i = 0; i < g_nmp && g_nuc < MAXUC; i++) {
            if (g_mp[i].len != 0x1000 || !g_mp[i].ret || g_mp[i].ret == (uint64_t)-1) continue;
            g_uc[g_nuc].va = g_mp[i].ret;
            g_uc[g_nuc].ch = -1;
            rd32(g_mp[i].ret + USERD_GP_GET, &g_uc[g_nuc].get0);
            rd32(g_mp[i].ret + USERD_GP_PUT, &g_uc[g_nuc].put0);
            g_nuc++;
        }
    } else {
        for (i = 0; i < g_nuc; i++) {
            rd32(g_uc[i].va + USERD_GP_GET, &g_uc[i].get1);
            rd32(g_uc[i].va + USERD_GP_PUT, &g_uc[i].put1);
        }
    }
}

/* ------------------------------------------------------------------- CUDA */

#define CK(x) do { int _r = (x); L("  %-28s -> %d", #x, _r); if (_r) { L("FAIL at %s", #x); goto done; } } while (0)

int main(void)
{
    void *lib;
    const char *o;
    int ndev = 0, dev = 0;
    void *ctx = NULL;
    uint64_t dp = 0;
    unsigned hv = 0xabcd1234, rv = 0;
    char nm[256] = {0};
    int i, ci;
    pthread_t th;
    struct bp bp_sem, bp_gpput;
    uint64_t c_sem = 0, c_gpput = 0;
    uint32_t ce_pre[4] = {0}, ce_post[4] = {0}, gr_pre[4] = {0}, gr_post[4] = {0};
    int armed_sem = 0, armed_gpput = 0;
    uint64_t pb_hit = 0, ring_va = 0, pb_va = 0;
    uint32_t ring_idx = 0, pb_len = 0;
    uint64_t ring_base_seen = 0;               /* ★ w274 -- for ITEM 2c */
    struct pbfacts allf; memset(&allf, 0, sizeof allf);
    struct pbfacts ctxf; memset(&ctxf, 0, sizeof ctxf);

    g_t0 = now_s();
    g_pid = getpid();
    g_pgsz = sysconf(_SC_PAGESIZE);
    g_pagemap = open("/proc/self/pagemap", O_RDONLY);
    install_fault_handlers();
    o = getenv("NVDP_OUT");
    g_log = o ? fopen(o, "w") : stdout;
    if (!g_log) g_log = stdout;
    if ((o = getenv("NVDP_RAW"))) snprintf(g_rawdir, sizeof g_rawdir, "%s", o);
    if ((o = getenv("NVDP_SCANCAP"))) g_scan_cap = strtoull(o, NULL, 0) << 20;
    mkdir(g_rawdir, 0755);
    pci_bars_load();

    L("=== nvdp: NATIVE data-plane capture of the cup2 workload ===");
    L("pid=%d pagesize=%ld pagemap=%s", (int)g_pid, g_pgsz, g_pagemap >= 0 ? "open" : "UNAVAILABLE");
    L("GPU pci device = %s ; %d BAR(s):", g_pcidev[0] ? g_pcidev : "<none>", g_nbar);
    for (i = 0; i < g_nbar; i++)
        L("  BAR%d 0x%012llx-0x%012llx flags=0x%llx", g_bar[i].idx,
          (unsigned long long)g_bar[i].start, (unsigned long long)g_bar[i].end,
          (unsigned long long)g_bar[i].flags);

    lib = dlopen("libcuda.so.1", RTLD_NOW);
    if (!lib) { L("dlopen(libcuda.so.1) FAILED: %s", dlerror()); return 1; }
    p_cuInit           = dlsym(lib, "cuInit");
    p_cuDeviceGetCount = dlsym(lib, "cuDeviceGetCount");
    p_cuDeviceGet      = dlsym(lib, "cuDeviceGet");
    p_cuDeviceGetName  = dlsym(lib, "cuDeviceGetName");
    p_cuCtxCreate      = dlsym(lib, "cuCtxCreate_v2");
    p_cuMemAlloc       = dlsym(lib, "cuMemAlloc_v2");
    p_cuMemcpyHtoD     = dlsym(lib, "cuMemcpyHtoD_v2");
    p_cuMemcpyDtoH     = dlsym(lib, "cuMemcpyDtoH_v2");
    p_cuCtxSynchronize = dlsym(lib, "cuCtxSynchronize");
    if (!p_cuInit || !p_cuCtxCreate || !p_cuMemAlloc || !p_cuMemcpyHtoD) { L("dlsym failed"); return 1; }

    L("--- STAGE 1: cuInit / device / cuCtxCreate ---");
    CK(p_cuInit(0));
    CK(p_cuDeviceGetCount(&ndev)); L("  devices=%d", ndev);
    CK(p_cuDeviceGet(&dev, 0));
    CK(p_cuDeviceGetName(nm, sizeof nm, dev)); L("  name=%s", nm);
    CK(p_cuCtxCreate(&ctx, 0, dev));

    /* A census zero needs a known-positive: print EVERY class RM_ALLOC saw, so
     * "0 channels" can be told apart from "the recogniser missed the class". */
    L("--- RM_ALLOC CLASS CENSUS (%d alloc(s) total) ---", g_nobj);
    {
        uint32_t seen[256]; int cnt[256], ns = 0, k;
        for (i = 0; i < g_nobj; i++) {
            for (k = 0; k < ns; k++) if (seen[k] == g_obj[i].cls) break;
            if (k == ns && ns < 256) { seen[ns] = g_obj[i].cls; cnt[ns] = 0; ns++; }
            if (k < 256) cnt[k]++;
        }
        for (k = 0; k < ns; k++)
            L("  class 0x%08x x%d%s", seen[k], cnt[k],
              is_channel_class(seen[k]) ? "   <== CHANNEL CLASS" : "");
    }
    L("--- CHANNEL-ALLOC DECODE DIAGNOSIS (%d channel-class alloc(s) reached it) ---", g_nchdiag);
    for (i = 0; i < g_nchdiag; i++) {
        char hx[160]; int k, oo = 0; hx[0] = 0;
        for (k = 0; k < 48 && (uint32_t)k < g_chdiag[i].got; k++)
            oo += snprintf(hx + oo, sizeof hx - oo, "%02x", g_chdiag[i].head[k]);
        L("  [%d] class=0x%04x iocsize=%u pAllocParms=0x%llx paramsSize=%u bytes_read=%u",
          i, g_chdiag[i].hClass, g_chdiag[i].iocsize,
          (unsigned long long)g_chdiag[i].pptr, g_chdiag[i].psz, g_chdiag[i].got);
        L("      first48=%s", hx);
    }
    L("--- CHANNELS DECODED (NV_CHANNEL_ALLOC_PARAMS) : %d ---", g_nch);
    for (ci = 0; ci < g_nch; ci++) {
        struct chan *c = &g_ch[ci];
        L("  ch[%d] t=%.6f class=0x%04x hObj=0x%08x parent=0x%08x", ci, c->t, c->hClass, c->hObjectNew, c->hObjectParent);
        L("        gpFifoOffset(GPU VA)=0x%016llx  gpFifoEntries=%u  flags=0x%08x",
          (unsigned long long)c->gpFifoOffset, c->gpFifoEntries, c->flags);
        L("        engineType=%u cid=%u hUserdMemory[0]=0x%08x userdOffset[0]=0x%llx",
          c->engineType, c->cid, c->hUserdMemory0, (unsigned long long)c->userdOffset0);
        L("        instanceMem base=0x%llx AS=%u | userdMem base=0x%llx AS=%u",
          (unsigned long long)c->instBase, c->instAS,
          (unsigned long long)c->userdBase, c->userdAS);
    }

    maps_reload();
    L("--- nvidia mmap windows (%d) ---", g_nmp);
    for (i = 0; i < g_nmp; i++)
        L("  mmap[%d] t=%.6f %-12s off=0x%012llx len=0x%llx prot=0x%x -> 0x%016llx",
          i, g_mp[i].t, g_mp[i].dev, (unsigned long long)g_mp[i].off,
          (unsigned long long)g_mp[i].len, g_mp[i].prot, (unsigned long long)g_mp[i].ret);

    L("--- STAGE 2: cuMemAlloc ---");
    CK(p_cuMemAlloc(&dp, 4096));
    L("  dp (device VA) = 0x%016llx  cpu-readable=%d", (unsigned long long)dp, readable(dp));

    L("=== CHANNEL DATA-PLANE TABLE (derived from NV_CHANNEL_ALLOC_PARAMS) ===");
    L("  gpFifoOffset is a GPU VA. The window that CONTAINS it gives the CPU VA of");
    L("  the same allocation, and hUserdMemory[0]/userdOffset[0] index into it.");
    for (ci = 0; ci < g_nch; ci++) {
        struct chan *c = &g_ch[ci];
        uint64_t base = 0;
        char why[256];
        for (i = 0; i < g_nmp; i++)
            if (c->gpFifoOffset >= g_mp[i].ret && c->gpFifoOffset < g_mp[i].ret + g_mp[i].len)
                { base = g_mp[i].ret; break; }
        c->ring_cpu  = readable(c->gpFifoOffset) ? c->gpFifoOffset : 0;
        c->userd_cpu = base ? base + c->userdOffset0 : 0;
        L("  ch[%2d] ring GPU VA=0x%llx cpu_readable=%d | window base=0x%llx | USERD=0x%llx",
          ci, (unsigned long long)c->gpFifoOffset, c->ring_cpu ? 1 : 0,
          (unsigned long long)base, (unsigned long long)c->userd_cpu);
        if (ci == 0) {
            L("         ring APERTURE : %s", aperture_phys(c->gpFifoOffset, why, sizeof why));
            L("                         [%s]", why);
            L("         ring mapping  : %s", aperture_of(c->gpFifoOffset, why, sizeof why));
            L("                         [%s]", why);
        }
    }

    L("--- STAGE 3: USERD differential across ALL channels ---");
    userd_snapshot(0);
    L("  %d candidate USERD window(s) snapshotted", g_nuc);
    CK(p_cuMemcpyHtoD(dp, &hv, 4));
    if (p_cuCtxSynchronize) p_cuCtxSynchronize();
    userd_snapshot(1);
    for (i = 0; i < g_nuc; i++) {
        int moved = (g_uc[i].put0 != g_uc[i].put1) || (g_uc[i].get0 != g_uc[i].get1);
        L("  userd_cand[%2d] ch=%-3d 0x%016llx GP_GET %u->%u GP_PUT %u->%u %s", i,
          g_uc[i].ch, (unsigned long long)g_uc[i].va, g_uc[i].get0, g_uc[i].get1,
          g_uc[i].put0, g_uc[i].put1, moved ? "  <== MOVED: this channel ran the copy" : "");
        if (moved && !g_userd_va) { g_userd_va = g_uc[i].va; g_userd_ch = g_uc[i].ch; }
    }
    if (!g_userd_va) L("  NO window moved. That is a RESULT: either USERD is not in a 4 KiB");
    if (!g_userd_va) L("  nvidia mmap, or the copy did not go through a doorbell channel.");

    maps_reload();

    /* ---- find the pushbuffer via the device pointer ------------------------ */
    L("=== ITEM 2: THE PUSHBUFFER (found via the device pointer it names) ===");
    L("  A CE writes OFFSET_OUT_UPPER then _LOWER; the compute class's I2M unit");
    L("  writes OFFSET_OUT_UPPER (0x188) then OFFSET_OUT (0x18c). Either way the");
    L("  pair reads as (dp>>32) followed by (dp & 0xffffffff).");
    scan_pattern(((uint64_t)(uint32_t)(dp & 0xffffffffu) << 32) | (uint32_t)(dp >> 32), "OUT_UPPER,OUT_LOWER");
    for (i = 0; i < g_nhit; i++) {
        char d[256];
        L("    hit %016llx  %s", (unsigned long long)g_hit[i].addr, map_desc(g_hit[i].addr, d, sizeof d));
        if (!pb_hit) {
            const struct maprec *m = map_of(g_hit[i].addr);
            if (m && strncmp(m->path, "/dev/nvidia", 11) == 0) pb_hit = g_hit[i].addr;
        }
    }
    if (!pb_hit && g_nhit) pb_hit = g_hit[0].addr;
    L("  chosen pushbuffer anchor = 0x%016llx", (unsigned long long)pb_hit);
    if (pb_hit) hexdump_to("pb_anchor_context.bin", pb_hit > 2048 ? pb_hit - 2048 : pb_hit, 4096);

    /* ---- 1: the ring ------------------------------------------------------- */
    L("=== ITEM 1: THE RING (GPFIFO) ===");
    if (pb_hit) find_ring_for(pb_hit, &ring_va, &ring_idx, &pb_va, &pb_len);
    if (!ring_va && pb_hit) {
        L("  strict containment found nothing; widening to ANY entry whose address");
        L("  is within 64 KiB below the anchor (a segment start we mis-sized).");
        find_ring_near(pb_hit, 65536, &ring_va, &ring_idx, &pb_va, &pb_len);
    }
    if (g_userd_ch >= 0 && g_ch[g_userd_ch].ring_cpu) {
        L("  AUTHORITATIVE: the channel whose GP_PUT moved is ch[%d]; its ring is at",
          g_userd_ch);
        L("  gpFifoOffset 0x%llx with %u entries.",
          (unsigned long long)g_ch[g_userd_ch].gpFifoOffset, g_ch[g_userd_ch].gpFifoEntries);
        ring_va = g_ch[g_userd_ch].gpFifoOffset;
        ring_idx = 0;
    }
    if (!ring_va && g_nch) {
        for (ci = 0; ci < g_nch; ci++)
            if (g_ch[ci].gpFifoOffset && readable(g_ch[ci].gpFifoOffset)) {
                L("  falling back to the channel-alloc gpFifoOffset 0x%llx (ch[%d])",
                  (unsigned long long)g_ch[ci].gpFifoOffset, ci);
                ring_va = g_ch[ci].gpFifoOffset; ring_idx = 0;
                break;
            }
    }
    if (ring_va) {
        char d[256];
        uint64_t ring_base = ring_va - 8ull * ring_idx;
        uint32_t e;
        L("  ring entry VA = 0x%016llx  (page 0x%016llx, entry #%u within the page)",
          (unsigned long long)ring_va, (unsigned long long)(ring_va & ~0xfffull), ring_idx);
        L("  containing mapping: %s", map_desc(ring_va, d, sizeof d));
        {
            char why[256];
            L("  ring APERTURE: %s [%s]", aperture_phys(ring_va, why, sizeof why), why);
        }
        if (g_userd_ch >= 0) ring_base = g_ch[g_userd_ch].gpFifoOffset;
        else                 ring_base = ring_va & ~0xfffull;
        hexdump_to("ring.bin", ring_base, 8192);
        ring_base_seen = ring_base;
        for (e = 0; e < 1024; e++) {
            uint32_t e0, e1;
            if (!rd32(ring_base + 8ull * e, &e0) || !rd32(ring_base + 8ull * e + 4, &e1)) break;
            if (!e0 && !e1) continue;
            if (pb_hit && gpe_addr(e0, e1) <= pb_hit &&
                pb_hit < gpe_addr(e0, e1) + 4ull * gpe_len(e1)) {
                pb_va = gpe_addr(e0, e1); pb_len = gpe_len(e1); ring_idx = e;
                ring_va = ring_base + 8ull * e;   /* so the marker below is right */
                L("  the entry that CONTAINS our pushbuffer anchor is gpe[%u]", e);
            }
        }
        L("  --- live GPFIFO entries in the containing page ---");
        for (e = 0; e < 512; e++) {
            uint32_t e0, e1;
            if (!rd32(ring_base + 8ull * e, &e0) || !rd32(ring_base + 8ull * e + 4, &e1)) break;
            if (!e0 && !e1) continue;
            L("    gpe[%3u] @0x%llx = %08x %08x -> pbuf 0x%010llx len=%u dw priv=%u lvl=%u sync=%u%s",
              e, (unsigned long long)(ring_base + 8ull * e), e0, e1,
              (unsigned long long)gpe_addr(e0, e1), gpe_len(e1),
              (e1 >> 8) & 1, (e1 >> 9) & 1, e1 >> 31,
              (ring_base + 8ull * e == ring_va) ? "   <== the entry carrying our copy" : "");
        }
    } else {
        L("  NO ring entry found pointing at the pushbuffer anchor.");
        L("  That is a RESULT: either the ring is not in a present, readable page of");
        L("  this process, or the submission did not go through a GPFIFO entry.");
    }

    /* ---- 2: decode the method stream --------------------------------------- */
    L("=== ITEM 2b: THE METHOD STREAM ===");
    if (pb_va && pb_len) {
        L("  decoding pushbuffer segment 0x%010llx, %u dwords", (unsigned long long)pb_va, pb_len);
        hexdump_to("pushbuffer.bin", pb_va, (size_t)pb_len * 4);
        decode_pb(pb_va, pb_len, &allf, 1);
    } else if (pb_hit) {
        uint64_t s = (pb_hit > 512 ? pb_hit - 512 : 0) & ~63ull;
        L("  no ring entry -- decoding 192 dwords from 0x%llx as a best effort;",
          (unsigned long long)s);
        L("  the segment START is NOT known, so leading headers may be misaligned.");
        decode_pb(s, 192, &allf, 1);
    }
    L("  FACTS: i2m_launch=%d inline_dwords=%d first_inline=0x%08x ce_launch_dma=%d "
      "ce_setsem=%d host_sem=%d gr_report_sem=%d qmd=%d",
      allf.n_i2m_launch, allf.n_inline_data, allf.inline_first, allf.n_launch_dma,
      allf.n_ce_setsem, allf.n_host_sem, allf.n_report_sem, allf.n_qmd);
    if (allf.i2m_off_out)
        L("  I2M destination = 0x%016llx  line_len=%u line_count=%u launch=0x%08x  (dp=0x%llx, match=%d)",
          (unsigned long long)allf.i2m_off_out, allf.i2m_line_len, allf.i2m_line_cnt,
          allf.i2m_launch, (unsigned long long)dp, allf.i2m_off_out == dp);

    /* ---- 2c: THE CONTEXT-INIT SEGMENT (gpe[0]) ------------------------------
     * ★★★ w274. The guest's decoder only ever dumps ring index 0, so gpe[0] is the ONE
     * segment a native<->guest byte comparison can be made over -- and it is also the only
     * segment that names SET_SHADER_SHARED_MEMORY_WINDOW. ITEM 2/2b above decode the
     * segment carrying the COPY (gpe[110] in the reference run), which is a different one.
     *
     * ⊘ For every 64-bit operand it names we print the CONTAINING /proc/self/maps record,
     * not merely "is it in an nvidia mmap window". The distinction is the result: a device
     * pointer has NO cpu mapping at all, an aperture base sits inside a PROT_NONE
     * RESERVATION, and a real pool is mapped. Printing only "not in a window" would collapse
     * all three into one answer. */
    L("=== ITEM 2c: THE CONTEXT-INIT SEGMENT (gpe[0]) ===");
    maps_reload();   /* ⚠ the census below is only as current as this */
    if (!ring_base_seen) {
        L("  ⊘ NO DUMP: no ring base was established. This is NOT 'the segment was empty'.");
    } else {
        uint32_t e0 = 0, e1 = 0;
        if (!rd32(ring_base_seen, &e0) || !rd32(ring_base_seen + 4, &e1)) {
            L("  ⊘ NO DUMP: gpe[0] at 0x%llx is not readable.",
              (unsigned long long)ring_base_seen);
        } else if (!e0 && !e1) {
            L("  ⊘ gpe[0] IS ZERO -- measured, not assumed.");
        } else {
            uint64_t cva = gpe_addr(e0, e1);
            uint32_t cdw = gpe_len(e1);
            L("  gpe[0] = %08x %08x -> pbuf 0x%010llx len=%u dw (%u bytes)",
              e0, e1, (unsigned long long)cva, cdw, cdw * 4);
            hexdump_to("pushbuffer_ctxinit.bin", cva, (size_t)cdw * 4);
            decode_pb(cva, cdw, &ctxf, 1);
            L("  CTX-INIT FACTS: mme_dwords=%d i2m_launch=%d qmd=%d gr_report_sem=%d",
              ctxf.n_mme_dw, ctxf.n_i2m_launch, ctxf.n_qmd, ctxf.n_report_sem);
            {
                struct { const char *k; uint64_t va; int have; } o[6] = {
                    { "SET_SHADER_SHARED_MEMORY_WINDOW (aperture)", ctxf.shared_win, ctxf.have_shared_win },
                    { "SET_SHADER_LOCAL_MEMORY_WINDOW  (aperture)", ctxf.local_win,  ctxf.have_local_win  },
                    { "SET_SHADER_LOCAL_MEMORY         (pointer) ", ctxf.local_mem,  ctxf.have_local_mem  },
                    { "SET_TEX_HEADER_POOL             (pointer) ", ctxf.tex_hdr,    ctxf.have_tex_hdr    },
                    { "SET_TEX_SAMPLER_POOL            (pointer) ", ctxf.tex_smp,    ctxf.have_tex_smp    },
                    { "SET_VALID_SPAN_OVERFLOW_AREA    (pointer) ", ctxf.span_ovf,   ctxf.have_span_ovf   } };
                int k;
                L("  --- every 64-bit operand gpe[0] names, against THIS process's own maps ---");
                L("  ⊘ 'NOT PRESENT' below means the method was ABSENT from the stream. It is a");
                L("     different fact from a value of 0, and neither is evidence of the other.");
                for (k = 0; k < 6; k++) {
                    char d[256];
                    const struct maprec *m;
                    if (!o[k].have) { L("    %-44s : NOT PRESENT in gpe[0]", o[k].k); continue; }
                    m = map_of(o[k].va);
                    L("    %-44s : 0x%016llx", o[k].k, (unsigned long long)o[k].va);
                    L("        containing map      = %s", map_desc(o[k].va, d, sizeof d));
                    L("        cpu-readable        = %d", readable(o[k].va));
                    if (m) {
                        L("        ★ the record        = 0x%llx-0x%llx perm=%s (%llu MiB), value at"
                          " +0x%llx = %llu%% of the way in",
                          (unsigned long long)m->lo, (unsigned long long)m->hi, m->perm,
                          (unsigned long long)((m->hi - m->lo) >> 20),
                          (unsigned long long)(o[k].va - m->lo),
                          (unsigned long long)(m->hi > m->lo
                              ? (100ull * (o[k].va - m->lo)) / (m->hi - m->lo) : 0));
                    } else {
                        L("        ★ NO /proc/self/maps RECORD CONTAINS IT — the VA is not in this");
                        L("          process's address space at all (a GPU-only mapping).");
                    }
                }
                L("  ⊘ CONTROL: dp = 0x%llx (a device pointer the GPU demonstrably writes) is",
                  (unsigned long long)dp);
                L("     itself %s. So 'no cpu mapping' can NEVER be read as 'no GPU backing'.",
                  map_of(dp) ? "inside a map record" : "in NO map record at all");
            }
        }
    }
    /* The whole map, so the reservation structure above is checkable after the fact. */
    {
        FILE *mf = fopen("/proc/self/maps", "r");
        char pth[512];
        snprintf(pth, sizeof pth, "%s/maps.txt", g_rawdir);
        if (mf) {
            FILE *of = fopen(pth, "w");
            if (of) {
                char ln[512];
                while (fgets(ln, sizeof ln, mf)) fputs(ln, of);
                fclose(of);
                L("  raw -> %s (the whole address space, for after-the-fact checking)", pth);
            }
            fclose(mf);
        }
    }

    /* ---- 3: the semaphore -------------------------------------------------- */
    L("=== ITEM 3: THE REPORT SEMAPHORE ===");
    g_sem_va     = allf.gr_sem_va;      /* GR SET_REPORT_SEMAPHORE -- the one we care about */
    g_sem_va2    = allf.ce_sem_va;
    g_semhost_va = allf.host_sem_va;
    {
        struct { const char *k; uint64_t va; uint32_t declared; } s[3] = {
            { "GR  SET_REPORT_SEMAPHORE", allf.gr_sem_va,   allf.gr_sem_payload },
            { "CE  SET_SEMAPHORE",        allf.ce_sem_va,   allf.ce_sem_payload },
            { "HOST SEM_ADDR",            allf.host_sem_va, allf.host_sem_payload_lo } };
        for (i = 0; i < 3; i++) {
            char d[256], why[256]; uint32_t w[4] = {0,0,0,0};
            if (!s[i].va) { L("  %-26s : NOT PRESENT in the decoded stream", s[i].k); continue; }
            L("  %-26s : VA=0x%016llx", s[i].k, (unsigned long long)s[i].va);
            L("       page offset          = +0x%03llx", (unsigned long long)(s[i].va & 0xfff));
            L("       cpu-readable         = %d", readable(s[i].va));
            L("       APERTURE (phys/BAR)  = %s", aperture_phys(s[i].va, why, sizeof why));
            L("                              [%s]", why);
            L("       APERTURE (mapping)   = %s", aperture_of(s[i].va, why, sizeof why));
            L("                              [%s]", why);
            L("       mapping              = %s", map_desc(s[i].va, d, sizeof d));
            L("       declared payload     = 0x%08x  (from the method stream)", s[i].declared);
            if (readable(s[i].va)) {
                sread((void *)s[i].va, w, 16);
                L("       report words NOW     = [payload=0x%08x pad=0x%08x ts_lo=0x%08x ts_hi=0x%08x]",
                  w[0], w[1], w[2], w[3]);
                L("       ts as u64            = %llu  (GPU clock -- nothing CPU-side can synthesise it)",
                  (unsigned long long)(((uint64_t)w[3] << 32) | w[2]));
                L("       landed == declared   = %d", w[0] == s[i].declared);
                { char fn[64]; snprintf(fn, sizeof fn, "sempage_%d.bin", i);
                  hexdump_to(fn, s[i].va & ~0xfffull, 4096); }
                L("       --- the whole semaphore page, last 256 bytes (the slot region) ---");
                hexlog((s[i].va & ~0xfffull) + 0xf00, 256);
            }
        }
    }

    /* ---- 4: authorship ------------------------------------------------------ */
    L("=== ITEM 4: WHO WRITES IT ===");
    L("  PRIMARY = the polling loop: the GPU timestamp inside the report, the landed");
    L("  payload vs the declared one, and the ordering against GP_GET.");
    L("  The write-breakpoint is a NEGATIVE CONTROL ONLY. A GPU release is a DMA");
    L("  write: it never touches the CPU MMU and x86 debug registers watch CPU");
    L("  accesses only, so SILENCE IS EXPECTED FOR ANY DMA AND PROVES NOTHING.");
    L("  If it FIRES, a CPU store did it and GPU authorship is REFUTED.");
    L("  GP_PUT carries the same breakpoint as the KNOWN-POSITIVE that the");
    L("  mechanism is live -- a census zero needs one.");
    memset(&bp_sem, 0, sizeof bp_sem); memset(&bp_gpput, 0, sizeof bp_gpput);
    if (g_sem_va && readable(g_sem_va)) armed_sem   = bp_arm(&bp_sem, "GR_SEM", g_sem_va, 4);
    if (g_userd_va)                     armed_gpput = bp_arm(&bp_gpput, "GP_PUT", g_userd_va + USERD_GP_PUT, 4);
    L("  armed: GR_SEM on %d thread(s), GP_PUT on %d thread(s)", armed_sem, armed_gpput);

    sread((void *)g_sem_va,  gr_pre, 16);
    sread((void *)g_sem_va2, ce_pre, 16);

    g_poll_nuserd = 0;
    for (ci = 0; ci < g_nch && g_poll_nuserd < NCHMAX_POLL; ci++)
        if (g_ch[ci].userd_cpu) g_poll_userd[g_poll_nuserd++] = g_ch[ci].userd_cpu;
    if (g_sem_va) g_slotbase = (g_sem_va & ~0xfffull) + 0xf00;
    L("  polling %d channel cursor pair(s) and %d semaphore slots from 0x%llx",
      g_poll_nuserd, NSLOT, (unsigned long long)g_slotbase);

    g_poll_run = 1; g_nsamp = 0;
    pthread_create(&th, NULL, poller, NULL);
    if (armed_sem > 0)   bp_enable(&bp_sem);
    if (armed_gpput > 0) bp_enable(&bp_gpput);

    L("--- STAGE 4: the WATCHED copies ---");
    L("  t_pre_htod  = %.6f", now_s() - g_t0);
    hv = 0x5a5a1234;
    CK(p_cuMemcpyHtoD(dp, &hv, 4));
    L("  t_post_htod = %.6f", now_s() - g_t0);
    CK(p_cuMemcpyDtoH(&rv, dp, 4));
    L("  t_post_dtoh = %.6f", now_s() - g_t0);

    if (armed_sem > 0)   { bp_disable(&bp_sem);   c_sem   = bp_count(&bp_sem); }
    if (armed_gpput > 0) { bp_disable(&bp_gpput); c_gpput = bp_count(&bp_gpput); }
    g_poll_run = 0;
    pthread_join(th, NULL);

    sread((void *)g_sem_va,  gr_post, 16);
    sread((void *)g_sem_va2, ce_post, 16);

    L("  copy result: rv=0x%08x want=0x%08x -> %s", rv, hv, rv == hv ? "PASS" : "MISMATCH");
    L("  --- report words BEFORE -> AFTER (raw, [payload pad ts_lo ts_hi]) ---");
    L("  GR  0x%016llx : [%08x %08x %08x %08x] -> [%08x %08x %08x %08x]",
      (unsigned long long)g_sem_va, gr_pre[0], gr_pre[1], gr_pre[2], gr_pre[3],
      gr_post[0], gr_post[1], gr_post[2], gr_post[3]);
    L("      GPU ts %llu -> %llu   (delta %lld ns-ish; GPU clock, not ours)",
      (unsigned long long)(((uint64_t)gr_pre[3] << 32) | gr_pre[2]),
      (unsigned long long)(((uint64_t)gr_post[3] << 32) | gr_post[2]),
      (long long)((((uint64_t)gr_post[3] << 32) | gr_post[2]) - (((uint64_t)gr_pre[3] << 32) | gr_pre[2])));
    L("  CE  0x%016llx : [%08x %08x %08x %08x] -> [%08x %08x %08x %08x]",
      (unsigned long long)g_sem_va2, ce_pre[0], ce_pre[1], ce_pre[2], ce_pre[3],
      ce_post[0], ce_post[1], ce_post[2], ce_post[3]);
    L("  --- NEGATIVE CONTROL (CPU-store breakpoints) ---");
    L("  GR_SEM cpu-store count = %llu (%d thread bps)  [>0 would REFUTE GPU authorship]",
      (unsigned long long)c_sem, armed_sem);
    L("  GP_PUT cpu-store count = %llu (%d thread bps)  [KNOWN-POSITIVE: if this is 0 the",
      (unsigned long long)c_gpput, armed_gpput);
    L("         instrument is dead and the GR_SEM zero above is UNINTERPRETABLE]");
    L("  VERDICT-INPUTS: gr_payload_changed=%d gr_ts_changed=%d gr_cpu_stores=%llu known_positive=%llu",
      gr_pre[0] != gr_post[0], (gr_pre[2] != gr_post[2] || gr_pre[3] != gr_post[3]),
      (unsigned long long)c_sem, (unsigned long long)c_gpput);

    /* ---- 5: GP_GET vs GP_PUT vs the semaphore ------------------------------- */
    L("=== ITEM 5: GP_GET vs GP_PUT vs SEMAPHORE, sampled ===");
    L("  USERD polled at 0x%016llx (GP_GET +0x88, GP_PUT +0x8c)", (unsigned long long)g_userd_va);
    L("  %d samples; each carries its OWN timestamp, so a dump at teardown still", g_nsamp);
    L("  reports time correctly, not just order.");
    {
        int n = g_nsamp, printed = 0;
        struct sample last; char p[512]; int fd;
        memset(&last, 0xff, sizeof last);
        snprintf(p, sizeof p, "%s/samples.csv", g_rawdir);
        fd = open(p, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        if (fd >= 0) {
            char hdr[] = "t,gp_get,gp_put,gr_payload,gr_pad,gr_ts_lo,gr_ts_hi,"
                         "ce_payload,ce_pad,ce_ts_lo,ce_ts_hi,"
                         "host_payload,host_pad,host_ts_lo,host_ts_hi\n";
            if (write(fd, hdr, sizeof hdr - 1) < 0) {}
        }
        for (i = 0; i < n; i++) {
            struct sample *s = &g_samp[i];
            char line[320]; int o2;
            if (fd >= 0) {
                o2 = snprintf(line, sizeof line, "%.9f,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u\n",
                    s->t, s->gpget, s->gpput,
                    s->ce.w[0], s->ce.w[1], s->ce.w[2], s->ce.w[3],
                    s->gr.w[0], s->gr.w[1], s->gr.w[2], s->gr.w[3],
                    s->host.w[0], s->host.w[1], s->host.w[2], s->host.w[3]);
                if (write(fd, line, o2) < 0) {}
            }
            if (s->gpget != last.gpget || s->gpput != last.gpput ||
                memcmp(s->chget, last.chget, sizeof s->chget) ||
                memcmp(s->chput, last.chput, sizeof s->chput) ||
                memcmp(s->slot, last.slot, sizeof s->slot) ||
                memcmp(&s->ce, &last.ce, sizeof s->ce)) {
                if (printed < 500) {
                    char cb[512]; int oc = 0, c;
                    for (c = 0; c < g_poll_nuserd; c++)
                        oc += snprintf(cb + oc, sizeof cb - oc, "%u/%u ", s->chget[c], s->chput[c]);
                    L("  T t=%.9f SEM[%08x pad=%08x ts=%08x%08x]", s->t,
                      s->ce.w[0], s->ce.w[1], s->ce.w[3], s->ce.w[2]);
                    L("       ch get/put: %s", cb);
                    oc = 0;
                    for (c = 0; c < NSLOT; c++)
                        oc += snprintf(cb + oc, sizeof cb - oc, "%x ", s->slot[c]);
                    L("       slots +0xf00..+0xff0: %s", cb);
                }
                printed++;
                last = *s;
            }
        }
        L("  %d transition(s); full series -> %s", printed, p);
        if (fd >= 0) close(fd);
    }

    /* ---- post-run ring re-read --------------------------------------------- */
    L("=== PER-CHANNEL CURSORS, before STAGE 3 vs after STAGE 4 ===");
    L("  Any channel whose GP_PUT moved between the two took a submission.");
    for (ci = 0; ci < g_nch; ci++) {
        uint32_t gg = 0, gp = 0;
        int k, base_get = -1, base_put = -1;
        if (!g_ch[ci].userd_cpu) continue;
        rd32(g_ch[ci].userd_cpu + USERD_GP_GET, &gg);
        rd32(g_ch[ci].userd_cpu + USERD_GP_PUT, &gp);
        for (k = 0; k < g_nuc; k++) if (g_uc[k].ch == ci) { base_get = (int)g_uc[k].get1; base_put = (int)g_uc[k].put1; }
        L("  ch[%2d] USERD 0x%llx GP_GET %d->%u GP_PUT %d->%u%s", ci,
          (unsigned long long)g_ch[ci].userd_cpu, base_get, gg, base_put, gp,
          (base_put >= 0 && (uint32_t)base_put != gp) ? "   <== took work" : "");
    }
    L("=== POST-RUN: the ring after the watched copies ===");
    if (ring_va) {
        uint64_t rb = ring_va & ~0xfffull;
        uint32_t e;
        hexdump_to("ring_post.bin", rb, 8192);
        for (e = 0; e < 512; e++) {
            uint32_t e0, e1;
            if (!rd32(rb + 8ull * e, &e0) || !rd32(rb + 8ull * e + 4, &e1)) break;
            if (!e0 && !e1) continue;
            L("    gpe[%3u] = %08x %08x -> 0x%010llx len=%u dw", e, e0, e1,
              (unsigned long long)gpe_addr(e0, e1), gpe_len(e1));
        }
        {   /* decode the LAST non-empty entry: that is the DtoH submission */
            uint32_t e0 = 0, e1 = 0, last = 0;
            for (e = 0; e < 512; e++) {
                uint32_t a0, a1;
                if (!rd32(rb + 8ull * e, &a0) || !rd32(rb + 8ull * e + 4, &a1)) break;
                if (a0 || a1) { e0 = a0; e1 = a1; last = e; }
            }
            if (e0 || e1) {
                struct pbfacts f; memset(&f, 0, sizeof f);
                L("  --- decoding the LAST entry (#%u), i.e. the most recent submission ---", last);
                hexdump_to("pushbuffer_last.bin", gpe_addr(e0, e1), (size_t)gpe_len(e1) * 4);
                decode_pb(gpe_addr(e0, e1), gpe_len(e1), &f, 1);
                L("  LAST-ENTRY FACTS: i2m_launch=%d inline_dw=%d first_inline=0x%08x "
                  "ce_launch=%d gr_report_sem=%d qmd=%d",
                  f.n_i2m_launch, f.n_inline_data, f.inline_first, f.n_launch_dma,
                  f.n_report_sem, f.n_qmd);
                if (f.gr_sem_va) L("  LAST-ENTRY GR sem VA=0x%016llx payload=0x%08x",
                                   (unsigned long long)f.gr_sem_va, f.gr_sem_payload);
                if (f.i2m_off_out) L("  LAST-ENTRY I2M out=0x%016llx len=%u",
                                     (unsigned long long)f.i2m_off_out, f.i2m_line_len);
            }
        }
    }

    L("=== SUMMARY ===");
    L("  channels_decoded=%d rm_allocs=%d userd=%s ring=%s sem=0x%llx",
      g_nch, g_nobj, g_userd_va ? "FOUND" : "NOT FOUND", ring_va ? "FOUND" : "NOT FOUND",
      (unsigned long long)g_sem_va);
    L("  DONE");
    return 0;
done:
    L("=== ABORTED ===");
    return 1;
}
