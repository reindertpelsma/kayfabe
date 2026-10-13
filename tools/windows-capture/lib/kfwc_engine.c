/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * See kfwc_engine.h for the contract. Read-only: this file never calls anything that stores to
 * RAM, registers or PCI config (the platform interface has no such method).
 */
#include "kfwc_engine.h"
#include <string.h>

KFWC_STATIC_ASSERT(kfwc_stats_is_31_words, sizeof(kfwc_stats) == 31 * 8);

#define MAX_READ_FAIL 8u
#define MAX_STALL 8u
#define SEQ_NEWER(a, b) ((int)((kfwc_u32)(a) - (kfwc_u32)(b)) > 0)
#define SEQ_OLDER(a, b) ((int)((kfwc_u32)(a) - (kfwc_u32)(b)) < 0)

/* Reset a direction's cursor but keep the not-yet-announced sink drops: they must still be
 * reported by a LOSS_OVERFLOW record after a detach/attach cycle. */
static void reset_dir(kfwc_dir *s)
{
    kfwc_u32 lost = s->sink_lost, first = s->sink_first_entry;
    memset(s, 0, sizeof(*s));
    s->sink_lost = lost;
    s->sink_first_entry = first;
}

static kfwc_u64 now_ticks(kfwc_engine *e) { return e->plat.now(e->plat.ctx); }

/* ------------------------------------------------------------------------------------- */
/* diagnostic events                                                                        */
/* ------------------------------------------------------------------------------------- */

static void ev_push(kfwc_engine *e, kfwc_u32 code, const kfwc_u8 *data, kfwc_u32 len)
{
    kfwc_event *ev;
    if (e->ev_count == KFWC_EVENT_SLOTS) {
        e->stats.events_lost++;
        return;
    }
    ev = &e->events[(e->ev_head + e->ev_count) % KFWC_EVENT_SLOTS];
    e->ev_count++;
    memset(ev, 0, sizeof(*ev));
    ev->code = code;
    ev->generation = e->generation;
    ev->ts = now_ticks(e);
    if (len > sizeof(ev->data))
        len = (kfwc_u32)sizeof(ev->data);
    memcpy(ev->data, data, len);
}

kfwc_u32 kfwc_engine_events(kfwc_engine *e, kfwc_event *out, kfwc_u32 max)
{
    kfwc_u32 n = 0;
    while (n < max && e->ev_count) {
        out[n++] = e->events[e->ev_head];
        e->ev_head = (e->ev_head + 1) % KFWC_EVENT_SLOTS;
        e->ev_count--;
    }
    return n;
}

void kfwc_engine_note_start(kfwc_engine *e, kfwc_u32 pci_device_id, kfwc_u32 reg_mapped)
{
    kfwc_u8 b[24];
    memset(b, 0, sizeof(b));
    kfwc_put32(b + 0, KFWC_ENGINE_ABI);
    kfwc_put32(b + 4, e->cfg.layout);
    kfwc_put32(b + 8, e->cfg.use_mailbox);
    kfwc_put32(b + 12, e->cfg.use_scan);
    kfwc_put32(b + 16, reg_mapped);
    kfwc_put32(b + 20, pci_device_id);
    ev_push(e, KFWC_EV_START, b, sizeof(b));
}

/* ------------------------------------------------------------------------------------- */
/* record emission                                                                          */
/* ------------------------------------------------------------------------------------- */

/* Build and offer one record. The counter is consumed first: a record the sink refuses leaves
 * a hole in the counter, which the decoder reports even if no loss record can be stored. */
