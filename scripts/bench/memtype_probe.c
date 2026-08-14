/*
 * memtype_probe.c — the EFFECTIVE memory type, measured IN THE GUEST.
 * =====================================================================================
 *
 * ## Why this exists, and what it is the other half of
 *
 * `crates/kayfabe-linux-raw/src/memtype.rs` reads back what the kernel did to a *host*
 * userspace mapping. Its own header states the limit it cannot cross:
 *
 *   > Every instrument here observes decider 1, the host userspace PTE. [...] On Intel, EPT
 *   > sets IPAT for a normal-RAM backing and the guest PTE is ignored; AMD NPT has no IPAT
 *   > and honours the guest PTE. [...] Nothing in userspace can read a guest's effective
 *   > type. A consumer that needs that answer must measure it IN THE GUEST.
 *
 * This program is that consumer. It is a single C file with no dependencies so it can be
 * compiled inside the bench guest the way `cup3.c` and `e2_doorbell_poke.c` already are
 * (`gssh_nv` + `gcc`), on a guest that has no Rust toolchain.
 *
 * ## ★★★ The finding that shapes the design: in the guest the three instruments are NOT
 * ## co-equal, and the host module's framing does not port unchanged
 *
 * The host module has three instruments and says they are "deliberately three, because each
 * one alone has a way of being green while wrong". True there. In the guest the relationship
 * is *asymmetric*, and reading it as corroboration would be a mistake:
 *
 *   - `/proc/iomem` and `/sys/kernel/debug/x86/pat_memtype_list`, read INSIDE the guest,
 *     observe **decider 3 only** — the guest kernel's own request and its own bookkeeping.
 *     They are structurally blind to deciders 1 and 2 exactly as the host module is blind to
 *     2 and 3. Two blind instruments do not add up to sight.
 *   - The **timing witness is the only instrument that observes the COMBINATION.** The CPU
 *     resolves all three deciders in hardware; a load's latency is the resolution.
 *
 * ⇒ So the categorical half is not here to corroborate the timing half. It is here to
 *   **attribute** it: the timing says *what the type is*, and the disagreement between the
 *   two says *which decider produced it*. The pair is the measurement; neither alone is.
 *
 *   | guest record (iomem/PAT) | timed verdict | reading                                       |
 *   |--------------------------|---------------|-----------------------------------------------|
 *   | System RAM / write-back  | cached        | consistent — nothing overrode the guest       |
 *   | System RAM / write-back  | uncached      | ★★★ decider 1 or 2 forced UC. The guest thinks|
 *   |                          |               | it holds cached RAM and every access is a bus |
 *   |                          |               | transaction. THIS IS A REAL PROBLEM.          |
 *   | uncached / uncached-minus| cached        | ★★★ the guest asked for UC and did not get it.|
 *   |                          |               | On x86 that is IPAT (or an equivalent host    |
 *   |                          |               | override) DISCARDING the guest's choice — the |
 *   |                          |               | Intel side of the asymmetry, measured. A      |
 *   |                          |               | register poll on such a page can hang.        |
 *   | uncached / uncached-minus| uncached      | consistent — the guest's choice was honoured  |
 *
 * ★★ That fourth-vs-third row is the whole point. "Map write-back and let the guest decide"
 * is a *proposal about decider 3*; whether decider 3 is even consulted is what this table
 * answers, and it answers it per-host, at run time, instead of per-vendor from a rumour.
 *
 * ## ★★★★★ The instrument's own failure mode, found here and controlled for
 *
 * A read-bandwidth ratio cannot tell **uncached** from **cache-missing**. A strided pass over
 * a buffer far larger than the LLC is write-back memory and still times 50-100x slower than a
 * resident one — which is squarely past `UNCACHED_RATIO_FLOOR`. The host module's constants
 * are stated as a property of the *memory type*; they are in fact a property of the
 * comparison, and only hold when the subject and the reference have the **same footprint and
 * the same stride**.
 *
 * ⇒ This probe therefore makes footprint an explicit parameter of every comparison and
 *   measures a fresh anonymous reference **at the subject's own footprint**. And it runs the
 *   deliberately-mismatched comparison as a standing control (`control:footprint-mismatch`)
 *   so the failure mode is *exhibited on every run* rather than trusted not to occur.
 *
 * ## ★★ A known-positive is mandatory, and its absence VOIDS the run
 *
 * A probe that reports "everything is write-back" without ever having reported anything else
 * is not a measurement. So this program refuses to certify a run in which no region was
 * reported uncached: **exit status 2 = VOID**, distinct from 0 = measured and 3 = could not
 * run at all. The known-positive is a mapping whose type is known to differ *by construction*:
 *
 *   1. `/dev/mem` at a physical address `/proc/iomem` does NOT call `System RAM`. x86's
 *      `phys_mem_access_prot()` gives it `pgprot_noncached` (UC-). ★ On most machines that
 *      range is ordinary DRAM behind a firmware label, which makes it the ideal control: the
 *      physical *medium* is the same as guest RAM and only the *attribute* differs, so the
 *      ratio isolates the attribute. This is also the exact shape of the C artifact's #111.
 *   2. Failing that, a PCI BAR through `/sys/bus/pci/devices/<BDF>/resourceN`, which the kernel
 *      maps uncached. ⊘ Refused by the kernel while a driver holds the region, so the NVIDIA
 *      BARs need the module unloaded — see `e2_doorbell_witness.sh`, which already does this.
 *
 * ## ⊘ Two ways this program is lied to, both encoded as refusals rather than as zeros
 *
 *   - **`/proc/iomem` is REDACTED for unprivileged readers** — every address reads as
 *     `00000000-00000000`. Parsed naively that says "nothing is System RAM", which reads as
 *     "the downgrade applies everywhere". Detected and refused by name.
 *   - **`/proc/self/pagemap` zeroes the PFN** without `CAP_SYS_ADMIN`, while still setting
 *     the present bit. Parsed naively that says "guest-physical address 0". Detected and
 *     refused by name.
 *
 * ## ⊘ Not x86
 *
 * `pat_memtype_list` is x86-only, and on arm64 the question is not PAT at all: Normal vs
 * Device is a difference in *kind*, mismatched aliases are architecturally UNPREDICTABLE, and
 * a bulk copy that is merely slow on x86 can fault there. On a non-x86 guest this program
 * reports the categorical instruments as UNAVAILABLE and runs the timing half only — it does
 * not fall back to assuming write-back, which is the failure it exists to prevent.
 *
 * ## Build
 *   gcc -O2 -Wall -Wextra -o memtype_probe memtype_probe.c
 * Static, for staging into a guest with a different libc:
 *   gcc -O2 -static -o memtype_probe memtype_probe.c
 *
 * Run as root (needs /proc/iomem unredacted, pagemap PFNs, debugfs, /dev/mem).
 */

