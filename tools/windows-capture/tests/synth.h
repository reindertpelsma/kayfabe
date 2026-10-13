/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * Synthetic machine for the Linux tests: fake RAM with scattered pages, fake BAR0 mailbox
 * registers, a producer that behaves like the driver/firmware side of a msgq pair, and a
 * byte-level replica of kayfabe's transport encoder (kf-gsp encode_message).
 *
 * The replica is deliberately written independently of lib/kfwc_elem.c (its own fold, its own
 * offsets) so that the walker is tested against a second implementation of the framing, and
 * tests/test_kayfabe_roundtrip.c then checks the replica itself against bytes produced by
 * the real kf-gsp encoder.
 */
#ifndef KFWC_SYNTH_H
#define KFWC_SYNTH_H

#include "../lib/kfwc_engine.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define RAM_BYTES (8u << 20)
#define MAX_RANGES 4

typedef struct fake {
    kfwc_u8 *ram;
    kfwc_u64 base[MAX_RANGES], len[MAX_RANGES], off[MAX_RANGES];
    kfwc_u32 nranges;
    kfwc_u32 boot0, mailbox0, mailbox1;
    int regs_ok;
    kfwc_u32 reg_log[64], reg_log_n;      /* every BAR0 offset the library asked for */
    kfwc_u64 ram_reads, ram_bytes, refused;
    kfwc_u64 ticks;
    /* fault injection */
    kfwc_u64 tear_pa;                      /* if nonzero: first read touching this page flips a byte */
    int tear_remaining;                    /* number of reads to tear (persistent tear = large) */
    kfwc_u64 fail_pa;                      /* reads touching this page fail */
    kfwc_u64 stores;                       /* must stay 0: nothing in the library can store */
} fake;

/* ---- platform ---------------------------------------------------------------------- */
static int fk_find(fake *f, kfwc_u64 pa, kfwc_u64 len, kfwc_u64 *ram_off)
{
    kfwc_u32 i;
    for (i = 0; i < f->nranges; i++)
        if (pa >= f->base[i] && pa - f->base[i] < f->len[i] && len <= f->len[i] - (pa - f->base[i])) {
            *ram_off = f->off[i] + (pa - f->base[i]);
            return 1;
        }
    return 0;
}
static int fk_read_ram(void *ctx, kfwc_u64 pa, void *dst, kfwc_u32 len)
{
    fake *f = (fake *)ctx;
    kfwc_u64 o;
    if (!len || !fk_find(f, pa, len, &o)) {
        f->refused++;
        return 1;
    }
    if (f->fail_pa && pa <= f->fail_pa + 4095 && pa + len > f->fail_pa)
        return 1;
    f->ram_reads++;
    f->ram_bytes += len;
    memcpy(dst, f->ram + o, len);
    if (f->tear_pa && f->tear_remaining > 0 && pa <= f->tear_pa + 4095 && pa + len > f->tear_pa) {
        /* the producer is rewriting this page while we copy it: our copy is torn */
        ((kfwc_u8 *)dst)[len / 2] ^= 0x5a;
        f->tear_remaining--;
    }
    return 0;
}
static int fk_ram_contains(void *ctx, kfwc_u64 pa, kfwc_u64 len)
{
    kfwc_u64 o;
    return fk_find((fake *)ctx, pa, len, &o);
}
static kfwc_u32 fk_range_count(void *ctx) { return ((fake *)ctx)->nranges; }
static int fk_range(void *ctx, kfwc_u32 i, kfwc_u64 *base, kfwc_u64 *len)
{
    fake *f = (fake *)ctx;
    if (i >= f->nranges)
        return 1;
    *base = f->base[i];
    *len = f->len[i];
    return 0;
}
static int fk_reg(void *ctx, kfwc_u32 off, kfwc_u32 *out)
{
    fake *f = (fake *)ctx;
    if (f->reg_log_n < 64)
        f->reg_log[f->reg_log_n++] = off;
    if (!f->regs_ok)
        return 1;
    *out = off == KFWC_REG_PMC_BOOT_0 ? f->boot0 : off == KFWC_REG_GSP_MAILBOX0 ? f->mailbox0
         : off == KFWC_REG_GSP_MAILBOX1 ? f->mailbox1 : 0xFFFFFFFFu;
    return 0;
}
static kfwc_u64 fk_now(void *ctx) { return ++((fake *)ctx)->ticks; }