static int put_rec(kfwc_engine *e, kfwc_u32 kind, kfwc_u32 dir, kfwc_u32 first, kfwc_u32 count,
                   const void *payload, kfwc_u32 plen, kfwc_u32 *emitted)
{
    kfwc_cap_rec r;
    kfwc_u32 n;
    r.kind = kind;
    r.dir = dir;
    r.queue_id = e->generation & 0xFFFFu;
    r.payload_bytes = plen;
    r.counter = e->counter++;
    r.ts = now_ticks(e);
    r.first_entry = first;
    r.entry_count = count;
    e->stats.next_counter = e->counter;
    n = kfwc_cap_rec_encode(&r, payload, e->out, (kfwc_u32)sizeof(e->out));
    if (!n || e->sink.put(e->sink.ctx, e->out, n)) {
        e->stats.records_refused++;
        return 1;
    }
    (*emitted)++;
    return 0;
}

/* Announce entries the sink refused. Called before every other emission. */
static void flush_sink_loss(kfwc_engine *e, kfwc_u32 *emitted)
{
    kfwc_u32 d;
    for (d = 0; d < 2; d++) {
        kfwc_dir *s = &e->dir[d];
        if (s->sink_lost &&
            put_rec(e, KFWC_K_LOSS_OVERFLOW, d, s->sink_first_entry, s->sink_lost, 0, 0, emitted) == 0) {
            e->stats.loss_records++;
            s->sink_lost = 0;
        }
    }
    e->stats.sink_loss_pending = e->dir[0].sink_lost + e->dir[1].sink_lost;
}

static void note_sink_loss(kfwc_engine *e, kfwc_u32 d, kfwc_u32 first, kfwc_u32 count)
{
    kfwc_dir *s = &e->dir[d];
    if (!s->sink_lost)
        s->sink_first_entry = first;
    s->sink_lost += count;
    e->stats.sink_dropped += count;
    e->stats.sink_loss_pending = e->dir[0].sink_lost + e->dir[1].sink_lost;
}

static void emit_loss(kfwc_engine *e, kfwc_u32 kind, kfwc_u32 d, kfwc_u32 first, kfwc_u32 count,
                      kfwc_u32 *emitted)
{
    flush_sink_loss(e, emitted);
    if (put_rec(e, kind, d, first, count, 0, 0, emitted) == 0) {
        e->stats.loss_records++;
        if (count == KFWC_UNKNOWN_COUNT)
            e->stats.loss_events_unknown++;
    }
}

static int bind_dir(kfwc_engine *e, kfwc_u32 d, kfwc_u32 *emitted)
{
    kfwc_dir *s = &e->dir[d];
    if (s->bound)
        return 0;
    flush_sink_loss(e, emitted);
    if (put_rec(e, KFWC_K_BIND, d, 0, 0, s->hdr, KFWC_TXH_BYTES, emitted))
        return 1;
    s->bound = 1;
    return 0;
}

/* Copy one verified message (its raw entries are in e->run) as ELEMENTS record(s), split at the
 * ring end. The first message of a stream whose seq is not 0 announces the unseen prefix. */
static void emit_entries(kfwc_engine *e, kfwc_u32 d, kfwc_u32 slot, const kfwc_elem_info *info,
                         kfwc_u32 *emitted)
{
    kfwc_dir *s = &e->dir[d];
    kfwc_u32 count = e->bind.q[d].count, n1, n2;
    flush_sink_loss(e, emitted);
    if (bind_dir(e, d, emitted)) {
        note_sink_loss(e, d, slot, info->elems);
        return;
    }
    if (s->prefix_pending) {
        s->prefix_pending = 0;
        if (info->seq != 0)
            emit_loss(e, KFWC_K_LOSS_UNKNOWN, d, slot, KFWC_UNKNOWN_COUNT, emitted);
    }
    n1 = info->elems;
    if (slot + n1 > count)
        n1 = count - slot;
    n2 = info->elems - n1;
    if (put_rec(e, KFWC_K_ELEMENTS, d, slot, n1, e->run, n1 * KFWC_PAGE, emitted)) {
        note_sink_loss(e, d, slot, info->elems);
        return;
    }
    if (n2 && put_rec(e, KFWC_K_ELEMENTS, d, 0, n2, e->run + n1 * KFWC_PAGE, n2 * KFWC_PAGE, emitted)) {
        note_sink_loss(e, d, 0, n2);
        return;
    }
    e->stats.messages[d]++;
    e->stats.payload_bytes += info->elems * KFWC_PAGE;
}

