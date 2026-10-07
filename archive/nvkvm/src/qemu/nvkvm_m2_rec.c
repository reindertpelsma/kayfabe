/*
 * nvkvm_m2_rec.c — the §6 replay-trace recorder sink.  See nvkvm_m2_rec.h for
 * the format, the four rules, and why this is not qemu_log().
 *
 * Deliberately boring: one fd, one staging buffer, no allocation and no
 * formatting on the hot path, no locks (the device is single-threaded — see R1
 * in the header), and NO CAPS ANYWHERE (R2).
 */
#include "qemu/osdep.h"
#include "qemu/error-report.h"

#include <fcntl.h>
#include <unistd.h>
#include <string.h>

#include "nvkvm_m2_rec.h"

#define NVKVM_REC_BUFSZ  (4u * 1024u * 1024u)
#define NVKVM_REC_PROVSZ 4096u                 /* provenance text block         */

/* hdr_len is fixed so the header can be re-written in place at close without
 * knowing how long the provenance text was. */
#define NVKVM_REC_HDRLEN (sizeof(NvkvmRecHdr) + NVKVM_REC_PROVSZ)

static struct {
    int      fd;
    bool     on;
    uint64_t seq;          /* the single counter: R1                          */
    uint64_t mask;
    uint64_t nbytes;
    uint64_t nerrors;
    uint32_t pos;          /* bytes staged in buf                             */
    uint8_t *buf;
} g_rec = { .fd = -1 };

QEMU_BUILD_BUG_ON(sizeof(NvkvmRecEnt) != 32);

static void nvkvm_rec_raw_write(const void *p, size_t n)
{
    const uint8_t *q = p;
    while (n) {
        ssize_t w = write(g_rec.fd, q, n);
        if (w < 0) {
            if (errno == EINTR) {
                continue;
            }
            /* Never silently drop: count it, and the count lands in the header
             * so the artefact declares its own corruption. */
            g_rec.nerrors++;
            return;
        }
        if (w == 0) {
            g_rec.nerrors++;
            return;
        }
        q += w;
        n -= (size_t)w;
    }
}

static void nvkvm_rec_flush(void)
{
    if (g_rec.pos) {
        nvkvm_rec_raw_write(g_rec.buf, g_rec.pos);
        g_rec.pos = 0;
    }
}

/* Stage `n` bytes, flushing whenever the buffer cannot take them.  Payloads
 * are bounded (<= 4096 in this device) but the code does not rely on that: an
 * oversized blob bypasses the buffer entirely rather than being truncated. */
static void nvkvm_rec_stage(const void *p, size_t n)
{
    if (n > NVKVM_REC_BUFSZ) {
        nvkvm_rec_flush();
        nvkvm_rec_raw_write(p, n);
        g_rec.nbytes += n;
        return;
    }
    if (g_rec.pos + n > NVKVM_REC_BUFSZ) {
        nvkvm_rec_flush();
    }
    memcpy(g_rec.buf + g_rec.pos, p, n);
    g_rec.pos += (uint32_t)n;
    g_rec.nbytes += n;
}

bool nvkvm_rec_on(void)
{
    return g_rec.on;
}

uint64_t nvkvm_rec_mask(void)
{
    return g_rec.mask;
}

uint64_t nvkvm_rec_count(void)
{
    return g_rec.seq;
}

bool nvkvm_rec_open(const char *path, uint64_t props, uint64_t mask,
                    const char *prov, uint64_t t0_ns)
{
    if (g_rec.on) {
        return true;
    }
    if (!path || !path[0]) {
        return false;
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        error_report("nvkvm-rec: cannot open %s: %s", path, strerror(errno));
        return false;
    }

    g_rec.fd      = fd;
    g_rec.seq     = 0;
    g_rec.mask    = mask;
    g_rec.nbytes  = 0;
    g_rec.nerrors = 0;
    g_rec.pos     = 0;
    g_rec.buf     = g_malloc(NVKVM_REC_BUFSZ);

    uint8_t *hdrblk = g_malloc0(NVKVM_REC_HDRLEN);
    NvkvmRecHdr *h = (NvkvmRecHdr *)hdrblk;
    h->magic     = NVKVM_REC_MAGIC;
    h->version   = NVKVM_REC_VERSION;
    h->hdr_len   = (uint32_t)NVKVM_REC_HDRLEN;
    h->rec_size  = (uint32_t)sizeof(NvkvmRecEnt);
    h->props     = props;
    h->mask      = mask;
    h->n_records = 0;
    h->n_bytes   = 0;
    h->n_errors  = 0;
    h->t0_ns     = t0_ns;
    if (prov) {
        size_t n = strlen(prov);
        if (n > NVKVM_REC_PROVSZ - 1) {
            n = NVKVM_REC_PROVSZ - 1;
        }
        memcpy(hdrblk + sizeof(NvkvmRecHdr), prov, n);
    }
    nvkvm_rec_raw_write(hdrblk, NVKVM_REC_HDRLEN);
    g_free(hdrblk);

    g_rec.on = true;
    return true;
}

void nvkvm_rec_emit(uint64_t kindbit, uint8_t kind, uint8_t bar, uint8_t width,
                    uint64_t a, uint64_t b, const void *payload, uint32_t len)
{
    if (!g_rec.on || !(g_rec.mask & kindbit)) {
        return;   /* a filtered event does NOT consume a sequence number */
    }
    NvkvmRecEnt e;
    e.seq   = g_rec.seq++;
    e.kind  = kind;
    e.width = width;
    e.bar   = bar;
    e.pad0  = 0;
    e.len   = payload ? len : 0;
    e.a     = a;
    e.b     = b;
    nvkvm_rec_stage(&e, sizeof(e));
    if (e.len) {
        nvkvm_rec_stage(payload, e.len);
        /* pad so the next record starts 8-byte aligned */
        uint32_t padn = (uint32_t)((8u - (e.len & 7u)) & 7u);
        if (padn) {
            static const uint8_t zero[8] = { 0 };
            nvkvm_rec_stage(zero, padn);
        }
    }
}

void nvkvm_rec_close(void)
{
    if (!g_rec.on) {
        return;
    }
    g_rec.on = false;          /* stop accepting before we tear down */
    nvkvm_rec_flush();

    /* props/t0 were written at open and are not re-derivable here; patch only
     * the three counters, leaving the rest of the on-disk header untouched.
     * They are adjacent u64s in NvkvmRecHdr, asserted below. */
    QEMU_BUILD_BUG_ON(offsetof(NvkvmRecHdr, n_bytes) !=
                      offsetof(NvkvmRecHdr, n_records) + 8);
    QEMU_BUILD_BUG_ON(offsetof(NvkvmRecHdr, n_errors) !=
                      offsetof(NvkvmRecHdr, n_records) + 16);
    uint64_t counters[3] = { g_rec.seq, g_rec.nbytes, g_rec.nerrors };
    if (pwrite(g_rec.fd, counters, sizeof(counters),
               offsetof(NvkvmRecHdr, n_records)) < 0) {
        /* nothing useful to do; the reader falls back to scanning to EOF */
    }
    fsync(g_rec.fd);
    close(g_rec.fd);
    g_rec.fd = -1;
    g_free(g_rec.buf);
    g_rec.buf = NULL;
}
