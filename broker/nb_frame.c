/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nb_frame.c -- a READ-ONLY window onto the frame the broker is presenting, so
 * the broker can read the few pixels under an XOR cursor (D2, 2026-10-17).
 *
 * WHAT MAY BE READ, AND WHY ONLY THAT.  The broker's security property is that
 * it never touches frame memory: the compositor imports the buffer, the broker
 * only forwards a handle.  The owner allowed one exception, with these bounds:
 *
 *   - a memfd carrying F_SEAL_SHRINK (the wl_shm tier).  The seal is what makes
 *     the measured extent mean something: a memfd cannot be truncated under a
 *     read, so a page inside the mapping cannot be SIGBUS (a hole punched in a
 *     sealed-shrink memfd re-faults as zeros inside i_size).
 *   - a LINEAR dma-buf whose EXPORTER is system memory (udmabuf, the system
 *     dma-heap): mmap(PROT_READ), every read bracketed by DMA_BUF_IOCTL_SYNC.
 *     A dma-buf cannot change size.  An exporter that is not known to be RAM
 *     (a GPU's VRAM export is uncached on the far side of PCIe: a read is
 *     hundreds of times slower) is refused here, not timed: the allowlist is
 *     the whole policy, and a refusal is the approximation fallback.
 *
 * AND ONLY THE CURSOR'S ROWS ARE READ.  nb_frame_read_rect() copies a rectangle
 * of at most NVKVM_BROKER_CURSOR_MAX_DIM square pixels (the cursor's own
 * bounding box clipped to the frame): a few KB per frame, never the frame.
 * Every row's end is checked against the mapping's length in 64 bits before it
 * is copied, so a frame whose description and mapping disagree (resized,
 * replaced, lied about) reads nothing out of range -- it is refused.
 *
 * Nothing here logs or touches a display connection: it is linked into the
 * unit test (test/test_xor.c) and driven with a sealed memfd, and with a real
 * udmabuf where the box has one.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/statfs.h>
#include <unistd.h>
#include <linux/dma-buf.h>
#include <linux/magic.h>

#include "nvkvm_broker.h"

#define NB_FRAME_FOURCC_XR24 0x34325258u
#define NB_FRAME_FOURCC_AR24 0x34325241u
#ifndef DMA_BUF_MAGIC
#define DMA_BUF_MAGIC 0x444d4142
#endif
#ifndef F_SEAL_SHRINK
#define F_SEAL_SHRINK 0x0002
#endif
#ifndef F_GET_SEALS
#define F_GET_SEALS 1034
#endif

const char *nb_frame_why_text(enum nb_frame_why why)
{
    switch (why) {
    case NB_FW_OK:       return "readable";
    case NB_FW_NONE:     return "no frame is on screen";
    case NB_FW_FORMAT:   return "the frame is not a 32-bit XR24/AR24 layout";
    case NB_FW_GEOMETRY: return "the frame's description does not fit its buffer";
    case NB_FW_NOSEAL:   return "a memfd without F_SEAL_SHRINK cannot be read safely";
    case NB_FW_MODIFIER: return "a tiled dma-buf cannot be read by the CPU";
    case NB_FW_EXPORTER: return "the dma-buf is not known to live in system memory";
    case NB_FW_FDINFO:   return "the dma-buf's exporter could not be read";
    case NB_FW_MMAP:     return "the frame could not be mapped read-only";
    case NB_FW_STALE:    return "the buffer on screen is not the one that was mapped";
    case NB_FW_RANGE:    return "a read outside the mapped frame was asked for";
    case NB_FW_SYNC:     return "DMA_BUF_IOCTL_SYNC failed";
    }
    return "unreadable";
}

bool nb_fdinfo_exp_name(const char *text, size_t len, char *out, size_t n)
{
    static const char key[] = "\nexp_name:\t";
    size_t i, k = sizeof key - 1u, j = 0;

    if (!text || !out || n == 0) {
        return false;
    }
    out[0] = '\0';
    /* never the first line (pos: is), so a line start is always "\n" */
    for (i = 0; i + k <= len; i++) {
        if (memcmp(text + i, key, k) == 0) {
            size_t p = i + k;

            while (p < len && text[p] != '\n' && text[p] != '\0' && j + 1 < n) {
                out[j++] = text[p++];
            }
            out[j] = '\0';
            return j > 0;
        }
    }
    return false;
}