/* ------------------------------------------------------------------------------------- */
/* reading queue elements                                                                   */
/* ------------------------------------------------------------------------------------- */

static int read_elem_part(kfwc_engine *e, kfwc_u32 d, kfwc_u32 slot, kfwc_u32 off, void *dst,
                          kfwc_u32 len)
{
    const kfwc_qgeom *q = &e->bind.q[d];
    kfwc_u32 region_off = q->off + q->entry_off + slot * q->msg_size + off;
    return kfwc_region_read(&e->plat, &e->bind, region_off, dst, len);
}

static int read_run(kfwc_engine *e, kfwc_u32 d, kfwc_u32 slot, kfwc_u32 elems)
{
    const kfwc_qgeom *q = &e->bind.q[d];
    kfwc_u32 i;
    for (i = 0; i < elems; i++)
        if (read_elem_part(e, d, (slot + i) % q->count, 0, e->run + i * KFWC_PAGE, KFWC_PAGE))
            return 1;
    return 0;
}

/* Peek the first element at `slot` with the pinned layout, or try every layout when unpinned. */
static int peek_slot(kfwc_engine *e, kfwc_u32 d, kfwc_u32 slot, kfwc_elem_info *info)
{
    kfwc_u32 order[2], i, n = 0;
    int result = KFWC_ELEM_INVALID;
    if (read_elem_part(e, d, slot, 0, e->head, sizeof(e->head))) {
        e->stats.read_failures++;
        return -1;
    }
    if (e->bind.layout)
        order[n++] = e->bind.layout;
    else {
        order[n++] = KFWC_LAYOUT_PRE610;
        order[n++] = KFWC_LAYOUT_610;
    }
    for (i = 0; i < n; i++) {
        int r = kfwc_elem_peek(kfwc_layout_get(order[i]), e->head, info);
        if (r == KFWC_ELEM_OK)
            return KFWC_ELEM_OK;
        if (r == KFWC_ELEM_UNSUPPORTED)
            result = r;
    }
    return result;
}

/* Read the whole run and verify its checksum, retrying once for a torn copy.
 * 0 = verified (e->run holds it); 1 = unstable; -1 = read failure. */
static int load_verified(kfwc_engine *e, kfwc_u32 d, kfwc_u32 slot, const kfwc_elem_info *info)
{
    kfwc_u32 attempt;
    const kfwc_layout *l = kfwc_layout_get(info->layout);
    for (attempt = 0; attempt < 2; attempt++) {
        if (read_run(e, d, slot, info->elems)) {
            e->stats.read_failures++;
            return -1;
        }
        if (kfwc_elem_checksum_ok(l, e->run, info))
            return 0;
        e->stats.torn_reads++;
    }
    return 1;
}

/* ------------------------------------------------------------------------------------- */
/* attach / detach                                                                          */
/* ------------------------------------------------------------------------------------- */

