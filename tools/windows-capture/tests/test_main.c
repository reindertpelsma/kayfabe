/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Linux tests of the platform independent library against synthetic queues.
 * Build and run: make -C tools/windows-capture/tests test
 */
#include "synth.h"

static int g_checks, g_fail, g_test_fail;
static const char *g_name;
#define CHECK(cond)                                                                            \
    do {                                                                                       \
        g_checks++;                                                                            \
        if (!(cond)) {                                                                         \
            g_fail++;                                                                          \
            g_test_fail++;                                                                     \
            fprintf(stderr, "  FAIL %s:%d [%s] %s\n", __FILE__, __LINE__, g_name, #cond);       \
        }                                                                                      \
    } while (0)

/* body used by sy_post(function, plen) */
static kfwc_u32 expected_run(kfwc_u32 layout, kfwc_u32 seq, kfwc_u32 function, kfwc_u32 plen, kfwc_u8 *run)
{
    static kfwc_u8 body[70000];
    kfwc_u32 i;
    for (i = 0; i < plen; i++)
        body[i] = (kfwc_u8)(i * 31u + function);
    return rep_encode(layout, seq, function, seq * 3u, 0, body, plen, run);
}

/* does decoded message m equal what sy_post(function, plen) put in the ring with that seq? */
static int msg_is(const msg *m, kfwc_u32 layout, kfwc_u32 seq, kfwc_u32 function, kfwc_u32 plen)
{
    static kfwc_u8 run[70000];
    kfwc_u32 n = expected_run(layout, seq, function, plen, run);
    return m->seq == seq && m->function == function && m->elems * 4096 == n && memcmp(m->run, run, n) == 0;
}

/* ===================================================================================== */
static void t_record_format(void)
{
    kfwc_cap_rec r, p;
    kfwc_u8 buf[256], hdr[32];
    const kfwc_u8 *pl;
    kfwc_u32 n, used, i;
    kfwc_u8 tx[32];

    memset(tx, 0, 32);
    kfwc_put32(tx + 4, 0x40000);
    kfwc_put32(tx + 8, 4096);
    kfwc_put32(tx + 12, 63);
    kfwc_put32(tx + 20, 1);
    kfwc_put32(tx + 24, 32);
    kfwc_put32(tx + 28, 4096);
    r.kind = KFWC_K_BIND;
    r.dir = KFWC_DIR_STAT;
    r.queue_id = 0x0102;
    r.payload_bytes = 32;
    r.counter = 0x11223344;
    r.ts = 0x1122334455667788ull;
    r.first_entry = 5;
    r.entry_count = 9;
    n = kfwc_cap_rec_encode(&r, tx, buf, sizeof(buf));
    CHECK(n == 64);
    /* the byte layout of the reference format (crates/kf-census/src/capture.rs RECORD_FIELDS) */
    memcpy(hdr, buf, 32);
    CHECK(hdr[0] == 'K' && hdr[1] == 'R' && hdr[2] == 'E' && hdr[3] == 'C');   /* 0x4345524b LE */
    CHECK(hdr[4] == 1 && hdr[5] == 1 && hdr[6] == 0x02 && hdr[7] == 0x01);
    CHECK(kfwc_get32(hdr + 8) == 32 && kfwc_get32(hdr + 12) == 0x11223344);
    CHECK(kfwc_get64(hdr + 16) == 0x1122334455667788ull && kfwc_get32(hdr + 24) == 5 && kfwc_get32(hdr + 28) == 9);
    CHECK(kfwc_cap_rec_parse(buf, n, &p, &pl, &used) == KFWC_PARSE_OK && used == n && p.queue_id == 0x0102 &&
          p.kind == KFWC_K_BIND && memcmp(pl, tx, 32) == 0);
    CHECK(kfwc_cap_rec_parse(buf, n - 1, &p, &pl, &used) == KFWC_PARSE_SHORT);
    CHECK(kfwc_cap_rec_parse(buf, 31, &p, &pl, &used) == KFWC_PARSE_SHORT);
    /* the writer refuses what the reader refuses */
    r.payload_bytes = 31;
    CHECK(kfwc_cap_rec_encode(&r, tx, buf, sizeof(buf)) == 0);        /* BIND must be 32 bytes */
    r.kind = KFWC_K_LOSS_UNKNOWN;
    r.payload_bytes = 8;
    CHECK(kfwc_cap_rec_encode(&r, tx, buf, sizeof(buf)) == 0);        /* loss carries no payload */
    r.payload_bytes = 0;
    n = kfwc_cap_rec_encode(&r, 0, buf, sizeof(buf));
    CHECK(n == 32);
    r.kind = 7;
    CHECK(kfwc_cap_rec_encode(&r, 0, buf, sizeof(buf)) == 0);
    r.kind = KFWC_K_END;
    r.dir = 2;
    CHECK(kfwc_cap_rec_encode(&r, 0, buf, sizeof(buf)) == 0);
    /* corruption is BAD, never resynchronised */
    r.dir = 0;
    r.kind = KFWC_K_LOSS_OVERFLOW;
    n = kfwc_cap_rec_encode(&r, 0, buf, sizeof(buf));
    for (i = 0; i < 4; i++) {
        buf[i] ^= 1;
        CHECK(kfwc_cap_rec_parse(buf, n, &p, &pl, &used) == KFWC_PARSE_BAD);
        buf[i] ^= 1;
    }
    buf[4] = 0;
    CHECK(kfwc_cap_rec_parse(buf, n, &p, &pl, &used) == KFWC_PARSE_BAD);
    buf[4] = 3;
    buf[5] = 2;
    CHECK(kfwc_cap_rec_parse(buf, n, &p, &pl, &used) == KFWC_PARSE_BAD);
    buf[5] = 0;
    kfwc_put32(buf + 8, (1u << 20) + 1);
    CHECK(kfwc_cap_rec_parse(buf, n, &p, &pl, &used) == KFWC_PARSE_BAD);
}

static void t_file_header(void)
{
    kfwc_cap_file f, g;
    kfwc_u8 b[128];
    memset(&f, 0, sizeof(f));
    f.os_id = KFWC_CAP_OS_WINDOWS;
    f.module_version = 3;
    f.pci_vendor_id = 0x10de;
    f.pci_device_id = 0x2786;
    f.pci_revision_id = 0xa1;
    f.ts_hz = 10000000ull;
    f.ts_start = 99;
    CHECK(kfwc_cap_set_string(f.driver_version, "32.0.15.8088") == 0);
    CHECK(kfwc_cap_set_string(f.decode_tag, "580.82.07") == 0);
    CHECK(kfwc_cap_file_encode(&f, b) == 128);
    CHECK(memcmp(b, "KFGSPCAP", 8) == 0 && kfwc_get32(b + 8) == 1 && kfwc_get32(b + 12) == 128 && kfwc_get32(b + 16) == 32);
    CHECK(kfwc_get32(b + 20) == 0 && kfwc_get32(b + 24) == 1 && kfwc_get32(b + 28) == 3);
    CHECK(b[32] == 0xde && b[33] == 0x10 && b[34] == 0x86 && b[35] == 0x27 && b[36] == 0xa1);
    CHECK(kfwc_get64(b + 40) == 10000000ull && kfwc_get64(b + 48) == 99);
    CHECK(memcmp(b + 56, "32.0.15.8088", 13) == 0 && memcmp(b + 88, "580.82.07", 10) == 0);
    CHECK(kfwc_cap_file_parse(b, 128, &g) == KFWC_PARSE_OK && g.pci_device_id == 0x2786 && g.os_id == 1);
    CHECK(kfwc_cap_file_parse(b, 127, &g) == KFWC_PARSE_SHORT);
    b[20] = 1; /* flags must be zero */
    CHECK(kfwc_cap_file_parse(b, 128, &g) == KFWC_PARSE_BAD);
    b[20] = 0;
    b[37] = 1; /* reserved */
    CHECK(kfwc_cap_file_parse(b, 128, &g) == KFWC_PARSE_BAD);
    b[37] = 0;
    b[8] = 2; /* version */
    CHECK(kfwc_cap_file_parse(b, 128, &g) == KFWC_PARSE_BAD);
    b[8] = 1;
    b[57] = 0x01; /* control character in a string */
    CHECK(kfwc_cap_file_parse(b, 128, &g) == KFWC_PARSE_BAD);
    /* strings: non-printable bytes are dropped and reported, long ones truncated, never overflow */
    CHECK(kfwc_cap_set_string(f.driver_version, "ab\x01" "cd") == 1 && strcmp(f.driver_version, "abcd") == 0);
    CHECK(kfwc_cap_set_string(f.driver_version, "0123456789012345678901234567890123456789") == 1 &&
          strlen(f.driver_version) == 31);
    memset(f.driver_version, 'x', 32);   /* no NUL terminator */
    CHECK(kfwc_cap_file_encode(&f, b) == 0);
    f.os_id = 3;
    kfwc_cap_set_string(f.driver_version, "1");
    CHECK(kfwc_cap_file_encode(&f, b) == 0);
}

static void t_elements(void)
{
    static kfwc_u8 run[3 * 4096], body[5000];
    kfwc_elem_info info;
    kfwc_u32 layout, i;
    for (i = 0; i < sizeof(body); i++)
        body[i] = (kfwc_u8)i;
    for (layout = 1; layout <= 2; layout++) {
        const kfwc_layout *l = kfwc_layout_get(layout);
        kfwc_u32 n = rep_encode(layout, 41, 0x55, 9, 0, body, 5000, run);
        CHECK(n == 2 * 4096);
        CHECK(kfwc_elem_peek(l, run, &info) == KFWC_ELEM_OK);
        CHECK(info.seq == 41 && info.elems == 2 && info.rpc_function == 0x55 && info.rpc_sequence == 9 &&
              info.msg_bytes == (kfwc_u32)l->hdr_size + 32u + 5000u);
        CHECK(kfwc_elem_checksum_ok(l, run, &info));
        run[l->hdr_size + 100] ^= 1; /* one flipped bit in the body */
        CHECK(!kfwc_elem_checksum_ok(l, run, &info));
        run[l->hdr_size + 100] ^= 1;
        run[l->hdr_size + 4] ^= 1; /* signature */
        CHECK(kfwc_elem_peek(l, run, &info) == KFWC_ELEM_INVALID);
        run[l->hdr_size + 4] ^= 1;
        run[l->hdr_size + 8] = 0xff; /* length way out of range */
        run[l->hdr_size + 9] = 0xff;
        CHECK(kfwc_elem_peek(l, run, &info) == KFWC_ELEM_INVALID);
    }
    /* a pre-610 element viewed as 610 (and the reverse) is rejected, not mis-decoded */
    rep_encode(1, 5, 1, 1, 0, body, 10, run);
    CHECK(kfwc_elem_peek(kfwc_layout_get(2), run, &info) == KFWC_ELEM_INVALID);
    rep_encode(2, 5, 1, 1, 0, body, 10, run);
    CHECK(kfwc_elem_peek(kfwc_layout_get(1), run, &info) == KFWC_ELEM_INVALID);
    /* confidential-compute authentication bytes: well formed but unsupported */
    rep_encode(1, 5, 1, 1, 0, body, 10, run);
    run[3] = 0x99;
    CHECK(kfwc_elem_peek(kfwc_layout_get(1), run, &info) == KFWC_ELEM_UNSUPPORTED);
    /* elemCount that disagrees with the length */
    rep_encode(1, 5, 1, 1, 0, body, 10, run);
    run[40] = 2;
    CHECK(kfwc_elem_peek(kfwc_layout_get(1), run, &info) == KFWC_ELEM_INVALID);
    {
        kfwc_u8 v[8] = {2, 0, 0, 0, 1, 0, 0, 0};   /* 0x0000000100000002 -> hi ^ lo = 3 */
        CHECK(kfwc_fold32(v, 8) == 3);
    }
}

/* ===================================================================================== */
static void t_mailbox_ok(void)
{
    harness h;
    kfwc_binding *b = (kfwc_binding *)calloc(1, sizeof(*b));
    kfwc_mb_diag d;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    CHECK(kfwc_discover_mailbox(&h.plat, b, &d) == 0 && d.status == KFWC_MB_OK);
    CHECK(b->method == KFWC_METHOD_MAILBOX && b->table_pa == h.s.table_pa && b->pages == h.s.pages);
    CHECK(b->libos_args_pa == 0x6F0000 && b->q[0].valid && b->q[1].valid);
    CHECK(b->q[0].off == 4096 && b->q[1].off == 4096 + 0x40000 && b->q[0].count == 63);
    CHECK(b->ptes[5] == h.s.page[5] && b->ptes[0] == h.s.table_pa);
    free(b);
    hn_free(&h);
}

static void expect_mb(harness *h, kfwc_u32 status)
{
    kfwc_binding *b = (kfwc_binding *)calloc(1, sizeof(*b));
    kfwc_mb_diag d;
    int rc = kfwc_discover_mailbox(&h->plat, b, &d);
    CHECK(rc != 0 && d.status == status);
    if (d.status != status)
        fprintf(stderr, "    got status %u want %u\n", d.status, status);
    free(b);
}

static void t_mailbox_failures(void)
{
    harness h;
    kfwc_u8 *arr;
    kfwc_u8 *args;

    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    arr = fk_ptr(&h.m, 0x6F0000);
    args = fk_ptr(&h.m, 0x6F1000);

    h.m.regs_ok = 0;
    expect_mb(&h, KFWC_MB_UNREADABLE);
    h.m.regs_ok = 1;
    h.m.boot0 = 0xFFFFFFFFu; /* bus returns all-ones: device not readable */
    expect_mb(&h, KFWC_MB_UNREADABLE);
    h.m.boot0 = 0x194000a1u;

    h.m.mailbox0 = 0;
    h.m.mailbox1 = 0;
    expect_mb(&h, KFWC_MB_ZERO); /* Hopper-or-later regime, or boot not started */
    h.m.mailbox0 = KFWC_MAILBOX0_SUSPENDED;
    expect_mb(&h, KFWC_MB_SUSPENDED);
    h.m.mailbox0 = 0x6F0000;
    h.m.mailbox1 = 0x1;       /* an address above all RAM */
    expect_mb(&h, KFWC_MB_BAD_ADDR);
    h.m.mailbox1 = 0;
    h.m.mailbox0 = 0x6F0008;  /* unaligned */
    expect_mb(&h, KFWC_MB_BAD_ADDR);
    h.m.mailbox0 = 0x6F0000;

    kfwc_put64(arr + 3 * 32, 0x1111);   /* no RMARGS entry */
    expect_mb(&h, KFWC_MB_NO_RMARGS);
    kfwc_put64(arr + 3 * 32, KFWC_RMARGS_ID);
    arr[3 * 32 + 25] = 2;               /* RMARGS present but not contiguous sysmem */
    expect_mb(&h, KFWC_MB_ARGS_BAD);
    arr[3 * 32 + 25] = 1;
    kfwc_put64(arr + 3 * 32 + 8, 0x9000000000ull);   /* args pointer outside RAM */
    expect_mb(&h, KFWC_MB_ARGS_BAD);
    kfwc_put64(arr + 3 * 32 + 8, 0x6F1000);
    kfwc_put32(args + 8, 100000);       /* absurd PTE count */
    expect_mb(&h, KFWC_MB_ARGS_BAD);
    kfwc_put32(args + 8, 1);
    expect_mb(&h, KFWC_MB_ARGS_BAD);
    kfwc_put32(args + 8, h.s.pages);
    kfwc_put64(fk_ptr(&h.m, h.s.table_pa), 0x123000);   /* page table whose first entry is not itself */
    expect_mb(&h, KFWC_MB_REGION_BAD);
    kfwc_put64(fk_ptr(&h.m, h.s.table_pa), h.s.table_pa);
    kfwc_put64(fk_ptr(&h.m, h.s.table_pa) + 16, h.s.page[1]);   /* duplicated page entries */
    expect_mb(&h, KFWC_MB_REGION_BAD);
    kfwc_put64(fk_ptr(&h.m, h.s.table_pa) + 16, h.s.page[2]);
    kfwc_put32(sy_region(&h.s, 4096) + 8, 2048);   /* msgSize != RM page */
    expect_mb(&h, KFWC_MB_QUEUE_BAD);
    kfwc_put32(sy_region(&h.s, 4096) + 8, 4096);
    kfwc_put32(sy_region(&h.s, 4096) + 16, 63);    /* writePtr == count */
    expect_mb(&h, KFWC_MB_QUEUE_BAD);
    kfwc_put32(sy_region(&h.s, 4096) + 16, 0);
    kfwc_put32(sy_region(&h.s, 4096) + 12, 64);    /* msgCount != (size-4096)/4096 */
    expect_mb(&h, KFWC_MB_QUEUE_BAD);
    kfwc_put32(sy_region(&h.s, 4096) + 12, 63);
    kfwc_put32(sy_region(&h.s, 4096) + 20, 0);     /* SWAP_RX flag missing: a ring that would deadlock */
    expect_mb(&h, KFWC_MB_QUEUE_BAD);
    kfwc_put32(sy_region(&h.s, 4096) + 20, 1);
    {
        kfwc_binding *b = (kfwc_binding *)calloc(1, sizeof(*b));
        kfwc_mb_diag d;
        CHECK(kfwc_discover_mailbox(&h.plat, b, &d) == 0);   /* after all repairs the chain works again */
        free(b);
    }
    hn_free(&h);
}

static void t_scan_signature_false_positives(void)
{
    harness h;
    kfwc_binding *b = (kfwc_binding *)calloc(1, sizeof(*b));
    kfwc_u8 *pg;
    hn_init(&h, KFWC_LAYOUT_PRE610, 0, 1);
    pg = fk_ptr(&h.m, 0x500000);              /* self-referencing first entry, junk elsewhere */
    memset(pg, 0xA5, 4096);
    kfwc_put64(pg, 0x500000);
    CHECK(kfwc_bind_candidate(&h.plat, b, 0x500000) != 0);
    pg = fk_ptr(&h.m, 0x501000);              /* entry 1 inside RAM but not a queue header */
    memset(pg, 0, 4096);
    kfwc_put64(pg, 0x501000);
    kfwc_put64(pg + 8, 0x502000);
    CHECK(kfwc_bind_candidate(&h.plat, b, 0x501000) != 0);
    {
        synth s;                              /* a real-looking table with a bad flags word */
        sy_init(&s, &h.m, KFWC_LAYOUT_PRE610, 0x300000, 0x40000, 0x40000, 1);
        kfwc_put32(sy_region(&s, 4096) + 20, 7);
        CHECK(kfwc_bind_candidate(&h.plat, b, 0x300000) != 0);
        kfwc_put32(sy_region(&s, 4096) + 20, 1);
        CHECK(kfwc_bind_candidate(&h.plat, b, 0x300000) == 0);
        CHECK(b->method == KFWC_METHOD_SCAN && b->q[0].size == 0x40000 && b->q[1].valid);
    }
    pg = fk_ptr(&h.m, 0x502000);              /* self reference and a pointer outside RAM */
    memset(pg, 0, 4096);
    kfwc_put64(pg, 0x502000);
    kfwc_put64(pg + 8, 0xFFFF000000ull);
    CHECK(kfwc_bind_candidate(&h.plat, b, 0x502000) != 0);
    CHECK(kfwc_bind_candidate(&h.plat, b, 0x500008) != 0);     /* unaligned */
    CHECK(kfwc_bind_candidate(&h.plat, b, 0x0) != 0);          /* outside RAM */
    CHECK(kfwc_bind_candidate(&h.plat, b, 0x80000000ull) != 0);
    CHECK(kfwc_scan_signature(0x1000, (const kfwc_u8 *)"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0") == 0);
    free(b);
    hn_free(&h);
}

/* ===================================================================================== */
static void t_walker_basic(kfwc_u32 layout)
{
    harness h;
    kfwc_u32 i;
    msgs c, s;
    hn_init(&h, layout, 1, 0);
    for (i = 0; i < 5; i++) {
        sy_post(&h.s, 0, 0x10 + i, 20 * i);
        sy_post(&h.s, 1, 0x20 + i, 3000 + 700 * i);  /* the last two are two-element messages */
    }
    hn_steps(&h, 3);
    CHECK(sb_kind(&h.sb, KFWC_K_BIND, 0) == 1 && sb_kind(&h.sb, KFWC_K_BIND, 1) == 1);
    c = sb_messages(&h.sb, 0);
    s = sb_messages(&h.sb, 1);
    CHECK(c.n == 5 && s.n == 5);
    CHECK(sb_losses(&h.sb) == 0);          /* sequences start at 0: nothing before the first entry */
    CHECK(sb_violations(&h.sb, 0) == 0);
    for (i = 0; i < 5 && i < c.n && i < s.n; i++) {
        CHECK(msg_is(&c.m[i], layout, i, 0x10 + i, 20 * i));
        CHECK(msg_is(&s.m[i], layout, i, 0x20 + i, 3000 + 700 * i));
    }
    msgs_free(&c);
    msgs_free(&s);
    hn_steps(&h, 5);
    /* incremental delivery, one message at a time */
    for (i = 0; i < 20; i++) {
        msgs t;
        sy_post(&h.s, 1, 0x40 + i, 50);
        hn_steps(&h, 1);
        t = sb_messages(&h.sb, 1);
        CHECK(t.n == 6 + i);
        msgs_free(&t);
    }
    CHECK(sb_losses(&h.sb) == 0 && sb_violations(&h.sb, 0) == 0);
    CHECK(h.e->stats.lost_messages_exact == 0 && h.e->stats.sink_dropped == 0);
    CHECK(h.m.stores == 0);
    hn_free(&h);
}
static void t_basic_pre610(void) { t_walker_basic(KFWC_LAYOUT_PRE610); }
static void t_basic_610(void) { t_walker_basic(KFWC_LAYOUT_610); }

static void t_late_attach_prefix(void)
{
    harness h;
    msgs c;
    kfwc_u32 i;
    /* the transport sequence of a queue instance starts at 0; attaching to a stream whose
     * oldest retained message is not 0 must say that a prefix was not seen */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    h.s.q[0].seq = 7;
    for (i = 0; i < 3; i++)
        sy_post(&h.s, 0, 0x10 + i, 10);
    hn_steps(&h, 3);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 3 && sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 0) == 1);
    CHECK(c.m[0].losses_before == 1 && c.m[1].losses_before == 0);
    CHECK(sb_loss_entries(&h.sb, KFWC_K_LOSS_UNKNOWN) == KFWC_UNKNOWN_COUNT);
    CHECK(sb_violations(&h.sb, 0) == 0);
    msgs_free(&c);
    hn_free(&h);
}