bool nb_frame_exporter_ok(const char *exp_name)
{
    /* system RAM by construction: udmabuf pins memfd pages; the dma-heaps
     * "system" / "system-uncached" allocate RAM.  Everything else may be
     * device memory. */
    return exp_name &&
           (strcmp(exp_name, "udmabuf") == 0 || strcmp(exp_name, "system") == 0 ||
            strcmp(exp_name, "system-uncached") == 0);
}

bool nb_frame_layout_ok(uint64_t modifier, const char *exp_name)
{
    if (!nb_frame_exporter_ok(exp_name)) {
        return false;
    }
    /* LINEAR is the CPU-readable layout.  The implicit modifier means "the
     * exporter's own layout"; for a system-memory exporter that is linear. */
    return modifier == NB_FRAME_MOD_LINEAR || modifier == NB_FRAME_MOD_INVALID;
}

static bool nb_frame_geom_ok(const struct nb_frame_geom *g, uint64_t size,
                             uint64_t *need_out)
{
    uint64_t need;

    if (g->w == 0 || g->h == 0 || g->w > NVKVM_BROKER_MAX_DIM ||
        g->h > NVKVM_BROKER_MAX_DIM) {
        return false;
    }
    if ((uint64_t)g->stride < (uint64_t)g->w * 4u) {
        return false;
    }
    /* the last row is only w*4 bytes long; 64-bit throughout, no operand wraps */
    need = (uint64_t)g->offset + (uint64_t)g->stride * (g->h - 1u) +
           (uint64_t)g->w * 4u;
    if (need > size || need > (uint64_t)INT32_MAX) {
        return false;
    }
    *need_out = need;
    return true;
}

bool nb_frame_open(struct nb_frame *f, int fd, bool is_shm, uint64_t size,
                   const struct nb_frame_geom *g, uint64_t modifier)
{
    uint64_t need = 0;
    char info[4096 + 1];
    char pathbuf[48];
    void *m;
    int sfd = -1;

    memset(f, 0, sizeof *f);
    f->sync_fd = -1;
    f->w = g->w;
    f->h = g->h;
    f->stride = g->stride;
    f->offset = g->offset;
    f->fourcc = g->fourcc;
    if (g->fourcc != NB_FRAME_FOURCC_XR24 && g->fourcc != NB_FRAME_FOURCC_AR24) {
        f->why = NB_FW_FORMAT;
        return false;
    }
    if (!nb_frame_geom_ok(g, size, &need)) {
        f->why = NB_FW_GEOMETRY;
        return false;
    }
    if (is_shm) {
        struct statfs sfs;
        int seals = fcntl(fd, F_GET_SEALS);

        /* tmpfs only: the same proof the validator made, repeated because the
         * mapping below leans on it */
        if (seals < 0 || fstatfs(fd, &sfs) < 0 || sfs.f_type != TMPFS_MAGIC ||
            !(seals & F_SEAL_SHRINK)) {
            f->why = NB_FW_NOSEAL;
            return false;
        }
    } else {
        struct statfs sfs;
        ssize_t got = 0;
        int pfd;

        /* early and cheap: a tiled layout is refused before anything is read */
        if (modifier != NB_FRAME_MOD_LINEAR && modifier != NB_FRAME_MOD_INVALID) {
            f->why = NB_FW_MODIFIER;
            return false;
        }
        snprintf(pathbuf, sizeof pathbuf, "/proc/self/fdinfo/%d", fd);
        pfd = open(pathbuf, O_RDONLY | O_CLOEXEC | O_NOCTTY);
        if (pfd < 0) {
            f->why = NB_FW_FDINFO;
            return false;
        }
        while ((size_t)got < sizeof info - 1u) {
            ssize_t r = read(pfd, info + got, sizeof info - 1u - (size_t)got);

            if (r < 0 && errno == EINTR) {
                continue;
            }
            if (r <= 0) {
                break;
            }
            got += r;
        }
        close(pfd);
        if (!nb_fdinfo_exp_name(info, (size_t)got, f->exporter, sizeof f->exporter)) {
            f->why = NB_FW_FDINFO;
            return false;
        }
        if (!nb_frame_exporter_ok(f->exporter)) {
            f->why = NB_FW_EXPORTER;
            return false;
        }
        if (!nb_frame_layout_ok(modifier, f->exporter)) {
            f->why = NB_FW_MODIFIER;
            return false;
        }
        if (fstatfs(fd, &sfs) < 0 || sfs.f_type != DMA_BUF_MAGIC) {
            f->why = NB_FW_FDINFO;
            return false;
        }
        sfd = fcntl(fd, F_DUPFD_CLOEXEC, 3);
        if (sfd < 0) {
            f->why = NB_FW_MMAP;
            return false;
        }
    }
    /* Exactly the extent the description needs (a dma-buf mapping may not
     * exceed its buffer; a sparse 4 GiB memfd must not be mapped whole). */
    m = mmap(NULL, (size_t)need, PROT_READ, MAP_SHARED, is_shm ? fd : sfd, 0);
    if (m == MAP_FAILED) {
        if (sfd >= 0) {
            close(sfd);
        }
        f->why = NB_FW_MMAP;
        return false;
    }
    f->kind = is_shm ? NB_FRAME_SHM : NB_FRAME_DMABUF;
    f->why = NB_FW_OK;
    f->map = m;
    f->len = (size_t)need;
    f->sync_fd = sfd;
    return true;
}