static void do_attach(kfwc_engine *e)
{
    kfwc_u8 b[48];
    kfwc_u32 d;
    const kfwc_binding *bd = &e->bind;
    e->generation++;
    e->state = KFWC_ST_ATTACHED;
    e->stats.attaches++;
    e->stats.method = bd->method;
    e->steps_since_map_check = 0;
    e->bind.layout = e->cfg.layout; /* 0: decided by the first verified element */
    for (d = 0; d < 2; d++) {
        reset_dir(&e->dir[d]);
        e->dir[d].prefix_pending = 1;
    }
    memset(b, 0, sizeof(b));
    kfwc_put32(b + 0, bd->method);
    kfwc_put32(b + 4, bd->layout);
    kfwc_put32(b + 8, bd->pages);
    kfwc_put32(b + 12, bd->q[0].size);
    kfwc_put32(b + 16, bd->q[1].valid ? bd->q[1].size : bd->q[0].size);
    kfwc_put32(b + 20, bd->q[0].msg_size);
    kfwc_put32(b + 24, bd->q[0].valid);
    kfwc_put32(b + 28, bd->q[1].valid);
    kfwc_put64(b + 32, bd->table_pa);
    kfwc_put64(b + 40, bd->libos_args_pa);
    ev_push(e, KFWC_EV_ATTACH, b, sizeof(b));

    /* When the scan found the queues first, the mailbox chain is a free cross-check of
     * option (a) on this very machine (the evidence the design is missing today). */
    if (bd->method == KFWC_METHOD_SCAN && e->cfg.use_mailbox) {
        kfwc_mb_diag diag;
        kfwc_u8 x[24];
        int rc = kfwc_discover_mailbox(&e->plat, &e->tmp, &diag);
        kfwc_put32(x + 0, diag.status);
        kfwc_put32(x + 4, rc == 0 && e->tmp.table_pa == e->bind.table_pa);
        kfwc_put64(x + 8, diag.mailbox_pa);
        kfwc_put64(x + 16, diag.shared_pa);
        ev_push(e, KFWC_EV_XCHECK, x, sizeof(x));
    }
}

static void do_detach(kfwc_engine *e, kfwc_u32 reason, kfwc_u32 *emitted)
{
    kfwc_u8 buf[16];
    kfwc_u32 d;
    if (reason != KFWC_DETACH_STOPPED)
        for (d = 0; d < 2; d++)
            if (e->dir[d].bound)
                emit_loss(e, KFWC_K_LOSS_UNKNOWN, d, e->dir[d].have_pos ? e->dir[d].next_slot : 0,
                          KFWC_UNKNOWN_COUNT, emitted);
    memset(buf, 0, sizeof(buf));
    kfwc_put32(buf + 0, reason);
    kfwc_put64(buf + 8, e->stats.messages[0] + e->stats.messages[1]);
    ev_push(e, KFWC_EV_DETACH, buf, sizeof(buf));
    e->stats.detaches++;
    e->state = KFWC_ST_SEARCHING;
    e->start_ticks = now_ticks(e);
    e->last_mailbox_ticks = 0;
    e->range_idx = 0;
    e->range_off = 0;
    reset_dir(&e->dir[0]);
    reset_dir(&e->dir[1]);
}

/* ------------------------------------------------------------------------------------- */
/* chain reconstruction (bootstrap and resync)                                              */
/* ------------------------------------------------------------------------------------- */

/* Walk backwards from the producer position `wp` collecting a contiguous suffix of intact
 * messages (each message ends exactly where its successor starts, seq decreasing by one).
 * Index 0 is the newest. Returns the number of messages. */
static kfwc_u32 build_chain(kfwc_engine *e, kfwc_u32 d, kfwc_u32 wp)
{
    const kfwc_qgeom *q = &e->bind.q[d];
    kfwc_u32 n = 0, end = wp, used = 0, want_seq = 0, have_want = 0;
    while (n < KFWC_MAX_CHAIN && used < q->count) {
        kfwc_u32 k, found = 0, f_slot = 0, f_elems = 0, f_seq = 0;
        for (k = 1; k <= KFWC_MAX_RUN_ELEMS && used + k <= q->count; k++) {
            kfwc_elem_info info;
            kfwc_u32 slot = (end + q->count - (k % q->count)) % q->count;
            int pr = peek_slot(e, d, slot, &info);
            if (pr != KFWC_ELEM_OK || info.elems != k)
                continue;
            if (have_want && info.seq != want_seq)
                continue;
            if (load_verified(e, d, slot, &info) != 0)
                continue;
            if (!found || SEQ_NEWER(info.seq, f_seq)) {
                found = 1;
                f_slot = slot;
                f_elems = k;
                f_seq = info.seq;
            }
        }
        if (!found)
            break;
        e->chain_slot[n] = f_slot;
        e->chain_elems[n] = f_elems;
        e->chain_seq[n] = f_seq;
        n++;
        used += f_elems;
        end = f_slot;
        want_seq = f_seq - 1u;
        have_want = 1;
    }
    return n;
}

