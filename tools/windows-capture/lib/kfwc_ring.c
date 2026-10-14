/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include "kfwc_ring.h"
#include <string.h>

/* Acquire/release on both clang-cl and gcc/clang (builtins); real cl.exe falls back to
 * interlocked operations (full barriers, equally correct). */
#if defined(__GNUC__) || defined(__clang__)
#define LOAD_ACQ(p) __atomic_load_n((p), __ATOMIC_ACQUIRE)
#define STORE_REL(p, v) __atomic_store_n((p), (v), __ATOMIC_RELEASE)
#else
#include <intrin.h>
#define LOAD_ACQ(p) ((kfwc_u64)_InterlockedOr64((volatile __int64 *)(p), 0))
#define STORE_REL(p, v) ((void)_InterlockedExchange64((volatile __int64 *)(p), (__int64)(v)))
#endif

void kfwc_ring_init(kfwc_ring *r, kfwc_u8 *buf, kfwc_u32 cap, kfwc_u32 reserve)
{
    memset(r, 0, sizeof(*r));
    r->buf = buf;
    r->cap = cap;
    r->reserve = reserve;
}

kfwc_u32 kfwc_ring_used(const kfwc_ring *r)
{
    kfwc_u64 t = LOAD_ACQ(&r->tail), h = LOAD_ACQ(&r->head);
    return (kfwc_u32)(t - h);
}

static void copy_in(kfwc_ring *r, kfwc_u64 at, const kfwc_u8 *src, kfwc_u32 len)
{
    kfwc_u32 off = (kfwc_u32)(at % r->cap), first = r->cap - off;
    if (first > len)
        first = len;
    memcpy(r->buf + off, src, first);
    if (len > first)
        memcpy(r->buf, src + first, len - first);
}

static void copy_out(const kfwc_ring *r, kfwc_u64 at, kfwc_u8 *dst, kfwc_u32 len)
{
    kfwc_u32 off = (kfwc_u32)(at % r->cap), first = r->cap - off;
    if (first > len)
        first = len;
    memcpy(dst, r->buf + off, first);
    if (len > first)
        memcpy(dst + first, r->buf, len - first);
}

int kfwc_ring_put(kfwc_ring *r, const kfwc_u8 *rec, kfwc_u32 len)
{
    kfwc_u64 t = r->tail, h = LOAD_ACQ(&r->head);
    kfwc_u32 used = (kfwc_u32)(t - h), kind = rec[4];
    kfwc_u32 room = r->cap - used;
    /* Data records may not eat the reserve; priority records may. */
    kfwc_u32 need = (kind == KFWC_K_ELEMENTS) ? len + r->reserve : len;
    if (len > r->cap || room < need) {
        r->refused++;
        return 1;
    }
    copy_in(r, t, rec, len);
    STORE_REL(&r->tail, t + len);
    r->puts++;
    r->bytes_in += len;
    if (used + len > r->high_water)
        r->high_water = used + len;
    return 0;
}

kfwc_u32 kfwc_ring_get(kfwc_ring *r, kfwc_u8 *out, kfwc_u32 out_cap)
{
    kfwc_u64 h = r->head, t = LOAD_ACQ(&r->tail);
    kfwc_u32 done = 0;
    while (h + KFWC_CAP_REC_HEADER_BYTES <= t) {
        kfwc_u8 hdr[KFWC_CAP_REC_HEADER_BYTES];
        kfwc_u32 total;
        copy_out(r, h, hdr, sizeof(hdr));
        total = KFWC_CAP_REC_HEADER_BYTES + kfwc_get32(hdr + 8);
        if (h + total > t || total > out_cap - done)
            break; /* not fully published (cannot happen) or does not fit this call */
        copy_out(r, h, out + done, total);
        done += total;
        h += total;
    }
    if (done) {
        STORE_REL(&r->head, h);
        r->bytes_out += done;
    }
    return done;
}
