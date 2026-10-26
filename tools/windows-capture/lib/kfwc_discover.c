/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include "kfwc_discover.h"
#include <string.h>

/* Public-source family capability selection, independent of die and PCI product id.
 * FSP path: kernel_gsp_gh100.c:418-468, gspRmParams.bootArgsOffset.
 * Legacy path: kernel_gsp_tu102.c:347-359, direct LibOS mailbox address. */
void kfwc_family_decode(kfwc_u32 boot0, kfwc_capabilities *c)
{
    memset(c, 0, sizeof(*c));
    if (boot0 == 0xffffffffu)
        return;
    c->architecture = ((boot0 >> 24) & 0x1fu) | (((boot0 >> 8) & 1u) << 5);
    c->implementation = (boot0 >> 20) & 0xfu;
    switch (c->architecture) {
    case 0x16: c->family = KFWC_FAMILY_TURING; break;
    case 0x17: c->family = KFWC_FAMILY_AMPERE; break;
    case 0x19: c->family = KFWC_FAMILY_ADA; break;
    case 0x18: c->family = KFWC_FAMILY_HOPPER; break;
    case 0x1a: c->family = KFWC_FAMILY_BLACKWELL_DATA; break;
    case 0x1b: c->family = KFWC_FAMILY_BLACKWELL_CONSUMER; break;
    default: return;
    }
    c->boot_path = c->family <= KFWC_FAMILY_ADA ? KFWC_BOOT_LIBOS : KFWC_BOOT_FSP;
}

int kfwc_read_capabilities(const kfwc_platform *p, kfwc_capabilities *c)
{
    kfwc_u32 boot0;
    memset(c, 0, sizeof(*c));
    if (kfwc_reg_read(p, KFWC_REG_PMC_BOOT_0, &boot0))
        return 1;
    kfwc_family_decode(boot0, c);
    return 0;
}

#define BIND_REGION_BAD 1
#define BIND_QUEUE_BAD 2

int kfwc_region_read(const kfwc_platform *p, const kfwc_binding *b, kfwc_u32 off, void *dst,
                     kfwc_u32 len)
{
    kfwc_u8 *out = (kfwc_u8 *)dst;
    kfwc_u64 end = (kfwc_u64)off + len;
    if (end > (kfwc_u64)b->pages * KFWC_PAGE)
        return 1;
    while (len) {
        kfwc_u32 page = off / KFWC_PAGE, within = off % KFWC_PAGE;
        kfwc_u32 take = KFWC_PAGE - within;
        if (take > len)
            take = len;
        if (p->read_ram(p->ctx, b->ptes[page] + within, out, take))
            return 1;
        out += take;
        off += take;
        len -= take;
    }
    return 0;
}

static int all_zero32(const kfwc_u8 *h)
{
    kfwc_u32 i;
    for (i = 0; i < KFWC_TXH_BYTES; i++)
        if (h[i])
            return 0;
    return 1;
}

int kfwc_tx_header_check(const kfwc_u8 *h, kfwc_qgeom *g, int allow_zero)
{
    kfwc_u32 size, msg_size, count, write_ptr, rx_off, entry_off;
    if (all_zero32(h))
        return allow_zero ? 2 : 0;
    size = kfwc_get32(h + KFWC_TXH_SIZE);
    msg_size = kfwc_get32(h + KFWC_TXH_MSGSIZE);
    count = kfwc_get32(h + KFWC_TXH_MSGCOUNT);
    write_ptr = kfwc_get32(h + KFWC_TXH_WRITEPTR);
    rx_off = kfwc_get32(h + KFWC_TXH_RXHDROFF);
    entry_off = kfwc_get32(h + KFWC_TXH_ENTRYOFF);
    /* msgqRxLink predicate (kf-gsp rx_link_check), specialised to the one geometry the
     * driver creates: element = one RM page, elements start on the second page. */
    if (kfwc_get32(h + KFWC_TXH_VERSION) != KFWC_MSGQ_VERSION ||
        kfwc_get32(h + KFWC_TXH_FLAGS) != KFWC_MSGQ_FLAG_SWAP_RX)
        return 0;
    if (msg_size != KFWC_PAGE || entry_off != KFWC_PAGE || size % KFWC_PAGE ||
        size < KFWC_QUEUE_MIN_BYTES || size > KFWC_QUEUE_MAX_BYTES)
        return 0;
    if (rx_off < KFWC_TXH_BYTES || rx_off + KFWC_RXH_BYTES > entry_off)
        return 0;
    if (count == 0 || count != (size - entry_off) / msg_size || write_ptr >= count)
        return 0;
    g->valid = 1;
    g->size = size;
    g->msg_size = msg_size;
    g->count = count;
    g->rx_hdr_off = rx_off;
    g->entry_off = entry_off;
    g->write_ptr = write_ptr;
    return 1;
}