/* Copy chain messages from index `lo` (oldest wanted) down to 0 (newest), oldest first. */
static void emit_chain(kfwc_engine *e, kfwc_u32 d, kfwc_u32 lo, kfwc_u32 *emitted)
{
    kfwc_u32 i = lo + 1;
    while (i--) {
        kfwc_elem_info info;
        int pr = peek_slot(e, d, e->chain_slot[i], &info);
        if (pr == KFWC_ELEM_OK && info.seq == e->chain_seq[i] &&
            load_verified(e, d, e->chain_slot[i], &info) == 0) {
            e->bind.layout = info.layout;
            emit_entries(e, d, e->chain_slot[i], &info, emitted);
        } else {
            emit_loss(e, KFWC_K_LOSS_UNREADABLE, d, e->chain_slot[i], e->chain_elems[i], emitted);
            e->stats.torn_reads++;
        }
        e->dir[d].next_seq = e->chain_seq[i] + 1u;
        e->dir[d].seq_known = 1;
    }
}

static void resync(kfwc_engine *e, kfwc_u32 d, kfwc_u32 wp, kfwc_u32 *emitted)
{
    kfwc_dir *s = &e->dir[d];
    kfwc_u32 n = build_chain(e, d, wp), lo;
    e->stats.resyncs++;
    if (!n) {
        /* No intact message ends at the write pointer. That is "nothing yet" only for a ring that
         * was never written (wp == 0 at first sight); otherwise entries exist that we cannot
         * parse, and skipping them silently would hide them. */
        if (s->seq_known || wp != 0) {
            emit_loss(e, KFWC_K_LOSS_UNKNOWN, d, s->have_pos ? s->next_slot : wp, KFWC_UNKNOWN_COUNT, emitted);
            s->prefix_pending = 0; /* announced just now */
        }
        s->have_pos = 1;
        s->next_slot = wp;
        s->seq_known = 0;
        s->stall = 0;
        return;
    }
    lo = n - 1;
    if (!s->seq_known) {
        /* First sight of this queue, or a restart after an announced loss: copy everything
         * retained. emit_entries announces the unseen prefix unless the oldest seq is 0. */
    } else if (e->chain_seq[0] + 1u == s->next_seq) {
        /* Nothing new: the newest retained message is the one copied last. */
        s->have_pos = 1;
        s->next_slot = wp;
        s->stall = 0;
        return;
    } else if (SEQ_OLDER(e->chain_seq[0], s->next_seq)) {
        /* The producer's sequence went backwards (queue restarted in place). */
        emit_loss(e, KFWC_K_LOSS_UNKNOWN, d, s->next_slot, KFWC_UNKNOWN_COUNT, emitted);
        s->prefix_pending = 0;
    } else {
        while (lo > 0 && SEQ_OLDER(e->chain_seq[lo], s->next_seq))
            lo--;
        if (SEQ_NEWER(e->chain_seq[lo], s->next_seq)) {
            /* Messages next_seq .. chain_seq[lo]-1 left the ring before we copied them. */
            if (e->chain_slot[lo] == s->next_slot)
                e->stats.seq_gap_messages += e->chain_seq[lo] - s->next_seq;   /* numbers skipped in place */
            else
                e->stats.lapped_messages += e->chain_seq[lo] - s->next_seq;    /* history left the ring */
            e->stats.lost_messages_exact += e->chain_seq[lo] - s->next_seq;
            emit_loss(e, KFWC_K_LOSS_UNKNOWN, d, s->next_slot, KFWC_UNKNOWN_COUNT, emitted);
        }
    }
    emit_chain(e, d, lo, emitted);
    s->have_pos = 1;
    s->next_slot = wp;
    s->stall = 0;
}