static void t_walker_wrap(void)
{
    harness h;
    kfwc_u32 i;
    msgs c;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    /* 400 messages of mixed size through a 63-entry ring, drained every 5 posts: the ring wraps
     * many times and multi-element runs straddle the end of the ring (split into two records). */
    for (i = 0; i < 400; i++) {
        kfwc_u32 plen = (i % 7 == 3) ? 9000 : (i % 5 == 1 ? 4100 : 64);
        sy_post(&h.s, 0, 0x100 + i, plen);
        if (i % 5 == 4)
            hn_steps(&h, 1);
    }
    hn_steps(&h, 2);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 400);
    CHECK(sb_losses(&h.sb) == 0 && sb_violations(&h.sb, 0) == 0);
    for (i = 0; i < 400 && i < c.n; i++) {
        kfwc_u32 plen = (i % 7 == 3) ? 9000 : (i % 5 == 1 ? 4100 : 64);
        if (!msg_is(&c.m[i], KFWC_LAYOUT_PRE610, i, 0x100 + i, plen)) {
            CHECK(0 && "wrapped message mismatch");
            break;
        }
    }
    {
        /* at least one message was cut at the ring end into two ELEMENTS records */
        kfwc_u32 split = 0, k;
        for (k = 1; k < h.sb.n; k++)
            if (h.sb.r[k].h.kind == KFWC_K_ELEMENTS && h.sb.r[k].h.first_entry == 0 &&
                h.sb.r[k - 1].h.kind == KFWC_K_ELEMENTS && h.sb.r[k - 1].h.dir == h.sb.r[k].h.dir &&
                (h.sb.r[k - 1].h.first_entry + h.sb.r[k - 1].h.entry_count) == 63)
                split++;
        CHECK(split > 0);
    }
    msgs_free(&c);
    hn_free(&h);
}