static int ptes_load(const kfwc_platform *p, kfwc_binding *b, kfwc_u64 table_pa, kfwc_u32 n)
{
    kfwc_u32 i, j;
    if (n < 3 || n > KFWC_MAX_REGION_PAGES || (table_pa & (KFWC_PAGE - 1)) ||
        !p->ram_contains(p->ctx, table_pa, KFWC_PAGE))
        return 1;
    if (p->read_ram(p->ctx, table_pa, b->scratch, KFWC_PAGE))
        return 1;
    for (i = 0; i < n; i++) {
        kfwc_u64 v = kfwc_get64(b->scratch + 8u * i);
        if (!v || (v & (KFWC_PAGE - 1)) || !p->ram_contains(p->ctx, v, KFWC_PAGE))
            return 1;
        for (j = 0; j < i; j++)
            if (b->ptes[j] == v)
                return 1;
        b->ptes[i] = v;
    }
    /* The table page names itself first (GspMsgQueuesInit includes it as page 0). */
    return b->ptes[0] != table_pa;
}

static int queue_header_at(const kfwc_platform *p, kfwc_binding *b, kfwc_u32 qi, kfwc_u32 off,
                           kfwc_u32 pages, int allow_zero)
{
    kfwc_u8 h[KFWC_TXH_BYTES];
    kfwc_qgeom g;
    int r;
    memset(&g, 0, sizeof(g));
    if (off < KFWC_PAGE || (off % KFWC_PAGE) || (kfwc_u64)off + KFWC_PAGE > (kfwc_u64)pages * KFWC_PAGE)
        return 1;
    if (kfwc_region_read(p, b, off, h, sizeof(h)))
        return 1;
    r = kfwc_tx_header_check(h, &g, allow_zero);
    if (r == 0)
        return 1;
    memset(&b->q[qi], 0, sizeof(b->q[qi]));
    if (r == 1) {
        if ((kfwc_u64)off + g.size > (kfwc_u64)pages * KFWC_PAGE)
            return 1;
        b->q[qi] = g;
    }
    b->q[qi].off = off;
    return 0;
}

int kfwc_bind_region(const kfwc_platform *p, kfwc_binding *b, kfwc_u64 table_pa, kfwc_u32 pte_count,
                     kfwc_u32 cmd_off, kfwc_u32 stat_off)
{
    memset(b->ptes, 0, sizeof(b->ptes));
    b->pages = 0;
    if (ptes_load(p, b, table_pa, pte_count))
        return BIND_REGION_BAD;
    b->pages = pte_count;
    b->table_pa = table_pa;
    if (queue_header_at(p, b, 0, cmd_off, pte_count, 0) ||
        queue_header_at(p, b, 1, stat_off, pte_count, 1))
        return BIND_QUEUE_BAD;
    {
        kfwc_u32 c0 = b->q[0].off, c1 = b->q[0].off + b->q[0].size;
        kfwc_u32 s0 = b->q[1].off;
        kfwc_u32 s1 = b->q[1].valid ? s0 + b->q[1].size : pte_count * KFWC_PAGE;
        /* An unpublished queue has no measured size yet. The validated region bounds
         * its possible extent; never copy the sibling's geometry. Recheck on publication. */
        if (!b->q[1].valid && s1 - s0 < KFWC_QUEUE_MIN_BYTES)
            return BIND_QUEUE_BAD;
        if (c0 < s1 && s0 < c1)
            return BIND_QUEUE_BAD;
    }
    return 0;
}