/* ------------------------------------------------------------------------------------- */
/* following a queue                                                                        */
/* ------------------------------------------------------------------------------------- */

static void follow(kfwc_engine *e, kfwc_u32 d, kfwc_u32 wp, kfwc_u32 *emitted)
{
    kfwc_dir *s = &e->dir[d];
    const kfwc_qgeom *q = &e->bind.q[d];
    kfwc_u32 budget = e->cfg.max_msgs_per_step;
    while (budget) {
        kfwc_u32 avail = (wp + q->count - s->next_slot) % q->count;
        kfwc_elem_info info;
        int pr, vr;
        if (!avail) {
            s->stall = 0;
            return;
        }
        pr = peek_slot(e, d, s->next_slot, &info);
        if (pr < 0)
            return; /* read failure counted; retry next step */
        if (pr == KFWC_ELEM_UNSUPPORTED) {
            /* Well-formed envelope but CC authentication bytes: the entries are still copied
             * raw (the decoder says what they are); only the run length is needed here. */
            if (info.elems > avail || (s->seq_known && info.seq != s->next_seq)) {
                e->stats.invalid_elements++;
                resync(e, d, wp, emitted);
                return;
            }
            if (read_run(e, d, s->next_slot, info.elems)) {
                e->stats.read_failures++;
                return;
            }
            e->stats.unsupported++;
            emit_entries(e, d, s->next_slot, &info, emitted);
            s->next_slot = (s->next_slot + info.elems) % q->count;
            s->next_seq = info.seq + 1u;
            s->seq_known = 1;
            budget--;
            continue;
        }
        if (pr != KFWC_ELEM_OK) {
            e->stats.invalid_elements++;
            resync(e, d, wp, emitted);
            return;
        }
        if (s->seq_known && SEQ_OLDER(info.seq, s->next_seq)) {
            /* A stale element where we expected a new one: our position is wrong. */
            e->stats.invalid_elements++;
            resync(e, d, wp, emitted);
            return;
        }
        if (info.elems > avail) {
            /* The run extends past the published write pointer: not published yet. */
            if (++s->stall >= MAX_STALL)
                resync(e, d, wp, emitted);
            return;
        }
        vr = load_verified(e, d, s->next_slot, &info);
        if (vr < 0)
            return;
        if (vr > 0) {
            /* Unstable or corrupt: retry next step; after MAX_STALL polls re-synchronise on
             * the ring, which accounts the unreadable message through the seq arithmetic.
             * It is not announced twice. */
            if (++s->stall >= MAX_STALL)
                resync(e, d, wp, emitted);
            return;
        }
        if (s->seq_known && SEQ_NEWER(info.seq, s->next_seq)) {
            /* A valid message whose number jumps ahead: either the producer skipped numbers
             * or it lapped us. The chain from the write pointer tells which (resync). */
            resync(e, d, wp, emitted);
            return;
        }
        e->bind.layout = info.layout;
        emit_entries(e, d, s->next_slot, &info, emitted);
        s->next_slot = (s->next_slot + info.elems) % q->count;
        s->next_seq = info.seq + 1u;
        s->seq_known = 1;
        s->stall = 0;
        budget--;
    }
}

static int check_mapping(kfwc_engine *e)
{
    /* Re-read the page table and compare with the pages we bound. */
    kfwc_u32 i;
    if (e->plat.read_ram(e->plat.ctx, e->bind.table_pa, e->tmp.scratch, KFWC_PAGE))
        return -1;
    for (i = 0; i < e->bind.pages; i++)
        if (kfwc_get64(e->tmp.scratch + 8u * i) != e->bind.ptes[i])
            return 1;
    return 0;
}