static kfwc_platform fk_platform(fake *f)
{
    kfwc_platform p;
    p.ctx = f;
    p.read_ram = fk_read_ram;
    p.ram_contains = fk_ram_contains;
    p.ram_range_count = fk_range_count;
    p.ram_range = fk_range;
    p.read_reg32 = fk_reg;
    p.now = fk_now;
    return p;
}

static void fk_init(fake *f)
{
    memset(f, 0, sizeof(*f));
    f->ram = (kfwc_u8 *)calloc(RAM_BYTES, 1);
    /* Two RAM ranges with a hole between them (like a real map): 6 MiB at 1 MiB, 2 MiB at 256 MiB. */
    f->nranges = 2;
    f->base[0] = 0x100000ull;
    f->len[0] = 6u << 20;
    f->off[0] = 0;
    f->base[1] = 0x10000000ull;
    f->len[1] = 2u << 20;
    f->off[1] = 6u << 20;
    f->boot0 = 0x194000a1u; /* arbitrary non-all-ones liveness word */
    f->regs_ok = 1;
}
static void fk_free(fake *f) { free(f->ram); }
static kfwc_u8 *fk_ptr(fake *f, kfwc_u64 pa)
{
    kfwc_u64 o = 0;
    if (!fk_find(f, pa, 1, &o)) {
        fprintf(stderr, "synth: pa %llx outside RAM\n", (unsigned long long)pa);
        abort();
    }
    return f->ram + o;
}

/* ---- the replica of kayfabe's encoder ------------------------------------------------ */
static void w32(kfwc_u8 *p, kfwc_u32 v) { kfwc_put32(p, v); }

/* Returns run length (elements * 4096). Mirrors kf-gsp encode_message. */
static kfwc_u32 rep_encode(kfwc_u32 layout, kfwc_u32 seq, kfwc_u32 function, kfwc_u32 rpc_seq,
                           kfwc_u32 result, const kfwc_u8 *payload, kfwc_u32 plen, kfwc_u8 *out)
{
    kfwc_u32 hdr = layout == KFWC_LAYOUT_PRE610 ? 48 : 16;
    kfwc_u32 rpc_len = 32 + plen, msg = hdr + rpc_len, elems = (msg + 4095) / 4096;
    kfwc_u32 total = elems * 4096, i;
    unsigned long long acc = 0;
    memset(out, 0, total);
    if (layout == KFWC_LAYOUT_PRE610) {
        w32(out + 36, seq);
        w32(out + 40, elems);
    } else {
        w32(out + 0, 0xC0000001u);
        w32(out + 4, 0x2510DE7Eu);
        w32(out + 12, seq);
    }
    w32(out + hdr + 0, 0x03000000u);
    w32(out + hdr + 4, 0x43505256u);
    w32(out + hdr + 8, rpc_len);
    w32(out + hdr + 12, function);
    w32(out + hdr + 16, result);
    w32(out + hdr + 20, 0);
    w32(out + hdr + 24, rpc_seq);
    w32(out + hdr + 28, 0);
    if (plen)
        memcpy(out + hdr + 32, payload, plen);
    for (i = 0; i < ((msg + 7) & ~7u); i += 8) {
        unsigned long long v = 0;
        int b;
        for (b = 7; b >= 0; b--)
            v = (v << 8) | out[i + (kfwc_u32)b];
        acc ^= v;
    }
    w32(out + (layout == KFWC_LAYOUT_PRE610 ? 32 : 8), (kfwc_u32)(acc >> 32) ^ (kfwc_u32)acc);
    return total;
}