int kfwc_scan_signature(kfwc_u64 pa, const kfwc_u8 *page)
{
    kfwc_u64 second = kfwc_get64(page + 8);
    return kfwc_get64(page) == pa && second && !(second & (KFWC_PAGE - 1));
}

int kfwc_bind_candidate(const kfwc_platform *p, kfwc_binding *b, kfwc_u64 pa)
{
    kfwc_u8 h[KFWC_TXH_BYTES];
    kfwc_qgeom cmd, stat;
    kfwc_u32 cmd_pages, stat_pages, i;
    int r;
    if ((pa & (KFWC_PAGE - 1)) || !p->ram_contains(p->ctx, pa, KFWC_PAGE))
        return 1;
    if (p->read_ram(p->ctx, pa, b->scratch, KFWC_PAGE) || !kfwc_scan_signature(pa, b->scratch))
        return 1;
    /* Entry 1 is the command queue's header page: validate it to learn the size. */
    {
        kfwc_u64 q0 = kfwc_get64(b->scratch + 8);
        memset(&cmd, 0, sizeof(cmd));
        if (!p->ram_contains(p->ctx, q0, KFWC_PAGE) || p->read_ram(p->ctx, q0, h, sizeof(h)) ||
            kfwc_tx_header_check(h, &cmd, 0) != 1)
            return 1;
    }
    cmd_pages = cmd.size / KFWC_PAGE;
    if (1u + cmd_pages >= KFWC_MAX_REGION_PAGES)
        return 1;
    /* Entry 1 + cmd_pages is the status queue's header page, if the producer laid them out
     * back to back (the observed layout; the mailbox route reads real offsets instead). */
    {
        kfwc_u64 q1 = kfwc_get64(b->scratch + 8u * (1u + cmd_pages));
        memset(&stat, 0, sizeof(stat));
        if (!q1 || (q1 & (KFWC_PAGE - 1)) || !p->ram_contains(p->ctx, q1, KFWC_PAGE) ||
            p->read_ram(p->ctx, q1, h, sizeof(h)))
            return 1;
        r = kfwc_tx_header_check(h, &stat, 1);
        if (r == 0)
            return 1;
        if (r == 1) {
            stat_pages = stat.size / KFWC_PAGE;
        } else {
            /* GspMsgQueuesInit zeroes the region, fills the entire self-table, then
             * creates the command header (ogkm 595.91.07 message_queue_cpu.c:277-309).
             * Its nonzero PTE prefix therefore bounds the still-unpublished status
             * queue. Keep the full validator; do not infer its size from the command. */
            for (i = 1u + cmd_pages; i < KFWC_MAX_REGION_PAGES; i++)
                if (kfwc_get64(b->scratch + 8u * i) == 0)
                    break;
            stat_pages = i - 1u - cmd_pages;
            if (stat_pages < KFWC_QUEUE_MIN_BYTES / KFWC_PAGE ||
                stat_pages > KFWC_QUEUE_MAX_BYTES / KFWC_PAGE)
                return 1;
        }
    }
    i = 1u + cmd_pages + stat_pages;
    if (i > KFWC_MAX_REGION_PAGES)
        return 1;
    if (kfwc_bind_region(p, b, pa, i, KFWC_PAGE, KFWC_PAGE + cmd.size))
        return 1;
    b->method = KFWC_METHOD_SCAN;
    b->libos_args_pa = 0;
    return 0;
}