static void t_walker_lapped(void)
{
    harness h;
    kfwc_u32 i, got;
    msgs s;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    for (i = 0; i < 10; i++)
        sy_post(&h.s, 1, 0x1 + i, 10);
    hn_steps(&h, 2);
    s = sb_messages(&h.sb, 1);
    CHECK(s.n == 10);
    msgs_free(&s);
    /* the producer runs 150 single-entry messages ahead of the observer: only the last 63
     * still exist in the ring */
    for (i = 10; i < 160; i++)
        sy_post(&h.s, 1, 0x1 + i, 10);
    hn_steps(&h, 2);
    s = sb_messages(&h.sb, 1);
    got = s.n;
    CHECK(got == 10 + 63);
    CHECK(h.e->stats.lapped_messages == 150 - 63);
    CHECK(got + h.e->stats.lapped_messages == 160);   /* every message is either copied or counted lost */
    CHECK(sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 99) == 1);
    CHECK(s.m[10].losses_before == 1 && s.m[10].seq == 160 - 63);
    CHECK(sb_violations(&h.sb, 0) == 0);
    msgs_free(&s);
    sy_post(&h.s, 1, 0x999, 10);
    hn_steps(&h, 1);
    s = sb_messages(&h.sb, 1);
    CHECK(s.n == got + 1 && sb_losses(&h.sb) == 1);   /* back in step, no further loss */
    msgs_free(&s);
    hn_free(&h);
}