/* ---- a producer pair --------------------------------------------------------------- */
typedef struct synth {
    fake *m;
    kfwc_u32 layout;
    kfwc_u64 table_pa;
    kfwc_u64 page[512];     /* region page list, deliberately scattered */
    kfwc_u32 pages;
    struct { kfwc_u32 off, size, count, write, seq; } q[2];
    kfwc_u64 libos_pa, args_pa;
} synth;

static kfwc_u8 *sy_region(synth *s, kfwc_u32 off) { return fk_ptr(s->m, s->page[off / 4096]) + off % 4096; }

static void sy_hdr(synth *s, kfwc_u32 d, kfwc_u32 write_ptr)
{
    kfwc_u8 *h = sy_region(s, s->q[d].off);
    w32(h + 0, 0);
    w32(h + 4, s->q[d].size);
    w32(h + 8, 4096);
    w32(h + 12, s->q[d].count);
    w32(h + 16, write_ptr);
    w32(h + 20, 1);
    w32(h + 24, 32);
    w32(h + 28, 4096);
}

/* Build a region: page 0 is the table; command queue then status queue. Pages are taken from
 * RAM in a stride pattern so that consecutive region pages are NOT adjacent in memory. */
static void sy_init(synth *s, fake *m, kfwc_u32 layout, kfwc_u64 first_page_pa, kfwc_u32 cmd_bytes,
                    kfwc_u32 stat_bytes, int publish_stat)
{
    kfwc_u32 i, n = 1 + cmd_bytes / 4096 + stat_bytes / 4096;
    memset(s, 0, sizeof(*s));
    s->m = m;
    s->layout = layout;
    s->pages = n;
    s->table_pa = first_page_pa;
    s->page[0] = first_page_pa;
    /* 37 is invertible mod the prime 509, so for 1 <= i < 509 the pages are distinct, nonzero
     * and scattered over a 2 MiB window: consecutive region pages are not adjacent. */
    for (i = 1; i < n; i++)
        s->page[i] = first_page_pa + (kfwc_u64)((i * 37u) % 509u) * 4096u;
    for (i = 0; i < n; i++)
        kfwc_put64(fk_ptr(m, s->page[0]) + 8 * i, s->page[i]);
    s->q[0].off = 4096;
    s->q[0].size = cmd_bytes;
    s->q[0].count = cmd_bytes / 4096 - 1;
    s->q[1].off = 4096 + cmd_bytes;
    s->q[1].size = stat_bytes;
    s->q[1].count = stat_bytes / 4096 - 1;
    sy_hdr(s, 0, 0);
    if (publish_stat)
        sy_hdr(s, 1, 0);
}

static void sy_publish_stat_header(synth *s) { sy_hdr(s, 1, s->q[1].write); }

/* Place a ready-made run into the ring at the producer position. publish=0 leaves writePtr alone. */
static kfwc_u32 sy_put_run(synth *s, kfwc_u32 d, const kfwc_u8 *run, kfwc_u32 run_len, int publish)
{
    kfwc_u32 elems = run_len / 4096, i, slot = s->q[d].write;
    for (i = 0; i < elems; i++) {
        kfwc_u32 sl = (slot + i) % s->q[d].count;
        memcpy(sy_region(s, s->q[d].off + 4096 + sl * 4096), run + i * 4096, 4096);
    }
    if (publish) {
        s->q[d].write = (slot + elems) % s->q[d].count;
        sy_hdr(s, d, s->q[d].write);
    }
    return elems;
}

