/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * The capture engine: discovery + queue walker + loss accounting, platform independent.
 * The Windows driver owns one kfwc_engine and calls kfwc_engine_step() from a worker thread;
 * the Linux tests drive the same code against synthetic memory.
 *
 * Output: KFGSPCAP v1 records (kfwc_record.h) handed to a sink, byte for byte what the
 * collector appends to the capture file after the 128-byte file header. The engine copies
 * RAW ring entries. It reads a few element fields (elemCount, length, seqNum, checksum) only
 * to find where a message starts and to account for loss; it never alters, filters or
 * reorders what it copies.
 *
 * Contract: the engine only calls the read-only operations of kfwc_platform. It never blocks,
 * never allocates, and one step is bounded (scan_bytes_per_step of RAM, max_msgs_per_step
 * messages per direction).
 *
 * Loss. Every entry the engine did not copy is announced by a LOSS record in stream order, or
 * (when even that could not be stored) is visible as a gap in the record counter, which is
 * assigned before the record is offered to the sink. Detection uses, in this order:
 *   1. ring pointers: the published writePtr is the only entry range the engine copies, and
 *      a stale position (the first entry at our position no longer starts a message) is a
 *      re-synchronisation, announced as LOSS_UNKNOWN;
 *   2. the per-message transport seqNum inside the element header (the census measured it
 *      continuous from 0 in all five real captures; rpc.sequence is NOT used, it is 0 in every
 *      Windows 580.88 message): a jump is announced and its size is kept in the statistics;
 *   3. the checksum: an element that stays unreadable is LOSS_UNREADABLE;
 *   4. the sink: a refused record is LOSS_OVERFLOW with an exact entry count, written before
 *      the next stored record.
 * The decoder repeats checks 2 independently, so a loss announced here is counted twice at
 * worst and a loss missed here is still caught there.
 */
#ifndef KFWC_ENGINE_H
#define KFWC_ENGINE_H

#include "kfwc_discover.h"
#include "kfwc_record.h"

#define KFWC_ENGINE_ABI 3u
#define KFWC_SCAN_CHUNK (1024u * 1024u)
#define KFWC_MAX_CHAIN 256u
#define KFWC_EVENT_SLOTS 64u

typedef struct kfwc_sink {
    void *ctx;
    /* Accept one serialized KFGSPCAP record. Return 0 when stored, nonzero when full. */
    int (*put)(void *ctx, const kfwc_u8 *rec, kfwc_u32 len);
} kfwc_sink;

typedef struct kfwc_config {
    kfwc_u32 layout;                /* 0 = auto-detect, or KFWC_LAYOUT_* */
    kfwc_u32 use_mailbox;           /* option (a) */
    kfwc_u32 use_scan;              /* option (b) */
    kfwc_u64 scan_delay_ticks;      /* wait this long after start before scanning when the mailbox reads zero */
    kfwc_u64 mailbox_period_ticks;  /* re-read the mailbox this often while searching */
    kfwc_u32 scan_bytes_per_step;   /* 4096..KFWC_SCAN_CHUNK; the rate limit is this per step */
    kfwc_u32 max_msgs_per_step;     /* per direction */
    kfwc_u32 mapping_check_steps;   /* re-verify the page list every N steps (0: every step) */
    /* Scan back-off. A scan that completes a full pass over RAM without attaching pauses for
     * scan_backoff_initial_ticks, then twice that after the next pass, ... capped at
     * scan_backoff_max_ticks. The back-off restarts from the initial value, and the scan resumes
     * from the beginning at once, when the mailbox pair changes (a GSP boot, a suspend/resume, a
     * BAR going readable): that is the only event after which a new queue can appear.
     * scan_backoff_initial_ticks == 0 disables the back-off (passes repeat back to back). */
    kfwc_u64 scan_backoff_initial_ticks;
    kfwc_u64 scan_backoff_max_ticks;
} kfwc_config;

typedef struct kfwc_stats {          /* counters monotonic (scan_backoff_ticks is a level); 33 x u64 */
    kfwc_u64 abi, steps, state, generation;
    kfwc_u64 scan_bytes, scan_passes, scan_candidates, attaches, detaches;
    kfwc_u64 messages[2], payload_bytes;
    kfwc_u64 loss_records, lost_messages_exact, loss_events_unknown;
    kfwc_u64 torn_reads, invalid_elements, unsupported, read_failures, resyncs;
    kfwc_u64 sink_dropped, sink_loss_pending;
    kfwc_u64 mailbox_reads, mailbox_status, method, layout, records_refused;
    kfwc_u64 seq_gap_messages, lapped_messages, next_counter, events_lost;
    kfwc_u64 scan_backoff_resets;   /* mailbox pair changed while searching: back-off restarted */
    kfwc_u64 scan_backoff_ticks;    /* the pause that follows the NEXT completed pass (level, not a counter) */
} kfwc_stats;

/* Diagnostic events (not part of the capture file; read through the driver's EVENTS ioctl and
 * written into the cell sidecar). 64 bytes each, little-endian fields. */