int kfwc_discover_mailbox(const kfwc_platform *p, kfwc_binding *b, kfwc_mb_diag *d)
{
    kfwc_u32 id, lo, hi, i;
    kfwc_u64 array_pa, args_pa = 0, shared, cmd_off, stat_off;
    kfwc_u32 pte_count;
    kfwc_u8 args[KFWC_MQA_MIN_610];
    int r;

    memset(d, 0, sizeof(*d));
    d->status = KFWC_MB_UNREADABLE;
    if (kfwc_reg_read(p, KFWC_REG_PMC_BOOT_0, &id))
        return 1;
    kfwc_family_decode(id, &d->caps);
    if (kfwc_reg_read(p, KFWC_REG_GSP_MAILBOX0, &lo) || kfwc_reg_read(p, KFWC_REG_GSP_MAILBOX1, &hi))
        return 1;
    /* Still sample the same mailbox registers for restart/back-off signatures.
     * FSP error/status words must never be dereferenced as a LibOS array. */
    if (d->caps.boot_path != KFWC_BOOT_LIBOS) {
        d->mailbox_pa = ((kfwc_u64)hi << 32) | lo;
        d->status = d->caps.boot_path == KFWC_BOOT_FSP ? KFWC_MB_FSP : KFWC_MB_UNKNOWN;
        return 1;
    }
    if (lo == 0 && hi == 0) {
        d->status = KFWC_MB_ZERO;
        return 1;
    }
    if (lo == KFWC_MAILBOX0_SUSPENDED) {
        d->status = KFWC_MB_SUSPENDED;
        return 1;
    }
    array_pa = ((kfwc_u64)hi << 32) | lo;
    d->mailbox_pa = array_pa;
    if ((array_pa & (KFWC_PAGE - 1)) || !p->ram_contains(p->ctx, array_pa, KFWC_LIBOS_ARRAY_BYTES)) {
        d->status = KFWC_MB_BAD_ADDR;
        return 1;
    }
    if (p->read_ram(p->ctx, array_pa, b->scratch, KFWC_LIBOS_ARRAY_BYTES)) {
        d->status = KFWC_MB_BAD_ADDR;
        return 1;
    }
    d->status = KFWC_MB_NO_RMARGS;
    for (i = 0; i < KFWC_LIBOS_MAX_ENTRIES; i++) {
        const kfwc_u8 *e = b->scratch + i * KFWC_LIBOS_STRIDE;
        if (kfwc_get64(e + KFWC_LIBOS_ID) != KFWC_RMARGS_ID)
            continue;
        /* Zero entries are skipped, not terminators (kf-gsp boot.rs). */
        args_pa = kfwc_get64(e + KFWC_LIBOS_PA);
        if (e[KFWC_LIBOS_KIND] != KFWC_LIBOS_KIND_CONTIGUOUS ||
            e[KFWC_LIBOS_LOC] != KFWC_LIBOS_LOC_SYSMEM ||
            kfwc_get64(e + KFWC_LIBOS_SIZE) < KFWC_MQA_MIN_PRE610) {
            d->status = KFWC_MB_ARGS_BAD;
            return 1;
        }
        break;
    }
    if (i == KFWC_LIBOS_MAX_ENTRIES)
        return 1;
    /* The args struct must sit inside one page so one bounded read covers it. */
    if ((args_pa & 7u) || (args_pa & (KFWC_PAGE - 1)) > KFWC_PAGE - sizeof(args) ||
        !p->ram_contains(p->ctx, args_pa, sizeof(args)) ||
        p->read_ram(p->ctx, args_pa, args, sizeof(args))) {
        d->status = KFWC_MB_ARGS_BAD;
        return 1;
    }
    shared = kfwc_get64(args + KFWC_MQA_SHARED_PA);
    pte_count = kfwc_get32(args + KFWC_MQA_PTE_COUNT);
    cmd_off = kfwc_get64(args + KFWC_MQA_CMD_OFF);
    stat_off = kfwc_get64(args + KFWC_MQA_STAT_OFF);
    d->shared_pa = shared;
    if (pte_count < 3 || pte_count > KFWC_MAX_REGION_PAGES || cmd_off >= 0x100000000ull ||
        stat_off >= 0x100000000ull) {
        d->status = KFWC_MB_ARGS_BAD;
        return 1;
    }
    r = kfwc_bind_region(p, b, shared, pte_count, (kfwc_u32)cmd_off, (kfwc_u32)stat_off);
    if (r) {
        d->status = (r == BIND_QUEUE_BAD) ? KFWC_MB_QUEUE_BAD : KFWC_MB_REGION_BAD;
        return 1;
    }
    b->method = KFWC_METHOD_MAILBOX;
    b->libos_args_pa = array_pa;
    d->status = KFWC_MB_OK;
    return 0;
}