static void t_walker_seq_gap(void)
{
    harness h;
    msgs c;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    sy_post(&h.s, 0, 1, 10);
    sy_post(&h.s, 0, 2, 10);
    hn_steps(&h, 2);
    h.s.q[0].seq += 5; /* five sequence numbers vanish without a trace in the ring */
    sy_post(&h.s, 0, 3, 10);
    hn_steps(&h, 1);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 3 && h.e->stats.seq_gap_messages == 5);
    CHECK(sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 0) == 1 && c.m[2].losses_before == 1);
    msgs_free(&c);
    hn_free(&h);

    /* the 32-bit sequence wrapping around is not a gap */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    h.s.q[1].seq = 0xFFFFFFFDu;
    sy_post(&h.s, 1, 1, 10);
    sy_post(&h.s, 1, 2, 10);
    hn_steps(&h, 1);
    sy_post(&h.s, 1, 3, 10);
    sy_post(&h.s, 1, 4, 10);
    sy_post(&h.s, 1, 5, 10);   /* seqs ... 0xFFFFFFFF, 0, 1 */
    hn_steps(&h, 1);
    c = sb_messages(&h.sb, 1);
    CHECK(c.n == 5 && h.e->stats.seq_gap_messages == 0 && h.e->stats.lapped_messages == 0);
    CHECK(sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 99) == 1);   /* only the unseen prefix (first seq != 0) */
    msgs_free(&c);
    hn_free(&h);
}