/* Post one message with the next transport seq. Returns the element count. */
static kfwc_u32 sy_post_ex(synth *s, kfwc_u32 d, kfwc_u32 function, kfwc_u32 rpc_seq, kfwc_u32 result,
                           const kfwc_u8 *payload, kfwc_u32 plen, int publish)
{
    static kfwc_u8 run[65536 + 4096];
    kfwc_u32 n = rep_encode(s->layout, s->q[d].seq, function, rpc_seq, result, payload, plen, run);
    s->q[d].seq++;
    return sy_put_run(s, d, run, n, publish);
}
static kfwc_u32 sy_post(synth *s, kfwc_u32 d, kfwc_u32 function, kfwc_u32 plen)
{
    static kfwc_u8 body[70000];
    kfwc_u32 i;
    for (i = 0; i < plen; i++)
        body[i] = (kfwc_u8)(i * 31u + function);
    return sy_post_ex(s, d, function, s->q[d].seq * 3u, 0, body, plen, 1);
}

/* Publish the boot-args chain: LibOS array page -> RMARGS entry -> init args -> region. */
static void sy_boot_args(synth *s, kfwc_u64 libos_pa, kfwc_u64 args_pa)
{
    fake *m = s->m;
    kfwc_u8 *arr = fk_ptr(m, libos_pa), *a = fk_ptr(m, args_pa);
    s->libos_pa = libos_pa;
    s->args_pa = args_pa;
    memset(arr, 0, 4096);
    /* entry 0: some other region (a zero id would be skipped, not a terminator) */
    kfwc_put64(arr + 0, 0x0000004c4f474b52ull); /* some non-RMARGS id */
    kfwc_put64(arr + 8, 0x1000);
    kfwc_put64(arr + 16, 4096);
    arr[24] = 1;
    arr[25] = 1;
    /* entry 3: RMARGS */
    kfwc_put64(arr + 3 * 32 + 0, KFWC_RMARGS_ID);
    kfwc_put64(arr + 3 * 32 + 8, args_pa);
    kfwc_put64(arr + 3 * 32 + 16, 4096);
    arr[3 * 32 + 24] = 1;
    arr[3 * 32 + 25] = 1;
    memset(a, 0, 64);
    kfwc_put64(a + 0, s->table_pa);
    w32(a + 8, s->pages);
    kfwc_put64(a + 16, s->q[0].off);
    kfwc_put64(a + 24, s->q[1].off);
    m->mailbox0 = (kfwc_u32)libos_pa;
    m->mailbox1 = (kfwc_u32)(libos_pa >> 32);
}

/* ---- sink collecting parsed KFGSPCAP records ---------------------------------------- */
typedef struct rec {
    kfwc_cap_rec h;
    kfwc_u8 *payload;
} rec;

typedef struct sink_buf {
    rec *r;
    kfwc_u32 n, cap;
    kfwc_u32 bytes, limit;   /* limit: refuse when bytes would exceed (0 = unlimited) */
    kfwc_u32 refused;
} sink_buf;

