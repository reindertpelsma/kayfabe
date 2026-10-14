/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * Single-producer / single-consumer byte ring for whole KFGSPCAP records, lock free.
 *
 *  - The producer is the capture worker thread (engine sink). It never waits: a record that does
 *    not fit is refused (the engine then accounts it with a LOSS_OVERFLOW record).
 *  - The consumer is the READ dispatch (or a test thread). It never blocks the producer.
 *  - Only whole records cross: the consumer parses payload_bytes from the 32-byte header and
 *    copies complete records that fit its buffer, so a reader can never see half a record.
 *  - A reserve is kept free for priority records (BIND, LOSS_*, END): data records
 *    (ELEMENTS) are refused first, so a loss notice can always be stored.
 *
 * head/tail are monotonically increasing byte counters (64-bit, never wrap in practice);
 * publication uses release stores and acquire loads.
 */
#ifndef KFWC_RING_H
#define KFWC_RING_H

#include "kfwc_record.h"

typedef struct kfwc_ring {
    kfwc_u8 *buf;
    kfwc_u32 cap;            /* bytes, > 2 * reserve */
    kfwc_u32 reserve;        /* bytes only priority records may use */
    volatile kfwc_u64 head;  /* consumer position (bytes consumed) */
    volatile kfwc_u64 tail;  /* producer position (bytes produced) */
    /* statistics: written by one side each, read by anyone */
    volatile kfwc_u64 puts, refused, bytes_in, bytes_out, high_water;
} kfwc_ring;

void kfwc_ring_init(kfwc_ring *r, kfwc_u8 *buf, kfwc_u32 cap, kfwc_u32 reserve);
/* Producer. rec is one complete serialized record. Returns 0 stored, 1 refused (full). */
int kfwc_ring_put(kfwc_ring *r, const kfwc_u8 *rec, kfwc_u32 len);
/* Consumer. Copies as many whole records as fit in out_cap; returns bytes copied. */
kfwc_u32 kfwc_ring_get(kfwc_ring *r, kfwc_u8 *out, kfwc_u32 out_cap);
kfwc_u32 kfwc_ring_used(const kfwc_ring *r);

#endif /* KFWC_RING_H */
