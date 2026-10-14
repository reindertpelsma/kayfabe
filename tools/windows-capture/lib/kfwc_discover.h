/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * Queue discovery. Two independent routes to the same object (a "binding": the shared
 * region's page list and the two queue geometries). Read-only.
 *
 *  (a) kfwc_discover_mailbox: BAR0 NV_PGSP_FALCON_MAILBOX0/1 -> LibOS init-args array ->
 *      "RMARGS" entry -> MESSAGE_QUEUE_INIT_ARGUMENTS -> page table -> two queues.
 *      This is exactly the chain kayfabe's own GspFsm::publish follows
 *      (crates/kf-gsp/src/boot.rs). UNVERIFIED ON REAL HARDWARE: whether the mailbox pair
 *      still holds the address after the GSP has booted is not measured; the design relies on
 *      polling it early and treats a zero/sentinel read as "not applicable here".
 *      Turing..Ada boot regime only; Hopper and later do not publish the address this way.
 *
 *  (b) kfwc_scan_step / kfwc_bind_candidate: bounded, rate-limited scan of system RAM for
 *      the self-referencing page table of the shared region, then strict validation. This is
 *      the route that produced 4,535 valid records on an RTX 4070 under driver 580.88 with
 *      the earlier tracer (origin/codex/windows-gsp-tracer-2026-10-04); it cannot recover
 *      messages that left the ring before it attached.
 */
#ifndef KFWC_DISCOVER_H
#define KFWC_DISCOVER_H

#include "kfwc_platform.h"
#include "kfwc_elem.h"
#include "kfwc_record.h"

/* discovery methods */
#define KFWC_METHOD_MAILBOX 1u      /* option (a): BAR0 boot-args mailbox */
#define KFWC_METHOD_SCAN 2u         /* option (b): physical RAM scan */

typedef struct kfwc_qgeom {
    kfwc_u32 valid;        /* tx header validated (the status header may still be zero) */
    kfwc_u32 off;          /* byte offset of this queue inside the shared region */
    kfwc_u32 size;         /* backing store bytes (header page + elements) */
    kfwc_u32 msg_size;     /* element size, KFWC_PAGE */
    kfwc_u32 count;        /* elements in the ring */
    kfwc_u32 rx_hdr_off;
    kfwc_u32 entry_off;
    kfwc_u32 write_ptr;    /* last observed, informational */
} kfwc_qgeom;

typedef struct kfwc_binding {
    kfwc_u32 method;       /* KFWC_METHOD_* */
    kfwc_u32 layout;       /* KFWC_LAYOUT_*, 0 until the first element decides */
    kfwc_u64 table_pa;     /* shared region page 0 == the page table */
    kfwc_u64 libos_args_pa;
    kfwc_u32 pages;        /* region pages == PTE count */
    kfwc_u64 ptes[KFWC_MAX_REGION_PAGES];
    kfwc_qgeom q[2];       /* [0] command queue, [1] status queue */
    kfwc_u8 scratch[KFWC_PAGE];   /* discovery working page (never on a kernel stack) */
} kfwc_binding;

/* Mailbox chain outcome. Also reported in KFWC_EV_MAILBOX_XCHECK. */
#define KFWC_MB_OK 0u
#define KFWC_MB_UNREADABLE 1u      /* BAR0 not mapped / not in D0 / all-ones */
#define KFWC_MB_ZERO 2u            /* both halves zero: boot args not published (or other regime) */
#define KFWC_MB_SUSPENDED 3u       /* MAILBOX0 == 0x80000000 */
#define KFWC_MB_BAD_ADDR 4u        /* unaligned or outside RAM */
#define KFWC_MB_NO_RMARGS 5u       /* array readable but no RMARGS entry */
#define KFWC_MB_ARGS_BAD 6u        /* init args out of bounds / inconsistent */
#define KFWC_MB_REGION_BAD 7u      /* page table failed validation */
#define KFWC_MB_QUEUE_BAD 8u       /* queue header failed validation */

typedef struct kfwc_mb_diag {
    kfwc_u32 status;           /* KFWC_MB_* */
    kfwc_u64 mailbox_pa;       /* value of MAILBOX1:MAILBOX0 when readable */
    kfwc_u64 shared_pa;        /* from the init args when reached */
} kfwc_mb_diag;

/* Validate a 32-byte msgqTxHeader. Returns 1 and fills g (valid=1) when it is acceptable;
 * 0 otherwise. allow_zero: an all-zero header is "not yet published" and returns 2. */
int kfwc_tx_header_check(const kfwc_u8 *hdr32, kfwc_qgeom *g, int allow_zero);

/* Load and validate the region described by (table_pa, pte_count) with the two queue offsets.
 * Returns 0 on success (b->q[0].valid must be 1, b->q[1].valid may be 0 until published). */
int kfwc_bind_region(const kfwc_platform *p, kfwc_binding *b, kfwc_u64 table_pa, kfwc_u32 pte_count,
                     kfwc_u32 cmd_off, kfwc_u32 stat_off);

/* Option (a). Returns 0 and a filled binding on success. diag always filled. */
int kfwc_discover_mailbox(const kfwc_platform *p, kfwc_binding *b, kfwc_mb_diag *diag);

/* Option (b) building blocks. `page` is a 4096 byte buffer holding the page at pa.
 * Cheap signature test: entry 0 names the page itself and entry 1 is a nonzero aligned address. */
int kfwc_scan_signature(kfwc_u64 pa, const kfwc_u8 *page);
/* Strict validation of a candidate table at pa (reads RAM). Fills the binding on success.
 * Infers the layout the producer used: command queue at region offset 4096, status queue
 * directly after it. Returns 0 on success. */
int kfwc_bind_candidate(const kfwc_platform *p, kfwc_binding *b, kfwc_u64 pa);

/* Read len bytes at region offset `off` through the binding's page list. 0 on success. */
int kfwc_region_read(const kfwc_platform *p, const kfwc_binding *b, kfwc_u32 off, void *dst,
                     kfwc_u32 len);

#endif /* KFWC_DISCOVER_H */