static int sb_put(void *ctx, const kfwc_u8 *b, kfwc_u32 len)
{
    sink_buf *s = (sink_buf *)ctx;
    kfwc_cap_rec h;
    const kfwc_u8 *pl;
    kfwc_u32 used;
    if (s->limit && s->bytes + len > s->limit) {
        s->refused++;
        return 1;
    }
    if (kfwc_cap_rec_parse(b, len, &h, &pl, &used) != KFWC_PARSE_OK || used != len) {
        fprintf(stderr, "sink: library emitted a malformed record\n");
        abort();
    }
    if (s->n == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 256;
        s->r = (rec *)realloc(s->r, s->cap * sizeof(rec));
    }
    s->r[s->n].h = h;
    s->r[s->n].payload = (kfwc_u8 *)malloc(h.payload_bytes ? h.payload_bytes : 1);
    memcpy(s->r[s->n].payload, pl, h.payload_bytes);
    s->n++;
    s->bytes += len;
    return 0;
}
static void sb_free(sink_buf *s)
{
    kfwc_u32 i;
    for (i = 0; i < s->n; i++)
        free(s->r[i].payload);
    free(s->r);
    memset(s, 0, sizeof(*s));
}
/* records of a kind (0 = any) in a direction (99 = any) */
static kfwc_u32 sb_kind(const sink_buf *s, kfwc_u32 kind, kfwc_u32 dir)
{
    kfwc_u32 i, c = 0;
    for (i = 0; i < s->n; i++)
        if ((!kind || s->r[i].h.kind == kind) && (dir == 99 || s->r[i].h.dir == dir))
            c++;
    return c;
}
static kfwc_u32 sb_losses(const sink_buf *s)
{
    return sb_kind(s, KFWC_K_LOSS_OVERFLOW, 99) + sb_kind(s, KFWC_K_LOSS_UNREADABLE, 99) +
           sb_kind(s, KFWC_K_LOSS_UNKNOWN, 99);
}
static kfwc_u32 sb_loss_entries(const sink_buf *s, kfwc_u32 kind)
{
    kfwc_u32 i, c = 0;
    for (i = 0; i < s->n; i++)
        if (s->r[i].h.kind == kind)
            c += s->r[i].h.entry_count;
    return c;
}

/* The messages a decoder would reassemble from the ELEMENTS stream of one direction: entries
 * are joined across records, a loss record discards a partial message, and message
 * boundaries come from the element header (the test-side twin of kf-census's stream.rs). */
typedef struct msg {
    kfwc_u32 seq, function, elems, msg_bytes, layout;
    kfwc_u32 losses_before;   /* loss records seen since the previous message of this direction */
    kfwc_u8 *run;
} msg;
typedef struct msgs {
    msg *m;
    kfwc_u32 n;
} msgs;

static msgs sb_messages(const sink_buf *s, kfwc_u32 dir)
{
    msgs out;
    kfwc_u8 *buf = (kfwc_u8 *)malloc(1 << 20);
    kfwc_u32 have = 0, i, losses = 0;
    out.m = (msg *)calloc(4096, sizeof(msg));
    out.n = 0;
    for (i = 0; i < s->n; i++) {
        const rec *r = &s->r[i];
        if (r->h.dir != dir)
            continue;
        if (r->h.kind >= KFWC_K_LOSS_OVERFLOW && r->h.kind <= KFWC_K_LOSS_UNKNOWN) {
            have = 0;
            losses++;
            continue;
        }
        if (r->h.kind != KFWC_K_ELEMENTS)
            continue;
        memcpy(buf + have, r->payload, r->h.payload_bytes);
        have += r->h.payload_bytes;
        while (have >= 4096) {
            kfwc_elem_info info;
            kfwc_u32 l, ok = 0, need;
            for (l = 1; l <= 2 && !ok; l++)
                ok = kfwc_elem_peek(kfwc_layout_get(l), buf, &info) != KFWC_ELEM_INVALID;
            if (!ok) { /* desync: a decoder drops one entry */
                memmove(buf, buf + 4096, have - 4096);
                have -= 4096;
                continue;
            }
            need = info.elems * 4096;
            if (have < need)
                break;
            out.m[out.n].seq = info.seq;
            out.m[out.n].function = info.rpc_function;
            out.m[out.n].elems = info.elems;
            out.m[out.n].msg_bytes = info.msg_bytes;
            out.m[out.n].layout = info.layout;
            out.m[out.n].losses_before = losses;
            out.m[out.n].run = (kfwc_u8 *)malloc(need);
            memcpy(out.m[out.n].run, buf, need);
            out.n++;
            losses = 0;
            memmove(buf, buf + need, have - need);
            have -= need;
        }
    }
    free(buf);
    return out;
}
static void msgs_free(msgs *m)
{
    kfwc_u32 i;
    for (i = 0; i < m->n; i++)
        free(m->m[i].run);
    free(m->m);
}