static void t_empty_then_messages_and_history(void)
{
    harness h;
    kfwc_u32 i;
    msgs c;
    /* attach to an empty ring, messages arrive later */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    hn_steps(&h, 3);
    CHECK(h.e->state == KFWC_ST_ATTACHED && sb_kind(&h.sb, KFWC_K_ELEMENTS, 99) == 0);
    sy_post(&h.s, 0, 1, 10);
    sy_post(&h.s, 0, 2, 10);
    hn_steps(&h, 1);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 2 && sb_losses(&h.sb) == 0 && c.m[0].seq == 0);
    msgs_free(&c);
    hn_free(&h);

    /* late attach with retained history of 30 messages (some multi-entry) whose oldest seq is 3000:
     * all recovered in order, one "prefix unseen" marker, nothing else lost */
    hn_init(&h, KFWC_LAYOUT_610, 1, 0);
    h.s.q[1].seq = 3000;
    for (i = 0; i < 30; i++)
        sy_post(&h.s, 1, 0x70 + i, (i % 4 == 0) ? 5000 : 100);
    hn_steps(&h, 2);
    c = sb_messages(&h.sb, 1);
    CHECK(c.n == 30 && sb_losses(&h.sb) == 1 && sb_violations(&h.sb, 0) == 0);
    for (i = 0; i < 30 && i < c.n; i++)
        CHECK(c.m[i].seq == 3000 + i && c.m[i].function == 0x70 + i);
    msgs_free(&c);
    hn_free(&h);
}

static void t_unpublished_and_budget(void)
{
    harness h;
    kfwc_u32 i, step_total = 0, last;
    msgs c;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    hn_steps(&h, 2);
    /* data present in the ring but writePtr not advanced: not published, so not copied */
    sy_post_ex(&h.s, 0, 5, 5, 0, (const kfwc_u8 *)"abcdefgh", 8, 0);
    hn_steps(&h, 3);
    CHECK(sb_kind(&h.sb, KFWC_K_ELEMENTS, 99) == 0);
    h.s.q[0].write = 1;
    sy_hdr(&h.s, 0, 1);
    hn_steps(&h, 1);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 1);
    msgs_free(&c);
    /* per-step message budget bounds one step */
    h.e->cfg.max_msgs_per_step = 4;
    for (i = 0; i < 40; i++)
        sy_post(&h.s, 0, 100 + i, 10);
    last = sb_kind(&h.sb, KFWC_K_ELEMENTS, 99);
    for (i = 0; i < 20 && step_total < 40; i++) {
        kfwc_u32 n;
        kfwc_engine_step(h.e);
        n = sb_kind(&h.sb, KFWC_K_ELEMENTS, 99) - last;
        CHECK(n <= 4);
        step_total += n;
        last += n;
    }
    CHECK(step_total == 40);
    CHECK(sb_losses(&h.sb) == 0);
    hn_free(&h);
}

static void t_stat_header_pending(void)
{
    harness h;
    msgs c, s;
    memset(&h, 0, sizeof(h));
    fk_init(&h.m);
    sy_init(&h.s, &h.m, KFWC_LAYOUT_PRE610, 0x200000, 0x40000, 0x40000, 0 /* status header not published */);
    h.e = (kfwc_engine *)calloc(1, sizeof(kfwc_engine));
    h.plat = fk_platform(&h.m);
    h.sink.ctx = &h.sb;
    h.sink.put = sb_put;
    kfwc_config_defaults(&h.cfg, 1000);
    h.cfg.scan_delay_ticks = 0;
    h.cfg.use_mailbox = 0;
    h.cfg.mapping_check_steps = 1;
    kfwc_engine_init(h.e, &h.plat, &h.sink, &h.cfg);
    sy_post(&h.s, 0, 1, 10);
    hn_steps(&h, 40);   /* one full scan pass is at most 8 MiB / 1 MiB steps */
    CHECK(h.e->state == KFWC_ST_ATTACHED && h.e->bind.method == KFWC_METHOD_SCAN);
    CHECK(h.e->bind.q[0].valid && !h.e->bind.q[1].valid);
    CHECK(sb_kind(&h.sb, KFWC_K_BIND, 0) == 1 && sb_kind(&h.sb, KFWC_K_BIND, 1) == 0);   /* no BIND for a zero header */
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 1);
    msgs_free(&c);
    /* the firmware starts: the status queue header appears, is bound, then followed */
    sy_publish_stat_header(&h.s);
    sy_post(&h.s, 1, 9, 10);
    hn_steps(&h, 2);
    s = sb_messages(&h.sb, 1);
    CHECK(h.e->bind.q[1].valid && s.n == 1 && sb_kind(&h.sb, KFWC_K_BIND, 1) == 1);
    CHECK(sb_losses(&h.sb) == 0 && sb_violations(&h.sb, 0) == 0);
    msgs_free(&s);
    hn_free(&h);
}

/* ===================================================================================== */
static void t_scan_discovery(void)
{
    harness h;
    kfwc_u32 steps = 0;
    memset(&h, 0, sizeof(h));
    fk_init(&h.m);
    /* the region sits in the SECOND RAM range */
    sy_init(&h.s, &h.m, KFWC_LAYOUT_610, 0x10000000ull, 0x40000, 0x40000, 1);
    h.e = (kfwc_engine *)calloc(1, sizeof(kfwc_engine));
    h.plat = fk_platform(&h.m);
    h.sink.ctx = &h.sb;
    h.sink.put = sb_put;
    kfwc_config_defaults(&h.cfg, 1000);
    h.cfg.use_mailbox = 0;
    h.cfg.scan_bytes_per_step = 256u * 1024u;   /* rate limit: 256 KiB per step */
    h.cfg.mapping_check_steps = 1;
    kfwc_engine_init(h.e, &h.plat, &h.sink, &h.cfg);
    sy_post(&h.s, 0, 0x11, 100);
    sy_post(&h.s, 1, 0x12, 100);
    while (h.e->state != KFWC_ST_ATTACHED && steps < 200) {
        kfwc_u64 before = h.m.ram_bytes;
        kfwc_engine_step(h.e);
        steps++;
        if (h.e->state != KFWC_ST_ATTACHED)
            CHECK(h.m.ram_bytes - before <= 256u * 1024u);   /* the rate limit is a hard bound per step */
    }
    CHECK(h.e->state == KFWC_ST_ATTACHED);
    CHECK(steps >= (6u << 20) / (256u * 1024u));   /* all of range 0 had to be read first */
    CHECK(h.e->bind.method == KFWC_METHOD_SCAN && h.e->bind.table_pa == 0x10000000ull);
    hn_steps(&h, 2);
    CHECK(sb_kind(&h.sb, KFWC_K_ELEMENTS, 99) == 2);
    CHECK(h.e->stats.scan_candidates >= 1 && h.e->stats.attaches == 1);
    hn_free(&h);
}