#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <setjmp.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

/* ─────────────────────────────────────────────────────────────────────────────────────
 * The constants, carried over from crates/kayfabe-linux-raw/src/memtype.rs so the two
 * halves of this instrument grade on the same numbers. Each one's justification is in that
 * file's rustdoc and is NOT repeated here — a second copy of a rationale is a second thing
 * to keep true.
 * ───────────────────────────────────────────────────────────────────────────────────── */
#define UNCACHED_RATIO_FLOOR      10.0
#define CACHED_RATIO_CEILING       2.0
#define REFERENCE_SPREAD_CEILING   2.0
#define SCHEDULER_AVERAGING_NS     5000000ULL   /* 5 ms */
#define FIRST_PROBE_READS          256ULL
#define MAX_PROBE_READS            (1ULL << 26)

/* ★ Footprint and stride are parameters of the COMPARISON, not of the memory. See the
 * header: holding them equal between subject and reference is what makes the ratio a
 * statement about the memory type rather than about cache residency. */
#define DEFAULT_FOOTPRINT          4096u
#define DEFAULT_STRIDE               64u
/* ⊘ A device aperture is read with the narrowest footprint that still exercises the type:
 * one cache line, re-read. Widening it means touching registers nobody chose to touch. */
#define DEVICE_FOOTPRINT             64u

#define IOMEM_PATH  "/proc/iomem"
#define PAT_PATH    "/sys/kernel/debug/x86/pat_memtype_list"
#define PAGEMAP     "/proc/self/pagemap"
#define SYSTEM_RAM  "System RAM"

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Small helpers
 * ───────────────────────────────────────────────────────────────────────────────────── */

static int g_verbose = 0;

/* The gate marker, deliberately on stderr and unbuffered, matching the `MEMTYPE-GATE:`
 * convention `crates/kayfabe-linux-raw/tests/effective_memtype.rs` established after
 * discovering its own skip messages had been swallowed on every passing run. */
static void gate(const char *fmt, ...)
{
    va_list ap;
    fputs("MEMTYPE-GATE: ", stderr);
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fputc('\n', stderr);
    fflush(stderr);
}

static uint64_t now_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

/* Read a whole /proc or /sys file. They report st_size 0, so a stat-then-read is wrong and
 * would silently produce an empty buffer — which parses as "no entries", which reads as
 * benign. Loop instead. Returns NULL and sets *why on failure. */
static char *slurp(const char *path, const char **why)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0) { *why = strerror(errno); return NULL; }
    size_t cap = 1 << 16, len = 0;
    char *buf = malloc(cap);
    if (!buf) { close(fd); *why = "out of memory"; return NULL; }
    for (;;) {
        if (len + 4096 > cap) {
            cap *= 2;
            char *n = realloc(buf, cap);
            if (!n) { free(buf); close(fd); *why = "out of memory"; return NULL; }
            buf = n;
        }
        ssize_t r = read(fd, buf + len, cap - len - 1);
        if (r < 0) { free(buf); close(fd); *why = strerror(errno); return NULL; }
        if (r == 0) break;
        len += (size_t)r;
    }
    buf[len] = '\0';
    close(fd);
    return buf;
}

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Instrument 1 — /proc/iomem, read INSIDE THE GUEST
 *
 * Same parse as `memtype::classify_physical`, with the same two traps handled: the ends are
 * INCLUSIVE (one off silently mis-answers the last page of every region, which is the page a
 * window is most likely to end on), and the predicate is `all`, not `any` — a range that is
 * half RAM is a range the downgrade applies to.
 * ───────────────────────────────────────────────────────────────────────────────────── */

typedef struct { uint64_t lo, hi; } ival_t;

typedef struct {
    int      available;      /* the file was readable                                  */
    int      redacted;       /* ★ every address was zero — unprivileged reader          */
    int      system_ram;     /* every byte of the queried range is `System RAM`         */
    char     label[96];      /* deepest label covering the first byte                   */
} physclass_t;

static int cmp_ival(const void *a, const void *b)
{
    uint64_t x = ((const ival_t *)a)->lo, y = ((const ival_t *)b)->lo;
    return (x > y) - (x < y);
}

static int covered_entirely(uint64_t base, uint64_t end, ival_t *v, size_t n)
{
    qsort(v, n, sizeof v[0], cmp_ival);
    uint64_t at = base;
    for (size_t i = 0; i < n; i++) {
        if (v[i].lo > at) return 0;
        if (v[i].hi > at) at = v[i].hi;
        if (at >= end) return 1;
    }
    return at >= end;
}