#define KFWC_EV_START 1u        /* data: u32 engine_abi, layout, use_mailbox, use_scan, reg_mapped, pci_device_id */
#define KFWC_EV_ATTACH 2u       /* data: u32 method, layout, pages, cmd_bytes, stat_bytes, msg_size, cmd_valid, stat_valid; u64 table_pa, libos_pa */
#define KFWC_EV_DETACH 3u       /* data: u32 reason; u64 messages emitted */
#define KFWC_EV_XCHECK 4u       /* data: u32 mailbox_status, matches; u64 mailbox_pa, shared_pa */
typedef struct kfwc_event {
    kfwc_u32 code, generation;
    kfwc_u64 ts;
    kfwc_u8 data[48];
} kfwc_event;
KFWC_STATIC_ASSERT(kfwc_event_is_64, sizeof(kfwc_event) == 64);

#define KFWC_DETACH_HEADER_INVALID 1u
#define KFWC_DETACH_MAPPING_CHANGED 2u
#define KFWC_DETACH_READ_FAILURE 3u
#define KFWC_DETACH_STOPPED 4u

typedef struct kfwc_dir {
    kfwc_u32 have_pos;       /* next_slot is meaningful */
    kfwc_u32 seq_known;      /* next_seq is meaningful */
    kfwc_u32 next_slot, next_seq;
    kfwc_u32 prefix_pending; /* the next copied message starts a stream: announce a missing prefix unless seq == 0 */
    kfwc_u32 bound;          /* BIND record stored for this generation */
    kfwc_u32 stall;          /* consecutive polls that could not progress */
    kfwc_u32 read_fail;      /* consecutive header read failures */
    kfwc_u32 sink_lost, sink_first_entry; /* entries refused by a full sink, not yet announced */
    kfwc_u8 hdr[KFWC_TXH_BYTES];          /* header bytes as last read (BIND payload) */
} kfwc_dir;

typedef struct kfwc_engine {
    kfwc_platform plat;
    kfwc_sink sink;
    kfwc_config cfg;
    kfwc_stats stats;
    kfwc_u32 state;                 /* KFWC_ST_* */
    kfwc_u32 generation;            /* also the record queue_id */
    kfwc_u32 mb_status;
    kfwc_u32 counter;               /* next record counter */
    kfwc_u32 ended;
    kfwc_u64 start_ticks, last_mailbox_ticks;
    kfwc_u64 scan_pause_until;      /* no scan step before this tick (0: none) */
    kfwc_u64 scan_backoff_cur;      /* pause after the next completed pass */
    kfwc_u64 mb_sig_pa;             /* last mailbox signature seen while searching */
    kfwc_u32 mb_sig_status, mb_sig_seen;
    kfwc_u32 range_idx;
    kfwc_u64 range_off;
    kfwc_u32 steps_since_map_check;
    kfwc_u32 ev_head, ev_count;
    kfwc_event events[KFWC_EVENT_SLOTS];
    kfwc_dir dir[2];
    kfwc_binding bind;
    kfwc_binding tmp;               /* cross-check scratch */
    kfwc_u32 chain_slot[KFWC_MAX_CHAIN], chain_elems[KFWC_MAX_CHAIN], chain_seq[KFWC_MAX_CHAIN];
    kfwc_u8 head[128];
    kfwc_u8 run[KFWC_ELEM_SIZE_MAX];
    kfwc_u8 out[KFWC_CAP_REC_HEADER_BYTES + KFWC_ELEM_SIZE_MAX + 64];
    kfwc_u8 scan_buf[KFWC_SCAN_CHUNK];
} kfwc_engine;

#define KFWC_ST_SEARCHING 0u
#define KFWC_ST_ATTACHED 1u

void kfwc_config_defaults(kfwc_config *c, kfwc_u64 ticks_per_second);
void kfwc_engine_init(kfwc_engine *e, const kfwc_platform *p, const kfwc_sink *s, const kfwc_config *c);
/* One bounded step. Returns the number of capture records handed to the sink (0 = idle). */
kfwc_u32 kfwc_engine_step(kfwc_engine *e);
/* Clean stop: announces nothing for a clean detach, then writes the END record. */
void kfwc_engine_stop(kfwc_engine *e);
/* 1 when the next step would spend its time scanning RAM (searching, scan enabled, past the
 * mailbox-zero delay and not in a back-off pause); the driver uses it to pick its sleep. */
int kfwc_engine_scan_due(kfwc_engine *e);
/* Record the START diagnostic event (done once by the driver after PCI discovery). */
void kfwc_engine_note_start(kfwc_engine *e, kfwc_u32 pci_device_id, kfwc_u32 reg_mapped);
/* Copy up to max events out (oldest first) and remove them. Returns the count. */
kfwc_u32 kfwc_engine_events(kfwc_engine *e, kfwc_event *out, kfwc_u32 max);

#endif /* KFWC_ENGINE_H */
