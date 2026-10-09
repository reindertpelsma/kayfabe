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

/* ---- 2026-10-09 refusal ablation: DEBUG ONLY, PERTURBING, default off ------------------------------
 * The one place where this helper WRITES guest RAM. vg_refuse_scan() is called at the command-queue
 * doorbell write BEFORE the write is forwarded to the device, i.e. after vg_mmio() has recorded the
 * original request. For every new message in the command queue whose (function, key) is on the rule
 * list it replaces the key by an invalid one and fixes the element checksum, so that the real GSP
 * refuses the request. fn 76 GSP_RM_CONTROL: key = cmd (u32 at element+88); fn 103 GSP_RM_ALLOC:
 * key = hClass (u32 at element+92). Checksum: XOR of the u32 words of 48+length bytes (rounded up to
 * 8) is zero (message_queue_cpu.c _checkSum32), so a key change old->new changes the stored
 * checksum word at element+32 by old^new. Nothing else is touched. */
typedef int (*VG_WRITE)(void *,uint64_t,const void *,size_t);
#define VG_REFUSE_MAX_RULES 128u
/* field 0 = the default key (fn76 cmd / fn103 hClass, as above); field 1 = fn76's hObject (offset
 * element+84): for a GSS-legacy control (cmd bit 0x8000 set) the firmware accepts an unknown cmd id
 * unconditionally, so an invalid cmd never refuses it; corrupting the object handle instead makes
 * the lookup itself fail (fn76 only; a rule with field 1 and fn!=76 is refused by vg_refuse_add). */
typedef struct { uint32_t fn,key,newkey; unsigned field; uint64_t hits; } VG_RULE;
typedef void (*VG_REFUSE_LOG)(void *,const VG_RULE *,uint32_t seq,uint32_t slot,uint32_t pages,int verified);
typedef struct {
    VG_RULE rule[VG_REFUSE_MAX_RULES]; unsigned nrules;
    VG_WRITE write; VG_REFUSE_LOG log; void *opaque;
    uint64_t generation; uint32_t next_seq; int have_next;
    uint64_t doorbells,inactive,header_bad,scanned,matched,rewritten,verify_failed,ambiguous,seq_gaps,truncated;
} VG_REFUSE;
void vg_refuse_init(VG_REFUSE *,VG_WRITE,VG_REFUSE_LOG,void *);
/* The default invalid key for a rule given without one (0 on failure for unsupported fn). */
uint32_t vg_refuse_default_key(uint32_t fn,uint32_t key);
int vg_refuse_add(VG_REFUSE *,uint32_t fn,uint32_t key,uint32_t newkey /* 0 = default */);
int vg_refuse_add_field(VG_REFUSE *,uint32_t fn,uint32_t key,uint32_t newkey,unsigned field);
void vg_refuse_scan(VG_OBSERVER *,VG_REFUSE *);
/* Call before forwarding BAR0 writes / interrupt injection. Never writes RAM. */
void vg_mmio(VG_OBSERVER *,uint64_t offset,uint64_t data,unsigned size,int write,uint64_t ns);
void vg_poll(VG_OBSERVER *,uint64_t ns);
void vg_reset(VG_OBSERVER *);
#endif