void nb_frame_close(struct nb_frame *f)
{
    if (f->map) {
        munmap((void *)(uintptr_t)f->map, f->len);
    }
    if (f->kind == NB_FRAME_DMABUF && f->sync_fd >= 0) {
        close(f->sync_fd);
    }
    memset(f, 0, sizeof *f);
    f->sync_fd = -1;
}

enum nb_frame_why nb_frame_check(const struct nb_frame *f,
                                 const struct nb_frame_geom *g)
{
    uint64_t need;

    if (!f || f->kind == NB_FRAME_NONE || !f->map) {
        return f && f->why != NB_FW_OK ? f->why : NB_FW_NONE;
    }
    if (f->w != g->w || f->h != g->h || f->stride != g->stride ||
        f->offset != g->offset || f->fourcc != g->fourcc) {
        return NB_FW_STALE;
    }
    if (!nb_frame_geom_ok(g, (uint64_t)f->len, &need) || need > f->len) {
        return NB_FW_GEOMETRY;
    }
    return NB_FW_OK;
}

static bool nb_frame_sync(const struct nb_frame *f, uint64_t flags)
{
    struct dma_buf_sync s;
    int tries;

    if (f->kind != NB_FRAME_DMABUF) {
        return true;
    }
    s.flags = flags | DMA_BUF_SYNC_READ;
    for (tries = 0; tries < 4; tries++) {
        if (ioctl(f->sync_fd, DMA_BUF_IOCTL_SYNC, &s) == 0) {
            return true;
        }
        if (errno != EINTR) {
            break;
        }
    }
    return false;
}

enum nb_frame_why nb_frame_read_rect(const struct nb_frame *f, uint32_t rx,
                                     uint32_t ry, uint32_t rw, uint32_t rh,
                                     uint32_t *out)
{
    uint32_t row, x;
    enum nb_frame_why why = NB_FW_OK;

    if (!f || f->kind == NB_FRAME_NONE || !f->map || !out) {
        return f && f->why != NB_FW_OK ? f->why : NB_FW_NONE;
    }
    if (rw == 0 || rh == 0 || rw > NVKVM_BROKER_CURSOR_MAX_DIM ||
        rh > NVKVM_BROKER_CURSOR_MAX_DIM ||
        (uint64_t)rx + rw > f->w || (uint64_t)ry + rh > f->h) {
        return NB_FW_RANGE;
    }
    if (!nb_frame_sync(f, DMA_BUF_SYNC_START)) {
        return NB_FW_SYNC;
    }
    for (row = 0; row < rh; row++) {
        uint64_t start = (uint64_t)f->offset + (uint64_t)(ry + row) * f->stride +
                         (uint64_t)rx * 4u;
        uint64_t end = start + (uint64_t)rw * 4u;
        const uint8_t *p;

        if (end > (uint64_t)f->len) {
            why = NB_FW_RANGE;
            break;
        }
        p = f->map + start;
        for (x = 0; x < rw; x++) {
            /* B,G,R,X in memory is DRM's little-endian XRGB8888 */
            out[(size_t)row * rw + x] =
                (uint32_t)p[x * 4u] | ((uint32_t)p[x * 4u + 1u] << 8) |
                ((uint32_t)p[x * 4u + 2u] << 16) | ((uint32_t)p[x * 4u + 3u] << 24);
        }
    }
    if (!nb_frame_sync(f, DMA_BUF_SYNC_END) && why == NB_FW_OK) {
        why = NB_FW_SYNC;
    }
    return why;
}