/* Format invariants every capture must satisfy (the decoder checks the same things):
 * record counters consecutive from 0 (unless a record was refused), BIND before ELEMENTS for
 * a (queue_id, dir), ELEMENTS contiguous modulo the ring unless a loss record intervenes,
 * payload = whole entries. Returns the number of violations. */
static kfwc_u32 sb_violations(const sink_buf *s, kfwc_u32 allow_counter_gaps)
{
    kfwc_u32 i, bad = 0, expect = 0;
    struct { int bound; kfwc_u32 ring, next; int after_loss; } q[2][8];
    memset(q, 0, sizeof(q));
    for (i = 0; i < s->n; i++) {
        const rec *r = &s->r[i];
        kfwc_u32 qi = r->h.queue_id % 8;
        if (r->h.counter != expect && !allow_counter_gaps)
            bad++;
        expect = r->h.counter + 1;
        if (r->h.kind == KFWC_K_BIND) {
            q[r->h.dir][qi].bound = 1;
            q[r->h.dir][qi].ring = kfwc_get32(r->payload + KFWC_TXH_MSGCOUNT);
            q[r->h.dir][qi].next = 0xFFFFFFFFu;
        } else if (r->h.kind == KFWC_K_ELEMENTS) {
            if (!q[r->h.dir][qi].bound || r->h.payload_bytes != r->h.entry_count * 4096 ||
                r->h.first_entry >= q[r->h.dir][qi].ring ||
                r->h.first_entry + r->h.entry_count > q[r->h.dir][qi].ring /* a record never crosses the ring end */)
                bad++;
            else {
                if (q[r->h.dir][qi].next != 0xFFFFFFFFu && q[r->h.dir][qi].next != r->h.first_entry &&
                    !q[r->h.dir][qi].after_loss)
                    bad++;
                q[r->h.dir][qi].after_loss = 0;
                q[r->h.dir][qi].next = (r->h.first_entry + r->h.entry_count) % q[r->h.dir][qi].ring;
            }
        } else if (r->h.kind >= KFWC_K_LOSS_OVERFLOW && r->h.kind <= KFWC_K_LOSS_UNKNOWN) {
            q[r->h.dir][qi].after_loss = 1;
        }
    }
    return bad;
}

/* ---- engine harness ----------------------------------------------------------------- */
typedef struct harness {
    fake m;
    synth s;
    sink_buf sb;
    kfwc_engine *e;
    kfwc_platform plat;
    kfwc_sink sink;
    kfwc_config cfg;
} harness;

static void hn_init(harness *h, kfwc_u32 layout, int mailbox, int scan)
{
    memset(h, 0, sizeof(*h));
    fk_init(&h->m);
    sy_init(&h->s, &h->m, layout, 0x200000, 0x40000, 0x40000, 1);
    h->e = (kfwc_engine *)calloc(1, sizeof(kfwc_engine));
    h->plat = fk_platform(&h->m);
    h->sink.ctx = &h->sb;
    h->sink.put = sb_put;
    kfwc_config_defaults(&h->cfg, 1000);
    h->cfg.use_mailbox = (kfwc_u32)mailbox;
    h->cfg.use_scan = (kfwc_u32)scan;
    h->cfg.scan_delay_ticks = 0;
    h->cfg.mailbox_period_ticks = 1;
    h->cfg.max_msgs_per_step = 1000;
    h->cfg.mapping_check_steps = 1;
    h->cfg.scan_bytes_per_step = 1u << 20;
    if (mailbox)
        sy_boot_args(&h->s, 0x6F0000, 0x6F1000);
    kfwc_engine_init(h->e, &h->plat, &h->sink, &h->cfg);
}
static void hn_free(harness *h)
{
    sb_free(&h->sb);
    free(h->e);
    fk_free(&h->m);
}
static void hn_steps(harness *h, int n)
{
    int i;
    for (i = 0; i < n; i++)
        kfwc_engine_step(h->e);
}

#endif /* KFWC_SYNTH_H */