static void poll_dir(kfwc_engine *e, kfwc_u32 d, kfwc_u32 *emitted)
{
    kfwc_binding *b = &e->bind;
    kfwc_qgeom *q = &b->q[d];
    kfwc_u8 h[KFWC_TXH_BYTES];
    kfwc_qgeom g;
    kfwc_dir *s = &e->dir[d];
    int r;

    if (kfwc_region_read(&e->plat, b, q->off, h, sizeof(h))) {
        e->stats.read_failures++;
        if (++s->read_fail >= MAX_READ_FAIL)
            do_detach(e, KFWC_DETACH_READ_FAILURE, emitted);
        return;
    }
    s->read_fail = 0;
    memset(&g, 0, sizeof(g));
    r = kfwc_tx_header_check(h, &g, !q->valid);
    if (r == 2)
        return; /* still unpublished (status queue before the firmware starts) */
    if (r == 0 || (q->valid && (g.size != q->size || g.count != q->count || g.msg_size != q->msg_size ||
                                g.rx_hdr_off != q->rx_hdr_off || g.entry_off != q->entry_off))) {
        do_detach(e, KFWC_DETACH_HEADER_INVALID, emitted);
        return;
    }
    if (!q->valid) {
        if ((kfwc_u64)q->off + g.size > (kfwc_u64)b->pages * KFWC_PAGE) {
            do_detach(e, KFWC_DETACH_HEADER_INVALID, emitted);
            return;
        }
        g.off = q->off;
        *q = g;
    }
    q->write_ptr = g.write_ptr;
    if (!s->bound)
        memcpy(s->hdr, h, sizeof(h)); /* the BIND payload: the header exactly as first seen */
    if (!s->bound && bind_dir(e, d, emitted))
        return; /* sink full: retry the BIND before any entries */
    if (!s->have_pos) {
        resync(e, d, g.write_ptr, emitted);
        return;
    }
    follow(e, d, g.write_ptr, emitted);
}

/* ------------------------------------------------------------------------------------- */
/* searching                                                                                */
/* ------------------------------------------------------------------------------------- */

static int scan_step(kfwc_engine *e)
{
    kfwc_u32 nranges = e->plat.ram_range_count(e->plat.ctx);
    kfwc_u32 budget = e->cfg.scan_bytes_per_step;
    if (budget > KFWC_SCAN_CHUNK)
        budget = KFWC_SCAN_CHUNK;
    budget &= ~(KFWC_PAGE - 1u);
    if (!budget || !nranges)
        return 0;
    while (budget) {
        kfwc_u64 base, len, pa, left;
        kfwc_u32 n, i;
        if (e->range_idx >= nranges) {
            e->range_idx = 0;
            e->range_off = 0;
            e->stats.scan_passes++;
            return 0; /* one pass boundary per step at most */
        }
        if (e->plat.ram_range(e->plat.ctx, e->range_idx, &base, &len) || (base & (KFWC_PAGE - 1))) {
            e->range_idx++;
            e->range_off = 0;
            continue;
        }
        len &= ~(kfwc_u64)(KFWC_PAGE - 1);
        if (e->range_off >= len) {
            e->range_idx++;
            e->range_off = 0;
            continue;
        }
        pa = base + e->range_off;
        left = len - e->range_off;
        n = (kfwc_u32)(left < budget ? left : budget);
        if (e->plat.read_ram(e->plat.ctx, pa, e->scan_buf, n)) {
            e->stats.read_failures++;
        } else {
            for (i = 0; i < n; i += KFWC_PAGE) {
                if (!kfwc_scan_signature(pa + i, e->scan_buf + i))
                    continue;
                e->stats.scan_candidates++;
                if (kfwc_bind_candidate(&e->plat, &e->bind, pa + i) == 0) {
                    e->stats.scan_bytes += i + KFWC_PAGE;
                    e->range_off += i + KFWC_PAGE;
                    do_attach(e);
                    return 1;
                }
            }
        }
        e->range_off += n;
        e->stats.scan_bytes += n;
        budget -= n;
    }
    return 0;
}