static void t_scan_gated_by_mailbox_zero(void)
{
    harness h;
    int i;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 1);
    h.m.mailbox0 = 0;
    h.m.mailbox1 = 0;                 /* boot args not published (or a later boot regime) */
    h.e->cfg.scan_delay_ticks = 1000000;
    for (i = 0; i < 50; i++)
        kfwc_engine_step(h.e);
    CHECK(h.e->state == KFWC_ST_SEARCHING && h.e->stats.scan_bytes == 0);
    CHECK(h.e->stats.mailbox_reads >= 40 && h.e->stats.mailbox_status == KFWC_MB_ZERO);
    sy_boot_args(&h.s, 0x6F0000, 0x6F1000);   /* the mailbox becomes valid: attach by option (a) */
    hn_steps(&h, 2);
    CHECK(h.e->state == KFWC_ST_ATTACHED && h.e->bind.method == KFWC_METHOD_MAILBOX);
    CHECK(h.e->stats.scan_bytes == 0);
    hn_free(&h);

    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 1);   /* unreadable BAR0 lets the scan start immediately */
    h.m.regs_ok = 0;
    h.e->cfg.scan_delay_ticks = 1000000;
    hn_steps(&h, 40);
    CHECK(h.e->state == KFWC_ST_ATTACHED && h.e->bind.method == KFWC_METHOD_SCAN);
    hn_free(&h);
}

static kfwc_u32 events_of(kfwc_engine *e, kfwc_event *out, kfwc_u32 code, kfwc_u32 *n)
{
    kfwc_event all[KFWC_EVENT_SLOTS];
    kfwc_u32 i, c = kfwc_engine_events(e, all, KFWC_EVENT_SLOTS), k = 0;
    for (i = 0; i < c; i++)
        if (all[i].code == code)
            out[k++] = all[i];
    *n = k;
    return c;
}

static void t_events_and_xcheck(void)
{
    harness h;
    kfwc_event ev[8];
    kfwc_u32 n;
    /* scan wins (mailbox reads zero); the free cross-check of option (a) reports ZERO */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 1);
    h.m.mailbox0 = h.m.mailbox1 = 0;
    h.e->cfg.scan_delay_ticks = 0;
    kfwc_engine_note_start(h.e, 0x2786, 1);
    hn_steps(&h, 40);
    events_of(h.e, ev, KFWC_EV_XCHECK, &n);
    CHECK(n == 1 && kfwc_get32(ev[0].data) == KFWC_MB_ZERO && kfwc_get32(ev[0].data + 4) == 0);
    CHECK(sb_kind(&h.sb, 0, 99) >= 2);   /* the events are NOT in the capture stream */
    hn_free(&h);

    /* scan wins while the mailbox chain is valid: the cross-check says MATCH */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 1);
    h.e->cfg.use_mailbox = 0;            /* only the scan finds it ... */
    h.e->cfg.use_scan = 1;
    hn_steps(&h, 40);
    CHECK(h.e->state == KFWC_ST_ATTACHED);
    h.e->cfg.use_mailbox = 1;            /* ... and the xcheck runs when use_mailbox is set at attach time */
    hn_free(&h);

    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 1);
    kfwc_engine_note_start(h.e, 0x2786, 1);
    h.m.regs_ok = 0;                     /* mailbox unreadable: scan attaches, xcheck says UNREADABLE */
    hn_steps(&h, 40);
    {
        kfwc_event all[KFWC_EVENT_SLOTS];
        kfwc_u32 i, c = kfwc_engine_events(h.e, all, KFWC_EVENT_SLOTS), seen_start = 0, seen_attach = 0, seen_x = 0;
        for (i = 0; i < c; i++) {
            seen_start += all[i].code == KFWC_EV_START && kfwc_get32(all[i].data + 20) == 0x2786;
            seen_attach += all[i].code == KFWC_EV_ATTACH && kfwc_get32(all[i].data) == KFWC_METHOD_SCAN;
            seen_x += all[i].code == KFWC_EV_XCHECK && kfwc_get32(all[i].data) == KFWC_MB_UNREADABLE;
        }
        CHECK(seen_start == 1 && seen_attach == 1 && seen_x == 1);
        CHECK(kfwc_engine_events(h.e, all, KFWC_EVENT_SLOTS) == 0);   /* drained */
    }
    hn_free(&h);
}

/* ===================================================================================== */
static void t_detach_and_reattach(void)
{
    harness h;
    kfwc_u8 *hdr;
    msgs c;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    sy_post(&h.s, 0, 1, 10);
    sy_post(&h.s, 1, 1, 10);
    hn_steps(&h, 2);
    CHECK(h.e->state == KFWC_ST_ATTACHED && h.e->generation == 1);
    hdr = sy_region(&h.s, 4096);
    kfwc_put32(hdr + 4, 0x80000);     /* the driver re-created the queue: size changed */
    hn_steps(&h, 1);
    CHECK(h.e->state == KFWC_ST_SEARCHING);
    /* both bound directions get an explicit "lost track, unknown count" marker */
    CHECK(sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 0) == 1 && sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 1) == 1);
    CHECK(sb_loss_entries(&h.sb, KFWC_K_LOSS_UNKNOWN) == 2u * KFWC_UNKNOWN_COUNT);
    kfwc_put32(hdr + 4, 0x40000);     /* restored (a new life): rediscovered on the next steps */
    hn_steps(&h, 2);
    CHECK(h.e->state == KFWC_ST_ATTACHED && h.e->generation == 2);
    sy_post(&h.s, 0, 2, 10);
    hn_steps(&h, 1);
    CHECK(sb_kind(&h.sb, KFWC_K_BIND, 0) == 2);      /* a second BIND, under a new queue_id */
    {
        kfwc_u32 i, ids = 0;
        for (i = 0; i < h.sb.n; i++)
            if (h.sb.r[i].h.kind == KFWC_K_BIND && h.sb.r[i].h.queue_id == 2)
                ids++;
        CHECK(ids == 2);
    }
    CHECK(sb_violations(&h.sb, 0) == 0);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n >= 2 && c.m[c.n - 1].function == 2);
    msgs_free(&c);
    hn_free(&h);

    /* page table rewritten under us */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    hn_steps(&h, 2);
    kfwc_put64(fk_ptr(&h.m, h.s.table_pa) + 8 * 9, h.s.page[9] + 0x1000000000ull);
    hn_steps(&h, 1);
    CHECK(h.e->state == KFWC_ST_SEARCHING && h.e->stats.detaches == 1);
    hn_free(&h);

    /* RAM reads start failing: detach after the bound, not at the first hiccup */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    h.e->cfg.mapping_check_steps = 1000;
    hn_steps(&h, 2);
    h.m.fail_pa = h.s.page[1];       /* the command queue header page */
    hn_steps(&h, 3);
    CHECK(h.e->state == KFWC_ST_ATTACHED);
    hn_steps(&h, 10);
    CHECK(h.e->state == KFWC_ST_SEARCHING && h.e->stats.read_failures >= 8);
    hn_free(&h);
}

