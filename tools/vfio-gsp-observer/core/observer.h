/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef VFIO_GSP_OBSERVER_CORE_H
#define VFIO_GSP_OBSERVER_CORE_H
#include "queue.h"
/* read(...,NULL,n) validates the entire range without touching bytes.
 * Both forms MUST refuse MMIO/non-guest RAM and arithmetic overflow. */
typedef int (*VG_READ)(void *,uint64_t,void *,size_t);
typedef void (*VG_EMIT)(void *,uint64_t,const KFGT_RECORD *,const unsigned char *);
typedef struct {
    uint64_t triggers,bootstrap_attempts,attached,read_failures,invalid_bootstrap;
    uint64_t uninitialized_headers,invalid_headers,unstable,mapping_changed;
    uint64_t records,gaps,invalid_elements;
} VG_STATS;
typedef struct {
    VG_READ read; VG_EMIT emit; void *opaque;
    uint64_t generation,table,ptes[KFGT_MAX_PAGES];
    uint32_t pages,offset[2],size[2],mailbox_low,mailbox_seen,active;
    KFGT_CURSOR cursor[2]; VG_STATS stats;
    unsigned char a[KFGT_MAX_QUEUE],b[KFGT_MAX_QUEUE],scratch[KFGT_MAX_MESSAGE];
    unsigned char table_copy[KFGT_PAGE],libos[KFGT_PAGE];
} VG_OBSERVER;
void vg_init(VG_OBSERVER *,VG_READ,VG_EMIT,void *);
/* Call before forwarding BAR0 writes / interrupt injection. Never writes RAM. */
void vg_mmio(VG_OBSERVER *,uint64_t offset,uint64_t data,unsigned size,int write,uint64_t ns);
void vg_poll(VG_OBSERVER *,uint64_t ns);
void vg_reset(VG_OBSERVER *);
#endif
