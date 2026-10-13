/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * The lock-free SPSC record ring: ordering, whole-record delivery, the priority reserve, and a
 * two-thread stress run (build with SAN=-fsanitize=thread to let ThreadSanitizer judge it).
 */
#include "../lib/kfwc_ring.h"
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int g_checks, g_fail;
#define CHECK(cond)                                                                    \
    do {                                                                               \
        g_checks++;                                                                    \
        if (!(cond)) {                                                                 \
            g_fail++;                                                                  \
            fprintf(stderr, "  FAIL %s:%d %s\n", __FILE__, __LINE__, #cond);          \
        }                                                                              \
    } while (0)

static kfwc_u32 mk(kfwc_u8 *out, kfwc_u32 kind, kfwc_u32 id, kfwc_u32 payload)
{
    kfwc_cap_rec r;
    kfwc_u8 body[70000];
    kfwc_u32 i;
    memset(&r, 0, sizeof(r));
    r.kind = kind;
    r.dir = id & 1;
    r.queue_id = 1;
    r.payload_bytes = kind == KFWC_K_ELEMENTS ? payload : 0;
    r.counter = id;
    r.first_entry = id;
    r.entry_count = payload;
    for (i = 0; i < r.payload_bytes; i++)
        body[i] = (kfwc_u8)(id * 7u + i);
    return kfwc_cap_rec_encode(&r, body, out, 70000);
}

static int verify(const kfwc_u8 *p, kfwc_u32 total, kfwc_u32 id)
{
    kfwc_cap_rec r;
    const kfwc_u8 *pl;
    kfwc_u32 used, i;
    if (kfwc_cap_rec_parse(p, total, &r, &pl, &used) != KFWC_PARSE_OK || used != total || r.counter != id)
        return 0;
    for (i = 0; i < r.payload_bytes; i++)
        if (pl[i] != (kfwc_u8)(id * 7u + i))
            return 0;
    return 1;
}

static void t_basic(void)
{
    static kfwc_u8 store[1 << 16], rec[70000], out[70000];
    kfwc_ring r;
    kfwc_u32 n, i, got = 0, total;
    kfwc_ring_init(&r, store, sizeof(store), 4096);
    for (i = 0; i < 20; i++) {
        n = mk(rec, KFWC_K_ELEMENTS, i, 100 + i);
        CHECK(kfwc_ring_put(&r, rec, n) == 0);
    }
    /* reading in tiny buffers still yields whole records in order */
    for (i = 0; i < 20; i++) {
        total = kfwc_ring_get(&r, out, 32 + 100 + i);
        CHECK(total == 32 + 100 + i && verify(out, total, i));
        got++;
    }
    CHECK(got == 20 && kfwc_ring_used(&r) == 0);
    CHECK(kfwc_ring_get(&r, out, sizeof(out)) == 0);
    /* a buffer smaller than the next record returns nothing rather than a fragment */
    n = mk(rec, KFWC_K_ELEMENTS, 99, 500);
    CHECK(kfwc_ring_put(&r, rec, n) == 0);
    CHECK(kfwc_ring_get(&r, out, 100) == 0 && kfwc_ring_used(&r) == n);
    CHECK(kfwc_ring_get(&r, out, sizeof(out)) == n);
}

static void t_reserve(void)
{
    static kfwc_u8 store[1 << 16], rec[70000];
    kfwc_ring r;
    kfwc_u32 n, i = 0, data = 0, prio = 0;
    kfwc_ring_init(&r, store, 20000, 4096);
    for (;; i++) {
        n = mk(rec, KFWC_K_ELEMENTS, i, 4096);
        if (kfwc_ring_put(&r, rec, n))
            break;
        data++;
    }
    CHECK(data == 3 && r.refused == 1);                 /* 3 * 4128 = 12384; the 4th would eat the reserve */
    for (i = 0; i < 100; i++) {
        n = mk(rec, i % 2 ? KFWC_K_LOSS_OVERFLOW : KFWC_K_LOSS_UNKNOWN, 1000 + i, 0);
        if (kfwc_ring_put(&r, rec, n) == 0)
            prio++;
    }
    CHECK(prio > 100 - 1 || prio >= 100);                /* 100 * 32 B = 3200 fits in the 4096 + slack */
    /* a record that can never fit is refused, not looped on */
    n = mk(rec, KFWC_K_ELEMENTS, 5, 60000);
    CHECK(kfwc_ring_put(&r, rec, n) == 1);
}

typedef struct { kfwc_ring *r; kfwc_u32 n; kfwc_u32 stored, refused; kfwc_u8 *order; } prod;
static void *producer(void *a)
{
    prod *p = (prod *)a;
    kfwc_u8 rec[70000];
    kfwc_u32 i, s = 12345;
    for (i = 0; i < p->n; i++) {
        kfwc_u32 n;
        s = s * 1103515245u + 12345u;
        n = mk(rec, (s >> 16) % 7 == 0 ? KFWC_K_LOSS_UNKNOWN : KFWC_K_ELEMENTS, i, 1 + (s >> 8) % 9000);
        if (kfwc_ring_put(p->r, rec, n) == 0) {
            p->order[i] = 1;
            __atomic_add_fetch(&p->stored, 1, __ATOMIC_RELEASE);
        } else {
            p->order[i] = 0;
            __atomic_add_fetch(&p->refused, 1, __ATOMIC_RELEASE);
        }
    }
    return NULL;
}

static void t_threads(void)
{
    enum { N = 200000 };
    static kfwc_u8 store[1 << 20], out[1 << 17];
    kfwc_ring r;
    prod p;
    pthread_t t;
    kfwc_u32 seen = 0, last = 0, bad = 0, next_expect = 0;
    kfwc_ring_init(&r, store, sizeof(store), 8192);
    p.r = &r;
    p.n = N;
    p.stored = p.refused = 0;
    p.order = (kfwc_u8 *)calloc(N, 1);
    pthread_create(&t, NULL, producer, &p);
    for (;;) {
        kfwc_u32 n = kfwc_ring_get(&r, out, sizeof(out)), at = 0;
        while (at < n) {
            kfwc_u32 total = 32 + kfwc_get32(out + at + 8), id = kfwc_get32(out + at + 12);
            if (!verify(out + at, total, id) || id < next_expect)
                bad++;
            last = id;
            next_expect = id + 1;
            seen++;
            at += total;
        }
        if (!n && __atomic_load_n(&p.stored, __ATOMIC_ACQUIRE) + __atomic_load_n(&p.refused, __ATOMIC_ACQUIRE) == N) {
            pthread_join(t, NULL);
            n = kfwc_ring_get(&r, out, sizeof(out));
            if (!n)
                break;
            /* leftovers after the producer finished */
            for (at = 0; at < n;) {
                kfwc_u32 total = 32 + kfwc_get32(out + at + 8), id = kfwc_get32(out + at + 12);
                if (!verify(out + at, total, id))
                    bad++;
                seen++;
                at += total;
            }
            break;
        }
    }
    (void)last;
    CHECK(bad == 0);
    CHECK(seen == p.stored);                   /* every stored record arrived exactly once, whole, in order */
    CHECK(p.stored + p.refused == N);
    printf("  stress: %u stored, %u refused, high water %llu of %u bytes\n", p.stored, p.refused,
           (unsigned long long)r.high_water, (unsigned)sizeof(store));
    free(p.order);
}

int main(void)
{
    t_basic();
    t_reserve();
    t_threads();
    printf("test_ring: %d checks, %d failed\n", g_checks, g_fail);
    return g_fail ? 1 : 0;
}
