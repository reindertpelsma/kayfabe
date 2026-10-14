/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#ifndef KF_GSP_QUEUE_H
#define KF_GSP_QUEUE_H
#include "trace_protocol.h"
#include <stddef.h>
typedef struct { uint32_t size, count, write, rx_offset, entry_offset; } KFGT_QUEUE;
typedef struct { uint32_t last, initialized; uint64_t gaps, invalid; } KFGT_CURSOR;
typedef void (*KFGT_EMIT)(void *, const KFGT_RECORD *, const unsigned char *);
uint32_t kf_u32(const void *);
uint64_t kf_u64(const void *);
int kf_queue_header(const unsigned char *, size_t, KFGT_QUEUE *);
/* Snapshot includes queue header and entries in logical page-table order.
 * Scratch is at least KFGT_MAX_MESSAGE bytes. Direction 0=request,1=reply.
 * Caller must obtain identical copies, validate mapping again, and keep snapshot private. */
int kf_queue_records(const unsigned char *, size_t, unsigned char *,
                    KFGT_CURSOR *, uint32_t, uint64_t, uint64_t, KFGT_EMIT, void *);
#endif