static void classify_physical(const char *iomem, uint64_t base, uint64_t len, physclass_t *out)
{
    memset(out, 0, sizeof *out);
    if (!iomem || len == 0) return;
    out->available = 1;

    uint64_t end = base + len;
    size_t   cap = 256, n = 0;
    ival_t  *ram = malloc(cap * sizeof *ram);
    if (!ram) return;

    size_t   label_depth = (size_t)-1;
    int      nonzero_seen = 0, lines = 0;

    const char *p = iomem;
    while (*p) {
        const char *nl = strchr(p, '\n');
        size_t plen = nl ? (size_t)(nl - p) : strlen(p);
        char line[512];
        if (plen >= sizeof line) plen = sizeof line - 1;
        memcpy(line, p, plen);
        line[plen] = '\0';
        p = nl ? nl + 1 : p + strlen(p);

        size_t depth = strspn(line, " ");
        char *body = line + depth;
        char *sep  = strstr(body, " : ");
        if (!sep) continue;
        *sep = '\0';
        const char *name = sep + 3;

        char *dash = strchr(body, '-');
        if (!dash) continue;
        *dash = '\0';
        uint64_t lo = strtoull(body, NULL, 16);
        uint64_t hi = strtoull(dash + 1, NULL, 16);
        lines++;
        if (lo || hi) nonzero_seen = 1;
        hi += 1;                       /* /proc/iomem ends are INCLUSIVE */

        if (strcmp(name, SYSTEM_RAM) == 0) {
            if (n == cap) {
                cap *= 2;
                ival_t *g = realloc(ram, cap * sizeof *ram);
                if (!g) { free(ram); return; }
                ram = g;
            }
            ram[n].lo = lo; ram[n].hi = hi; n++;
        }
        if (lo <= base && base < hi && (out->label[0] == '\0' || depth >= label_depth)) {
            snprintf(out->label, sizeof out->label, "%s", name);
            label_depth = depth;
        }
    }

    /* ⊘ THE REDACTION. An unprivileged reader gets `00000000-00000000` on every line. The
     * naive parse then says "no System RAM anywhere", which reads as "the write-back
     * downgrade applies to everything" — a confident, wrong, permissive-looking answer. */
    if (lines > 0 && !nonzero_seen) {
        out->redacted = 1;
        out->available = 0;
        free(ram);
        return;
    }
    out->system_ram = covered_entirely(base, end, ram, n);
    free(ram);
}

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Instrument 2 — the guest kernel's own PAT bookkeeping
 *
 * Same parse and same narrowest-wins tie-break as `memtype::recorded_memtype`: entries NEST,
 * and file order is arbitrary, so width is the only sound tie-break.
 * ───────────────────────────────────────────────────────────────────────────────────── */

typedef enum {
    MT_NONE = 0, MT_WB, MT_WT, MT_WC, MT_WP, MT_UC_MINUS, MT_UC
} memtype_t;

static const char *memtype_name(memtype_t t)
{
    switch (t) {
    case MT_WB:       return "write-back";
    case MT_WT:       return "write-through";
    case MT_WC:       return "write-combining";
    case MT_WP:       return "write-protected";
    case MT_UC_MINUS: return "uncached-minus";
    case MT_UC:       return "uncached";
    default:          return "(untracked)";
    }
}

/* Is this a *cached* type? The coarsening is one-way and deliberate, as in
 * `KernelMemtype::as_cache_policy`: write-through and write-protected are cached; both
 * uncached spellings are not; write-combining is uncached for LOADS, which is the only
 * thing a read-side timing witness can see. */
static int memtype_is_cached(memtype_t t) { return t == MT_WB || t == MT_WT || t == MT_WP; }

static memtype_t memtype_parse(const char *s)
{
    while (*s == ' ') s++;
    if (!strncmp(s, "write-back", 10))       return MT_WB;
    if (!strncmp(s, "write-through", 13))    return MT_WT;
    if (!strncmp(s, "write-combining", 15))  return MT_WC;
    if (!strncmp(s, "write-protected", 15))  return MT_WP;
    if (!strncmp(s, "uncached-minus", 14))   return MT_UC_MINUS;   /* BEFORE "uncached" */
    if (!strncmp(s, "uncached", 8))          return MT_UC;
    return MT_NONE;
}

static memtype_t recorded_memtype(const char *pat, uint64_t phys)
{
    if (!pat) return MT_NONE;
    memtype_t best = MT_NONE;
    uint64_t  best_w = UINT64_MAX;
    const char *p = pat;
    while (*p) {
        const char *nl = strchr(p, '\n');
        size_t plen = nl ? (size_t)(nl - p) : strlen(p);
        char line[256];
        if (plen >= sizeof line) plen = sizeof line - 1;
        memcpy(line, p, plen);
        line[plen] = '\0';
        p = nl ? nl + 1 : p + strlen(p);

        char *b = line;
        while (*b == ' ') b++;
        if (strncmp(b, "PAT: [mem ", 10)) continue;   /* header, and anything unknown */
        b += 10;
        char *close = strchr(b, ']');
        if (!close) continue;
        *close = '\0';
        char *dash = strchr(b, '-');
        if (!dash) continue;
        *dash = '\0';
        uint64_t lo = strtoull(b, NULL, 0);
        uint64_t hi = strtoull(dash + 1, NULL, 0);
        memtype_t t = memtype_parse(close + 1);
        if (t == MT_NONE) continue;
        if (lo <= phys && phys < hi && (hi - lo) < best_w) { best_w = hi - lo; best = t; }
    }
    return best;
}

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Guest-virtual -> guest-physical, so the two categorical instruments have a key at all
 *
 * ⊘ The host module takes `phys_base` as a parameter and never says where it came from. In
 * the guest we must derive it, and the derivation has its own way of lying.
 * ───────────────────────────────────────────────────────────────────────────────────── */

typedef enum { GPA_OK = 0, GPA_UNAVAILABLE, GPA_REDACTED, GPA_ABSENT } gpa_status_t;

static gpa_status_t gva_to_gpa(const void *va, uint64_t *out, const char **why)
{
    static long ps = 0;
    if (!ps) ps = sysconf(_SC_PAGESIZE);
    int fd = open(PAGEMAP, O_RDONLY);
    if (fd < 0) { *why = strerror(errno); return GPA_UNAVAILABLE; }
    uint64_t idx = (uint64_t)(uintptr_t)va / (uint64_t)ps;
    uint64_t ent = 0;
    ssize_t r = pread(fd, &ent, sizeof ent, (off_t)(idx * sizeof ent));
    close(fd);
    if (r != (ssize_t)sizeof ent) { *why = "short read"; return GPA_UNAVAILABLE; }
    if (!(ent >> 63)) { *why = "page not present — touch it first"; return GPA_ABSENT; }
    uint64_t pfn = ent & ((1ULL << 55) - 1);
    /* ⊘ THE REDACTION. Without CAP_SYS_ADMIN the kernel clears the PFN and LEAVES THE
     * PRESENT BIT SET. Read naively that is guest-physical address 0 — a real-looking
     * address that /proc/iomem will happily classify. */
    if (pfn == 0) { *why = "PFN redacted (needs CAP_SYS_ADMIN)"; return GPA_REDACTED; }
    *out = pfn * (uint64_t)ps;
    return GPA_OK;
}

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Instrument 3 — the mapping itself, timed. The ONLY one that sees the combination.
 * ───────────────────────────────────────────────────────────────────────────────────── */