static void t_corrupt_and_torn(void)
{
    harness h;
    kfwc_u32 i;
    msgs s, c;
    /* a corrupt element (bad checksum) in the middle of the unread range */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    sy_post(&h.s, 1, 1, 10);
    hn_steps(&h, 2);
    for (i = 0; i < 5; i++)
        sy_post(&h.s, 1, 2 + i, 10);   /* seqs 1..5 in entries 1..5 */
    sy_region(&h.s, h.s.q[1].off + 4096 + 2 * 4096)[60] ^= 0xff;   /* seq 2, inside the checksummed span */
    hn_steps(&h, 20);
    s = sb_messages(&h.sb, 1);
    /* 0 and 1 delivered; 2 unreadable; 3,4,5 recovered by the re-sync; the loss is announced
     * and counted exactly, and nothing is delivered twice */
    CHECK(s.n == 5);
    CHECK(h.e->stats.lapped_messages == 1 && sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 1) == 1);
    CHECK(s.n >= 5 && s.m[2].seq == 3 && s.m[2].losses_before == 1);
    msgs_free(&s);
    sy_post(&h.s, 1, 77, 10);
    hn_steps(&h, 1);
    s = sb_messages(&h.sb, 1);
    CHECK(s.n == 6);
    msgs_free(&s);
    hn_free(&h);

    /* a torn copy that heals on the immediate retry costs nothing */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    hn_steps(&h, 2);
    sy_post(&h.s, 0, 1, 3000);
    h.m.tear_pa = h.s.page[1 + 1];     /* entry 0 of the command queue */
    h.m.tear_remaining = 2;            /* the header peek consumes one, the first full copy the second */
    hn_steps(&h, 1);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 1 && h.e->stats.torn_reads >= 1 && sb_losses(&h.sb) == 0);
    msgs_free(&c);
    hn_free(&h);

    /* a persistently unstable entry: retried, then re-synchronised, and the message is counted lost */
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    hn_steps(&h, 2);
    sy_post(&h.s, 0, 1, 3000);          /* seq 0 */
    hn_steps(&h, 1);
    sy_post(&h.s, 0, 2, 3000);          /* seq 1, entry 1: torn forever */
    sy_post(&h.s, 0, 3, 3000);          /* seq 2 */
    h.m.tear_pa = h.s.page[1 + 1 + 1];
    h.m.tear_remaining = 100000;
    hn_steps(&h, 30);
    c = sb_messages(&h.sb, 0);
    CHECK(h.e->stats.torn_reads >= 8);
    CHECK(c.n == 2 && c.m[0].seq == 0 && c.m[1].seq == 2);
    CHECK(h.e->stats.lapped_messages == 1 && sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 0) == 1);
    msgs_free(&c);
    hn_free(&h);
}

static void t_unsupported_cc(void)
{
    harness h;
    static kfwc_u8 run[4096];
    kfwc_u32 n;
    msgs c;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    hn_steps(&h, 2);
    sy_post(&h.s, 0, 1, 10);
    n = rep_encode(KFWC_LAYOUT_PRE610, h.s.q[0].seq++, 2, 2, 0, (const kfwc_u8 *)"x", 1, run);
    run[5] = 0x42;                     /* authentication tag present: Confidential Compute data */
    sy_put_run(&h.s, 0, run, n, 1);
    sy_post(&h.s, 0, 3, 10);
    hn_steps(&h, 3);
    /* the module does not judge: the raw entries are copied, the decoder says what they are */
    CHECK(sb_kind(&h.sb, KFWC_K_ELEMENTS, 0) == 3 && h.e->stats.unsupported == 1 && sb_losses(&h.sb) == 0);
    c = sb_messages(&h.sb, 0);
    CHECK(c.n == 3);
    msgs_free(&c);
    hn_free(&h);
}

static void t_sink_full(void)
{
    harness h;
    kfwc_u32 i, kept;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    hn_steps(&h, 2);                         /* attach, BIND stored */
    h.sb.limit = h.sb.bytes + 2 * (32 + 4096) + 10;   /* room for two entries and no more */
    for (i = 0; i < 10; i++)
        sy_post(&h.s, 0, 0x10 + i, 100);
    hn_steps(&h, 2);
    kept = sb_kind(&h.sb, KFWC_K_ELEMENTS, 0);
    CHECK(h.sb.refused > 0 && kept == 2);
    CHECK(h.e->dir[0].sink_lost == 10 - kept);                  /* pending, to be announced */
    CHECK(h.e->stats.records_refused >= 8);
    /* the reader drains: space again. The next step announces the drop BEFORE any new entry. */
    h.sb.limit = 0;
    sy_post(&h.s, 0, 0x99, 100);
    hn_steps(&h, 2);
    CHECK(sb_kind(&h.sb, KFWC_K_LOSS_OVERFLOW, 0) == 1 && sb_loss_entries(&h.sb, KFWC_K_LOSS_OVERFLOW) == 10 - kept);
    {
        kfwc_u32 pos_loss = 0, pos_new = 0;
        for (i = 0; i < h.sb.n; i++) {
            if (h.sb.r[i].h.kind == KFWC_K_LOSS_OVERFLOW) {
                pos_loss = i;
                CHECK(h.sb.r[i].h.first_entry == kept);          /* the first refused entry */
            }
            if (h.sb.r[i].h.kind == KFWC_K_ELEMENTS && h.sb.r[i].h.first_entry == 10)
                pos_new = i;
        }
        CHECK(pos_loss && pos_new > pos_loss);
    }
    CHECK(sb_kind(&h.sb, KFWC_K_ELEMENTS, 0) == kept + 1);
    /* the refused records left holes in the record counter: visible even without the marker */
    CHECK(sb_violations(&h.sb, 0) > 0 && sb_violations(&h.sb, 1) == 0);
    hn_free(&h);
}