static void search_step(kfwc_engine *e)
{
    kfwc_u64 now = now_ticks(e);
    int scan_allowed;
    if (e->cfg.use_mailbox &&
        (e->last_mailbox_ticks == 0 || now - e->last_mailbox_ticks >= e->cfg.mailbox_period_ticks)) {
        kfwc_mb_diag diag;
        e->last_mailbox_ticks = now ? now : 1;
        e->stats.mailbox_reads++;
        if (kfwc_discover_mailbox(&e->plat, &e->bind, &diag) == 0) {
            e->mb_status = KFWC_MB_OK;
            e->stats.mailbox_status = KFWC_MB_OK;
            do_attach(e);
            return;
        }
        e->mb_status = diag.status;
        e->stats.mailbox_status = diag.status;
    }
    if (!e->cfg.use_scan)
        return;
    /* A mailbox that reads zero is "not published yet or another boot regime": give the boot
     * a chance before spending CPU on a scan. Every other status lets the scan start now. */
    scan_allowed = !e->cfg.use_mailbox || e->mb_status != KFWC_MB_ZERO ||
                   now - e->start_ticks >= e->cfg.scan_delay_ticks;
    if (scan_allowed)
        (void)scan_step(e);
}

/* ------------------------------------------------------------------------------------- */
/* public API                                                                               */
/* ------------------------------------------------------------------------------------- */

void kfwc_config_defaults(kfwc_config *c, kfwc_u64 tps)
{
    memset(c, 0, sizeof(*c));
    c->layout = 0;
    c->use_mailbox = 1;
    c->use_scan = 1;
    c->scan_delay_ticks = 5 * tps;
    c->mailbox_period_ticks = tps / 100u ? tps / 100u : 1; /* 10 ms */
    c->scan_bytes_per_step = 1024u * 1024u;
    c->max_msgs_per_step = 64;
    c->mapping_check_steps = 64;
}

void kfwc_engine_init(kfwc_engine *e, const kfwc_platform *p, const kfwc_sink *s, const kfwc_config *c)
{
    memset(e, 0, sizeof(*e));
    e->plat = *p;
    e->sink = *s;
    e->cfg = *c;
    if (e->cfg.max_msgs_per_step == 0)
        e->cfg.max_msgs_per_step = 1;
    e->stats.abi = KFWC_ENGINE_ABI;
    e->start_ticks = now_ticks(e);
}

kfwc_u32 kfwc_engine_step(kfwc_engine *e)
{
    kfwc_u32 emitted = 0;
    if (e->ended)
        return 0;
    e->stats.steps++;
    flush_sink_loss(e, &emitted);
    if (e->state == KFWC_ST_SEARCHING) {
        search_step(e);
    } else {
        if (e->cfg.mapping_check_steps == 0 || ++e->steps_since_map_check >= e->cfg.mapping_check_steps) {
            int m = check_mapping(e);
            e->steps_since_map_check = 0;
            if (m > 0) {
                do_detach(e, KFWC_DETACH_MAPPING_CHANGED, &emitted);
                goto out;
            }
            if (m < 0)
                e->stats.read_failures++;
        }
        poll_dir(e, KFWC_DIR_CMD, &emitted);
        if (e->state == KFWC_ST_ATTACHED)
            poll_dir(e, KFWC_DIR_STAT, &emitted);
    }
out:
    e->stats.state = e->state;
    e->stats.generation = e->generation;
    e->stats.layout = e->bind.layout;
    return emitted;
}

void kfwc_engine_stop(kfwc_engine *e)
{
    kfwc_u32 emitted = 0;
    kfwc_u32 before;
    if (e->ended)
        return;
    if (e->state == KFWC_ST_ATTACHED)
        do_detach(e, KFWC_DETACH_STOPPED, &emitted);
    flush_sink_loss(e, &emitted);
    before = e->counter; /* END.entry_count = number of records the module produced before it */
    (void)put_rec(e, KFWC_K_END, KFWC_DIR_CMD, 0, before, 0, 0, &emitted);
    e->ended = 1;
}