typedef enum { V_CACHED, V_UNCACHED_CLASS, V_INCONCL_BAND, V_INCONCL_REF, V_NOT_RUN } verdict_t;

static const char *verdict_name(verdict_t v)
{
    switch (v) {
    case V_CACHED:          return "cached";
    case V_UNCACHED_CLASS:  return "uncached-class";
    case V_INCONCL_BAND:    return "inconclusive(in-band)";
    case V_INCONCL_REF:     return "inconclusive(reference-unstable)";
    default:                return "not-run";
    }
}

/* A fault while reading a device aperture must be a report, not a core dump. */
static sigjmp_buf g_fault;
static volatile sig_atomic_t g_faulted;
static void fault_handler(int sig) { (void)sig; g_faulted = 1; siglongjmp(g_fault, 1); }

static volatile uint64_t g_sink;

/* One timed pass. Sequential, one 32-bit load per `stride` bytes, wrapping at `footprint`.
 * ★ `footprint` and `stride` are the same for subject and reference by construction — that
 * is what makes the ratio a statement about the memory type. */
static double timed_pass(volatile const unsigned char *base, unsigned footprint,
                         unsigned stride, uint64_t reads)
{
    uint64_t acc = 0;
    unsigned off = 0;
    uint64_t t0 = now_ns();
    for (uint64_t i = 0; i < reads; i++) {
        acc += *(volatile const uint32_t *)(base + off);
        off += stride;
        if (off >= footprint) off = 0;
    }
    uint64_t dt = now_ns() - t0;
    g_sink += acc;
    return (double)dt / (double)reads;
}

/* Grow the read count by doubling until the pass covers SCHEDULER_AVERAGING_NS. A fixed read
 * count is a per-box constant in disguise; this reaches the DURATION instead, so a host ten
 * times faster performs ten times as many reads and gets the same guarantee. */
static double measure_over(volatile const unsigned char *base, unsigned footprint,
                           unsigned stride, uint64_t *reads_out)
{
    uint64_t reads = FIRST_PROBE_READS;
    for (;;) {
        uint64_t t0 = now_ns();
        double ns = timed_pass(base, footprint, stride, reads);
        uint64_t elapsed = now_ns() - t0;
        if (elapsed >= SCHEDULER_AVERAGING_NS || reads >= MAX_PROBE_READS) {
            if (reads_out) *reads_out = reads;
            return ns;
        }
        reads *= 2;
    }
}

/* Two passes over anonymous memory of the SUBJECT'S OWN SHAPE, plus the check that they
 * agree. The reference is the whole instrument; an unstable one settles nothing. */
typedef struct { double first, second; int ok; } reference_t;