/* ===================================================================================== */
static void t_read_only_surface(void)
{
    harness h;
    kfwc_u32 v, i, bad = 0;
    kfwc_platform p;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 1);
    p = h.plat;
    /* the library refuses every register outside the whitelist, without asking the platform */
    h.m.reg_log_n = 0;
    CHECK(kfwc_reg_read(&p, 0x00110c00, &v) == 1);      /* NV_PGSP_QUEUE_HEAD(0): never read */
    CHECK(kfwc_reg_read(&p, 0x00110000, &v) == 1);
    CHECK(kfwc_reg_read(&p, 0x00000004, &v) == 1);
    CHECK(kfwc_reg_read(&p, 0x00100000, &v) == 1);
    CHECK(h.m.reg_log_n == 0);
    CHECK(kfwc_reg_read(&p, KFWC_REG_GSP_MAILBOX0, &v) == 0);
    CHECK(h.m.reg_log_n == 1);
    /* a full run (mailbox, scan fallback, walking, a detach) touches only whitelisted registers */
    h.m.reg_log_n = 0;
    sy_post(&h.s, 0, 1, 10);
    sy_post(&h.s, 1, 1, 10);
    hn_steps(&h, 10);
    kfwc_put32(sy_region(&h.s, 4096) + 4, 0x80000);
    hn_steps(&h, 5);
    CHECK(h.m.reg_log_n > 0);
    for (i = 0; i < h.m.reg_log_n; i++)
        if (!kfwc_reg_allowed(h.m.reg_log[i]))
            bad++;
    CHECK(bad == 0);
    /* the platform struct has one context and exactly six accessors, none of which stores */
    CHECK(sizeof(kfwc_platform) == 7 * sizeof(void *));
    hn_free(&h);
}

static void t_ram_range_discipline(void)
{
    harness h;
    kfwc_binding *b = (kfwc_binding *)calloc(1, sizeof(*b));
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    /* a region page list that points into the hole between RAM ranges is refused at validation */
    kfwc_put64(fk_ptr(&h.m, h.s.table_pa) + 8 * 4, 0x8000000ull);
    CHECK(kfwc_bind_region(&h.plat, b, h.s.table_pa, h.s.pages, 4096, 4096 + 0x40000) != 0);
    free(b);
    hn_free(&h);
}

static void t_layout_autodetect_and_pin(void)
{
    harness h;
    kfwc_u32 layout;
    msgs c;
    for (layout = 1; layout <= 2; layout++) {
        hn_init(&h, layout, 1, 0);
        sy_post(&h.s, 0, 1, 10);
        hn_steps(&h, 2);
        c = sb_messages(&h.sb, 0);
        CHECK(c.n == 1 && h.e->bind.layout == layout);
        msgs_free(&c);
        hn_free(&h);
    }
    /* pinned to the wrong layout: copies nothing it cannot validate, and counts it */
    hn_init(&h, KFWC_LAYOUT_610, 1, 0);
    h.e->cfg.layout = KFWC_LAYOUT_PRE610;
    sy_post(&h.s, 0, 1, 10);
    hn_steps(&h, 3);
    CHECK(sb_kind(&h.sb, KFWC_K_ELEMENTS, 0) == 0);
    CHECK(sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 0) == 1);   /* unparseable history is announced, not skipped */
    sy_post(&h.s, 0, 2, 10);
    hn_steps(&h, 3);
    CHECK(sb_kind(&h.sb, KFWC_K_ELEMENTS, 0) == 0 && h.e->stats.invalid_elements > 0);
    hn_free(&h);
}

static void t_end_record(void)
{
    harness h;
    kfwc_u32 n, last;
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    sy_post(&h.s, 0, 1, 10);
    hn_steps(&h, 2);
    kfwc_engine_stop(h.e);
    n = h.sb.n;
    last = h.sb.r[n - 1].h.kind;
    CHECK(last == KFWC_K_END && h.sb.r[n - 1].h.entry_count == n - 1 && h.sb.r[n - 1].h.counter == n - 1);
    CHECK(sb_losses(&h.sb) == 0);        /* a clean stop is not a loss */
    CHECK(sb_violations(&h.sb, 0) == 0);
    kfwc_engine_stop(h.e);               /* idempotent */
    kfwc_engine_step(h.e);
    CHECK(h.sb.n == n);
    hn_free(&h);
}

/* ===================================================================================== */
typedef void (*test_fn)(void);

int main(int argc, char **argv)
{
    static const struct { const char *name; test_fn fn; } tests[] = {
        {"record_format", t_record_format},
        {"file_header", t_file_header},
        {"elements", t_elements},
        {"mailbox_ok", t_mailbox_ok},
        {"mailbox_failures", t_mailbox_failures},
        {"scan_signature_false_positives", t_scan_signature_false_positives},
        {"walker_basic_pre610", t_basic_pre610},
        {"walker_basic_610", t_basic_610},
        {"walker_late_attach_prefix", t_late_attach_prefix},
        {"walker_wrapped_ring", t_walker_wrap},
        {"walker_lost_messages_lapped", t_walker_lapped},
        {"walker_seq_gap_and_wraparound", t_walker_seq_gap},
        {"walker_empty_then_history", t_empty_then_messages_and_history},
        {"walker_unpublished_and_budget", t_unpublished_and_budget},
        {"walker_status_header_pending", t_stat_header_pending},
        {"scan_discovery_rate_limited", t_scan_discovery},
        {"scan_gated_by_mailbox", t_scan_gated_by_mailbox_zero},
        {"events_and_mailbox_cross_check", t_events_and_xcheck},
        {"detach_and_reattach", t_detach_and_reattach},
        {"corrupt_and_torn", t_corrupt_and_torn},
        {"unsupported_cc_copied_raw", t_unsupported_cc},
        {"sink_full_is_announced", t_sink_full},
        {"read_only_surface", t_read_only_surface},
        {"ram_range_discipline", t_ram_range_discipline},
        {"layout_autodetect_and_pin", t_layout_autodetect_and_pin},
        {"end_record", t_end_record},
    };
    unsigned i, pass = 0, n = (unsigned)(sizeof(tests) / sizeof(tests[0]));
    (void)argc;
    (void)argv;
    for (i = 0; i < n; i++) {
        g_name = tests[i].name;
        g_test_fail = 0;
        tests[i].fn();
        if (!g_test_fail)
            pass++;
        printf("%s %s\n", g_test_fail ? "FAIL" : "ok  ", tests[i].name);
    }
    printf("test_main: %u/%u tests passed, %d checks, %d failed\n", pass, n, g_checks, g_fail);
    return pass == n ? 0 : 1;
}