static reference_t measure_reference(unsigned footprint, unsigned stride)
{
    reference_t r = { 0, 0, 0 };
    size_t len = footprint < 4096u ? 4096u : footprint;
    unsigned char *buf = mmap(NULL, len, PROT_READ | PROT_WRITE,
                              MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (buf == MAP_FAILED) return r;
    memset(buf, 0x5a, len);                     /* fault it in; anon is always write-back */
    r.first  = measure_over(buf, footprint, stride, NULL);
    r.second = measure_over(buf, footprint, stride, NULL);
    munmap(buf, len);
    r.ok = 1;
    return r;
}

static double ref_spread(reference_t r)
{
    double lo = r.first < r.second ? r.first : r.second;
    double hi = r.first < r.second ? r.second : r.first;
    return lo > 0.0 ? hi / lo : 1.0 / 0.0;
}

/* The DENOMINATOR is the SLOWER pass — the conservative choice. A larger denominator can
 * only ever make this instrument less willing to call something uncached, and calling a
 * mapping uncached when it is not is the failure that would let a #111 pass a green run. */
static double ref_ns(reference_t r) { return r.first > r.second ? r.first : r.second; }

static verdict_t judge(double subject_ns, reference_t r, double *ratio_out)
{
    if (!r.ok || ref_spread(r) > REFERENCE_SPREAD_CEILING) {
        if (ratio_out) *ratio_out = 0.0;
        return V_INCONCL_REF;
    }
    double ratio = subject_ns / ref_ns(r);
    if (ratio_out) *ratio_out = ratio;
    if (ratio >= UNCACHED_RATIO_FLOOR) return V_UNCACHED_CLASS;
    if (ratio <= CACHED_RATIO_CEILING) return V_CACHED;
    return V_INCONCL_BAND;
}

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Regions
 * ───────────────────────────────────────────────────────────────────────────────────── */

typedef enum { ROLE_SUBJECT, ROLE_CONTROL_WB, ROLE_CONTROL_MISMATCH, ROLE_KNOWN_POSITIVE }
        role_t;

typedef struct {
    char        name[64];
    role_t      role;
    unsigned    footprint, stride;
    /* ★★★ The reference's footprint, normally EQUAL to the subject's — that equality is
     * what makes the ratio a statement about the memory type. `control-mismatch` is the one
     * region that deliberately breaks it, so the failure mode is exhibited on every run. */
    unsigned    ref_footprint;
    void       *va;
    size_t      map_len;
    int         mapped;
    uint64_t    gpa;
    int         gpa_known;
    /* ⊘ A `/dev/mem` or sysfs-`resource` mapping's guest-physical address is DECLARED by the
     * caller (it is the mmap offset / the BAR base), so pagemap is never consulted for one.
     * Measured on this box: pagemap answers `not present` for a `/dev/mem` VM_PFNMAP vma, and
     * deriving the address the same way as for anonymous memory would have reported "gpa
     * unknown" for exactly the regions the probe exists to look at. */
    int         gpa_declared;
    char        gpa_why[64];
    physclass_t phys;
    memtype_t   recorded;
    verdict_t   verdict;
    double      ratio, subject_ns;
    uint64_t    reads;
    char        note[320];
} region_t;

#define MAX_REGIONS 32
static region_t g_regions[MAX_REGIONS];
static int      g_nregions;

static region_t *new_region(const char *name, role_t role, unsigned footprint, unsigned stride)
{
    if (g_nregions >= MAX_REGIONS) return NULL;
    region_t *r = &g_regions[g_nregions++];
    memset(r, 0, sizeof *r);
    snprintf(r->name, sizeof r->name, "%s", name);
    r->role = role;
    r->footprint = footprint;
    r->ref_footprint = footprint;      /* equal by default; see the field's comment */
    r->stride = stride;
    r->verdict = V_NOT_RUN;
    return r;
}

/* Run the two categorical instruments and the timing witness over one already-mapped
 * region. `iomem`/`pat` are the file contents, slurped once. */
static void measure_region(region_t *r, const char *iomem, const char *pat)
{
    if (!r->mapped) return;

    /* Guest-virtual -> guest-physical. Touch first: an absent page has no PFN. */
    const char *why = NULL;
    volatile const unsigned char *p = r->va;
    g_faulted = 0;
    if (sigsetjmp(g_fault, 1) == 0) {
        g_sink += *(volatile const uint32_t *)p;
    } else {
        snprintf(r->note, sizeof r->note, "SIGBUS/SIGSEGV on first read — region refused");
        return;
    }
    if (r->gpa_declared) {
        r->gpa_known = 1;
    } else {
        gpa_status_t gs = gva_to_gpa(r->va, &r->gpa, &why);
        r->gpa_known = (gs == GPA_OK);
        if (!r->gpa_known)
            snprintf(r->gpa_why, sizeof r->gpa_why, "%s", why ? why : "unknown");
    }

    if (r->gpa_known) {
        classify_physical(iomem, r->gpa, r->footprint, &r->phys);
        /* ⊘ RE-READ THE PAT LIST HERE, not once at startup. It records RESERVATIONS, and the
         * reservation this probe is asking about is the one THIS PROCESS just created by
         * mapping the region. A copy slurped before the mapping existed can only ever answer
         * `untracked` — which reads as "ordinary memory, so write-back", the permissive
         * answer, for precisely the regions the probe exists to look at. Measured: the
         * `/dev/mem` known-positive reported `untracked` from a startup-slurped copy while
         * timing at 126.8x. `tests/effective_memtype.rs` reads it "while the mapping is
         * live" for the same reason. */
        const char *pw = NULL;
        char *live = slurp(PAT_PATH, &pw);
        r->recorded = recorded_memtype(live ? live : pat, r->gpa);
        free(live);
    }

    reference_t ref = measure_reference(r->ref_footprint, r->stride);
    g_faulted = 0;
    if (sigsetjmp(g_fault, 1) == 0) {
        r->subject_ns = measure_over(p, r->footprint, r->stride, &r->reads);
        r->verdict = judge(r->subject_ns, ref, &r->ratio);
    } else {
        snprintf(r->note, sizeof r->note, "SIGBUS/SIGSEGV during the timed pass");
        r->verdict = V_NOT_RUN;
    }
    if (g_verbose)
        fprintf(stderr, "  [%s] ref %.2f/%.2f ns spread %.2f subject %.2f ns over %" PRIu64
                        " reads\n",
                r->name, ref.first, ref.second, ref_spread(ref), r->subject_ns, r->reads);
}

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Region constructors
 * ───────────────────────────────────────────────────────────────────────────────────── */

static int add_anon(const char *name, role_t role, size_t len, unsigned footprint,
                    unsigned stride)
{
    region_t *r = new_region(name, role, footprint, stride);
    if (!r) return 0;
    void *m = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (m == MAP_FAILED) {
        snprintf(r->note, sizeof r->note, "mmap failed: %s", strerror(errno));
        return 0;
    }
    memset(m, 0x5a, len);
    r->va = m; r->map_len = len; r->mapped = 1;
    return 1;
}

/* ★ The known-positive: /dev/mem at a physical address /proc/iomem does NOT call
 * `System RAM`. `phys_mem_access_prot()` gives it pgprot_noncached (UC-), so the mapping's
 * type differs from anonymous RAM BY CONSTRUCTION — and on most machines the backing is
 * ordinary DRAM behind a firmware label, so the ratio isolates the ATTRIBUTE rather than the
 * medium. That is precisely the #111 shape. */
static int pick_devmem_candidate(const char *iomem, uint64_t *base_out, char *label_out,
                                 size_t label_cap)
{
    const char *p = iomem;
    while (p && *p) {
        const char *nl = strchr(p, '\n');
        size_t plen = nl ? (size_t)(nl - p) : strlen(p);
        char line[512];
        if (plen >= sizeof line) plen = sizeof line - 1;
        memcpy(line, p, plen);
        line[plen] = '\0';
        p = nl ? nl + 1 : p + strlen(p);

        size_t depth = strspn(line, " ");
        if (depth != 0) continue;                 /* top-level regions only */
        char *body = line;
        char *sep = strstr(body, " : ");
        if (!sep) continue;
        *sep = '\0';
        const char *name = sep + 3;
        char *dash = strchr(body, '-');
        if (!dash) continue;
        *dash = '\0';
        uint64_t lo = strtoull(body, NULL, 16);
        uint64_t hi = strtoull(dash + 1, NULL, 16) + 1;

        if (!strcmp(name, SYSTEM_RAM)) continue;
        if (lo < 0x100000ULL) continue;           /* stay out of the legacy hole */
        if (hi - lo < 0x2000ULL) continue;        /* need at least two pages      */
        /* Reserved and ACPI NVS are inert for READS and are not a driver's registers. */
        if (strcmp(name, "Reserved") && strncmp(name, "ACPI", 4)) continue;
        *base_out = lo;
        snprintf(label_out, label_cap, "%s", name);
        return 1;
    }
    return 0;
}

static int add_devmem(const char *name, role_t role, uint64_t phys, unsigned footprint,
                      unsigned stride, const char *label)
{
    region_t *r = new_region(name, role, footprint, stride);
    if (!r) return 0;
    /* ★★★★★ `O_DSYNC`, AND IT IS THE WHOLE KNOWN-POSITIVE. The first version of this probe
     * opened /dev/mem `O_RDONLY` and asserted, from the fact that the range is not
     * `System RAM`, that the mapping "is UC- by construction". It is not. `drivers/char/mem.c`
     * decides with `uncached_access()`, and on x86 that function's ENTIRE test is:
     *
     *     if (file->f_flags & O_DSYNC) return 1;
     *     return addr >= __pa(high_memory);
     *
     * ⇒ below `high_memory` and without `O_DSYNC` you get an ordinary WRITE-BACK mapping,
     * whatever `/proc/iomem` calls the range. Measured on this box before the fix: the
     * "known-positive" timed at **1.0x**, i.e. cached, and the probe's own VOID gate refused
     * to certify the run. ★ That is the gate doing its job on its author — the exact failure
     * the brief names ("a probe that reports everything is write-back without ever having
     * reported anything else is not a measurement"), caught by construction rather than by
     * someone noticing. A "known" positive that was reasoned rather than watched is a
     * hypothesis. */
    int fd = open("/dev/mem", O_RDONLY | O_DSYNC);
    if (fd < 0) {
        snprintf(r->note, sizeof r->note, "/dev/mem: %s", strerror(errno));
        return 0;
    }
    size_t len = footprint < 4096u ? 4096u : footprint;
    void *m = mmap(NULL, len, PROT_READ, MAP_SHARED, fd, (off_t)phys);
    close(fd);
    if (m == MAP_FAILED) {
        snprintf(r->note, sizeof r->note, "mmap /dev/mem @%#" PRIx64 ": %s", phys,
                 strerror(errno));
        return 0;
    }
    r->va = m; r->map_len = len; r->mapped = 1;
    r->gpa = phys; r->gpa_declared = 1;
    snprintf(r->note, sizeof r->note, "/dev/mem O_DSYNC @%#" PRIx64 " (%s)", phys, label);
    return 1;
}

/* A PCI BAR through sysfs. ⊘ The kernel REFUSES this while a driver holds the region, so on
 * a guest with the NVIDIA module loaded the NVIDIA BARs answer EBUSY — which is a refusal to
 * report, never a report of "fine". `e2_doorbell_witness.sh` unloads the module first. */
static int add_pci_bar(const char *bdf, int bar, unsigned footprint, unsigned stride)
{
    char name[64];
    snprintf(name, sizeof name, "pci:%s:bar%d", bdf, bar);
    region_t *r = new_region(name, ROLE_SUBJECT, footprint, stride);
    if (!r) return 0;
    char path[256];
    snprintf(path, sizeof path, "/sys/bus/pci/devices/%s/resource%d", bdf, bar);
    int fd = open(path, O_RDWR);
    if (fd < 0) fd = open(path, O_RDONLY);
    if (fd < 0) {
        snprintf(r->note, sizeof r->note, "%s: %s", path, strerror(errno));
        return 0;
    }
    size_t len = footprint < 4096u ? 4096u : footprint;
    void *m = mmap(NULL, len, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (m == MAP_FAILED) {
        snprintf(r->note, sizeof r->note, "mmap %s: %s%s", path, strerror(errno),
                 errno == EBUSY ? " (a driver holds this BAR — unbind it first)" : "");
        return 0;
    }
    r->va = m; r->map_len = len; r->mapped = 1;
    /* The BAR's guest-physical base is DECLARED by `resource`, line `bar`, field 1. */
    char rpath[192];
    snprintf(rpath, sizeof rpath, "/sys/bus/pci/devices/%s/resource", bdf);
    const char *w = NULL;
    char *res = slurp(rpath, &w);
    if (res) {
        char *line = res;
        for (int i = 0; line && i < bar; i++) { line = strchr(line, '\n'); if (line) line++; }
        if (line) { r->gpa = strtoull(line, NULL, 0); r->gpa_declared = (r->gpa != 0); }
        free(res);
    }
    snprintf(r->note, sizeof r->note, "%s", path);
    return 1;
}

/* Find the first PCI device with the given vendor id. Returns 1 and fills `bdf`. */
static int find_pci_vendor(const char *vendor_hex, char *bdf, size_t cap)
{
    DIR *d = opendir("/sys/bus/pci/devices");
    if (!d) return 0;
    struct dirent *e;
    int found = 0;
    while (!found && (e = readdir(d))) {
        if (e->d_name[0] == '.') continue;
        char p[320], buf[32];
        snprintf(p, sizeof p, "/sys/bus/pci/devices/%s/vendor", e->d_name);
        int fd = open(p, O_RDONLY);
        if (fd < 0) continue;
        ssize_t n = read(fd, buf, sizeof buf - 1);
        close(fd);
        if (n <= 0) continue;
        buf[n] = '\0';
        char *nl2 = strchr(buf, '\n'); if (nl2) *nl2 = '\0';
        if (!strcasecmp(buf, vendor_hex) && strlen(e->d_name) < cap) {
            memcpy(bdf, e->d_name, strlen(e->d_name) + 1);
            found = 1;
        }
    }
    closedir(d);
    return found;
}

/* ─────────────────────────────────────────────────────────────────────────────────────
 * Report
 * ───────────────────────────────────────────────────────────────────────────────────── */

static const char *role_name(role_t r)
{
    switch (r) {
    case ROLE_CONTROL_WB:       return "control:wb";
    case ROLE_CONTROL_MISMATCH: return "control:footprint-mismatch";
    case ROLE_KNOWN_POSITIVE:   return "known-positive";
    default:                    return "subject";
    }
}

/* ★★★ The attribution table from this file's header, applied. This is the sentence the
 * probe exists to emit: not "the type is X" but "the type is X and decider N produced it". */
static const char *attribute(const region_t *r)
{
    if (r->verdict == V_NOT_RUN)      return "no verdict";
    if (r->verdict == V_INCONCL_REF)  return "reference unstable — nothing learned";
    if (r->verdict == V_INCONCL_BAND) return "in the band — this instrument does not claim to discriminate here";
    if (!r->gpa_known || !r->phys.available)
        return r->verdict == V_UNCACHED_CLASS
                   ? "uncached, and the guest's own record is UNREADABLE so nothing attributes it"
                   : "cached, and the guest's own record is UNREADABLE so nothing attributes it";

    int guest_says_cached = (r->recorded != MT_NONE)
                                ? memtype_is_cached(r->recorded)
                                : r->phys.system_ram;   /* untracked System RAM == WB */

    if (guest_says_cached && r->verdict == V_CACHED)
        return "consistent — nothing overrode the guest";
    if (guest_says_cached && r->verdict == V_UNCACHED_CLASS)
        return "★★★ THE GUEST RECORDS CACHED AND THE CPU IS NOT — decider 1 or 2 forced "
               "uncached. Every access here is a bus transaction.";
    if (!guest_says_cached && r->verdict == V_CACHED)
        return "★★★ THE GUEST ASKED FOR UNCACHED AND GOT CACHED — decider 2 DISCARDED the "
               "guest's choice (x86: IPAT). A register poll here can hang.";
    return "consistent — the guest's choice was honoured";
}

static void print_report(void)
{
    printf("\n");
    printf("%-22s %-26s %-34s %-24s %-9s %s\n",
           "region", "role", "guest record", "timed", "ratio", "gpa");
    printf("%-22s %-26s %-34s %-24s %-9s %s\n",
           "----------------------", "--------------------------",
           "----------------------------------", "------------------------",
           "---------", "---");
    for (int i = 0; i < g_nregions; i++) {
        const region_t *r = &g_regions[i];
        char rec[64];
        if (!r->mapped)                 snprintf(rec, sizeof rec, "(not mapped)");
        else if (!r->gpa_known)         snprintf(rec, sizeof rec, "(gpa unknown)");
        else if (r->phys.redacted)      snprintf(rec, sizeof rec, "(iomem redacted)");
        else if (r->recorded != MT_NONE)
            snprintf(rec, sizeof rec, "%s", memtype_name(r->recorded));
        else
            snprintf(rec, sizeof rec, "untracked/%s",
                     r->phys.system_ram ? "System RAM" : (r->phys.label[0] ? r->phys.label : "?"));
        char gpa[32];
        if (r->gpa_known) snprintf(gpa, sizeof gpa, "%#" PRIx64, r->gpa);
        else              snprintf(gpa, sizeof gpa, "-");
        char ratio[16];
        if (r->verdict == V_CACHED || r->verdict == V_UNCACHED_CLASS ||
            r->verdict == V_INCONCL_BAND)
            snprintf(ratio, sizeof ratio, "%.1fx", r->ratio);
        else snprintf(ratio, sizeof ratio, "-");
        printf("%-22s %-26s %-34s %-24s %-9s %s\n",
               r->name, role_name(r->role), rec, verdict_name(r->verdict), ratio, gpa);
    }
    printf("\nreadings\n--------\n");
    for (int i = 0; i < g_nregions; i++) {
        const region_t *r = &g_regions[i];
        printf("  %-28s %s\n", r->name, r->mapped ? attribute(r) : "NOT MEASURED");
        if (r->note[0]) printf("  %-28s   note: %s\n", "", r->note);
        if (!r->gpa_known && r->mapped && r->gpa_why[0])
            printf("  %-28s   gpa: %s\n", "", r->gpa_why);
    }
}

static void usage(const char *a0)
{
    fprintf(stderr,
        "usage: %s [options]\n"
        "  --nvidia                 add the NVIDIA (vendor 0x10de) BAR0 and BAR1 subjects\n"
        "  --pci BDF:BAR            add one PCI BAR subject\n"
        "  --devmem-phys 0xADDR     force the known-positive's physical address\n"
        "  --footprint N            bytes touched per pass for RAM subjects (default %u)\n"
        "  --stride N               bytes between loads (default %u)\n"
        "  --allow-device-reads     widen a device subject's footprint past one cache line\n"
        "  --no-mismatch-control    skip the footprint-mismatch control (not recommended)\n"
        "  -v                       per-region timing detail on stderr\n"
        "\nexit: 0 measured, 2 VOID (no known-positive fired), 3 could not run\n",
        a0, DEFAULT_FOOTPRINT, DEFAULT_STRIDE);
}

int main(int argc, char **argv)
{
    unsigned footprint = DEFAULT_FOOTPRINT, stride = DEFAULT_STRIDE;
    unsigned dev_footprint = DEVICE_FOOTPRINT;
    int want_nvidia = 0, want_mismatch = 1;
    uint64_t forced_devmem = 0;
    char extra_pci[8][32]; int extra_bar[8]; int n_extra = 0;

    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--nvidia")) want_nvidia = 1;
        else if (!strcmp(argv[i], "--allow-device-reads")) dev_footprint = DEFAULT_FOOTPRINT;
        else if (!strcmp(argv[i], "--no-mismatch-control")) want_mismatch = 0;
        else if (!strcmp(argv[i], "-v")) g_verbose = 1;
        else if (!strcmp(argv[i], "--footprint") && i + 1 < argc) footprint = (unsigned)strtoul(argv[++i], NULL, 0);
        else if (!strcmp(argv[i], "--stride") && i + 1 < argc) stride = (unsigned)strtoul(argv[++i], NULL, 0);
        else if (!strcmp(argv[i], "--devmem-phys") && i + 1 < argc) forced_devmem = strtoull(argv[++i], NULL, 0);
        else if (!strcmp(argv[i], "--pci") && i + 1 < argc && n_extra < 8) {
            char *s = argv[++i]; char *c = strrchr(s, ':');
            if (!c) { usage(argv[0]); return 3; }
            *c = '\0';
            snprintf(extra_pci[n_extra], sizeof extra_pci[0], "%s", s);
            extra_bar[n_extra] = atoi(c + 1);
            n_extra++;
        } else { usage(argv[0]); return 3; }
    }
    if (stride == 0 || footprint < stride) { usage(argv[0]); return 3; }

    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = fault_handler;
    sigaction(SIGBUS, &sa, NULL);
    sigaction(SIGSEGV, &sa, NULL);

    printf("memtype_probe — the EFFECTIVE memory type, measured in the guest\n");
    printf("uid=%d  page=%ld  footprint=%u stride=%u device-footprint=%u\n",
           (int)geteuid(), sysconf(_SC_PAGESIZE), footprint, stride, dev_footprint);

    const char *why = NULL;
    char *iomem = slurp(IOMEM_PATH, &why);
    if (!iomem) gate("SKIPPED iomem — %s (%s)", IOMEM_PATH, why);
    char *pat = slurp(PAT_PATH, &why);
    if (!pat) {
#if defined(__x86_64__) || defined(__i386__)
        gate("SKIPPED pat_memtype_list — %s (%s)", PAT_PATH, why);
#else
        gate("SKIPPED pat_memtype_list — not x86; on this architecture the question is "
             "Normal-vs-Device, not PAT, and this probe does NOT fall back to assuming "
             "write-back");
#endif
    }

    /* ⊘ A redacted /proc/iomem is worse than an absent one: it parses. Detect it once, on a
     * range we know the answer for, and refuse the whole categorical half rather than let
     * every region report "not System RAM". */
    if (iomem) {
        physclass_t t;
        classify_physical(iomem, 0x100000, 0x1000, &t);
        if (t.redacted) {
            gate("SKIPPED categorical instruments — /proc/iomem is REDACTED for this uid "
                 "(every address reads 0). Run as root.");
            free(iomem); iomem = NULL;
        }
    }

    /* ── the controls, always ────────────────────────────────────────────────────────── */
    add_anon("anon-wb", ROLE_CONTROL_WB, 1u << 20, footprint, stride);

    /* ★★★★★ The instrument's own failure mode, exhibited rather than trusted not to occur:
     * a 256 MiB write-back buffer strided a page at a time, judged against a reference of the
     * DEFAULT footprint rather than its own. This is ORDINARY WRITE-BACK MEMORY and it is
     * expected NOT to come back `cached`. If it ever does come back `cached`, the footprint
     * discipline stopped being load-bearing and the constants can be re-read as properties of
     * the memory type after all. */
    if (want_mismatch) {
        region_t *m = new_region("control-mismatch", ROLE_CONTROL_MISMATCH,
                                 256u << 20, 4096u);
        if (m) {
            m->ref_footprint = footprint;      /* ⊘ deliberately NOT m->footprint */
            size_t len = 256u << 20;
            void *p = mmap(NULL, len, PROT_READ | PROT_WRITE,
                           MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (p == MAP_FAILED) {
                snprintf(m->note, sizeof m->note, "mmap 256 MiB failed: %s", strerror(errno));
            } else {
                memset(p, 0x5a, len);
                m->va = p; m->map_len = len; m->mapped = 1;
                snprintf(m->note, sizeof m->note,
                         "256 MiB of ORDINARY WRITE-BACK RAM, one load per page, judged "
                         "against a %u-byte reference. Expected NOT to come back `cached` — "
                         "that is the point, and it is the instrument's own failure mode.",
                         footprint);
            }
        }
    }

    /* ── the known-positive ──────────────────────────────────────────────────────────── */
    uint64_t kp_phys = forced_devmem;
    char kp_label[64] = "forced";
    if (!kp_phys && iomem && !pick_devmem_candidate(iomem, &kp_phys, kp_label, sizeof kp_label))
        kp_phys = 0;
    if (kp_phys)
        add_devmem("devmem-nonram", ROLE_KNOWN_POSITIVE, kp_phys, footprint, stride, kp_label);
    else
        gate("SKIPPED known-positive devmem — no non-`System RAM` top-level range found");

    /* ── the subjects ────────────────────────────────────────────────────────────────── */
    if (want_nvidia) {
        char bdf[32];
        if (find_pci_vendor("0x10de", bdf, sizeof bdf)) {
            /* BAR0 = the register aperture, and the window the doorbell store lands in.
             * BAR1 = the framebuffer aperture. */
            add_pci_bar(bdf, 0, dev_footprint, stride < dev_footprint ? stride : 4);
            add_pci_bar(bdf, 1, dev_footprint, stride < dev_footprint ? stride : 4);
        } else {
            gate("SKIPPED nvidia — no PCI device with vendor 0x10de");
        }
    }
    for (int i = 0; i < n_extra; i++)
        add_pci_bar(extra_pci[i], extra_bar[i], dev_footprint,
                    stride < dev_footprint ? stride : 4);

    /* ── measure ─────────────────────────────────────────────────────────────────────── */
    for (int i = 0; i < g_nregions; i++)
        measure_region(&g_regions[i], iomem, pat);

    print_report();

    /* ── the gate ────────────────────────────────────────────────────────────────────── */
    int kp_fired = 0, kp_present = 0, subjects = 0;
    for (int i = 0; i < g_nregions; i++) {
        const region_t *r = &g_regions[i];
        if (r->role == ROLE_KNOWN_POSITIVE) {
            kp_present = 1;
            if (r->verdict == V_UNCACHED_CLASS) kp_fired = 1;
        }
        if (r->role == ROLE_SUBJECT && r->mapped) subjects++;
    }
    /* A PCI BAR that came back uncached is a known-positive too — it is a region whose type
     * we know differs, and it fired. */
    if (!kp_fired)
        for (int i = 0; i < g_nregions; i++)
            if (g_regions[i].role == ROLE_SUBJECT &&
                g_regions[i].verdict == V_UNCACHED_CLASS) { kp_fired = 1; kp_present = 1; }

    printf("\n");
    printf("★ MEMTYPE PROBE regions=%d subjects=%d known_positive=%s controls_ok=%s\n",
           g_nregions, subjects,
           kp_fired ? "FIRED" : (kp_present ? "DID-NOT-FIRE" : "ABSENT"),
           iomem ? "categorical-available" : "categorical-unavailable");

    if (!kp_fired) {
        gate("VOID — no region was reported uncached. A probe that reports \"everything is "
             "write-back\" without ever having reported anything else is not a measurement.");
        return 2;
    }
    gate("RAN memtype_probe — known-positive fired");
    return 0;
}
