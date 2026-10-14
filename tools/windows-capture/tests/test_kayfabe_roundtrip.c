/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * Round trip: kayfabe's OWN transport encoder (kf-gsp encode_message, via tests/kf-fixture) ->
 * synthetic rings -> this library's engine -> KFGSPCAP file.
 *
 * usage: test_kayfabe_roundtrip FIXTURE.bin OUTPREFIX
 * Writes OUTPREFIX.pre610.kfgspcap and OUTPREFIX.610.kfgspcap, which run_tests.sh then feeds to the
 * real decoder (kf-census) as the end-to-end check of the capture format.
 */
#include "synth.h"

static int g_checks, g_fail;
#define CHECK(cond)                                                                            \
    do {                                                                                       \
        g_checks++;                                                                            \
        if (!(cond)) {                                                                         \
            g_fail++;                                                                          \
            fprintf(stderr, "  FAIL %s:%d %s\n", __FILE__, __LINE__, #cond);                    \
        }                                                                                      \
    } while (0)

typedef struct fx_msg {
    kfwc_u32 seq, function, rpc_seq, result, payload_len, run_len;
    const kfwc_u8 *run;
} fx_msg;

/* the generator's payload formula: (k * 131 + i * 17) as u8 */
static void fx_payload(kfwc_u32 i, kfwc_u32 len, kfwc_u8 *out)
{
    kfwc_u32 k;
    for (k = 0; k < len; k++)
        out[k] = (kfwc_u8)(k * 131u + i * 17u);
}

static int write_capture(const char *path, harness *h, kfwc_u32 layout)
{
    FILE *f = fopen(path, "wb");
    kfwc_cap_file fh;
    kfwc_u8 hb[128];
    kfwc_u32 i, n;
    if (!f)
        return 1;
    memset(&fh, 0, sizeof(fh));
    fh.os_id = KFWC_CAP_OS_WINDOWS;
    fh.module_version = 1;
    fh.pci_vendor_id = 0x10de;
    fh.pci_device_id = 0x2786;
    fh.pci_revision_id = 0xa1;
    fh.ts_hz = 1000;
    fh.ts_start = 0;
    kfwc_cap_set_string(fh.driver_version, "32.0.15.8088");
    kfwc_cap_set_string(fh.decode_tag, layout == KFWC_LAYOUT_PRE610 ? "580.82.07" : "610.43.02");
    n = kfwc_cap_file_encode(&fh, hb);
    if (n != 128 || fwrite(hb, 1, n, f) != n)
        return 1;
    for (i = 0; i < h->sb.n; i++) {
        static kfwc_u8 buf[70000];
        kfwc_u32 len = kfwc_cap_rec_encode(&h->sb.r[i].h, h->sb.r[i].payload, buf, sizeof(buf));
        if (!len || fwrite(buf, 1, len, f) != len)
            return 1;
    }
    return fclose(f) != 0;
}

int main(int argc, char **argv)
{
    FILE *f;
    kfwc_u8 *buf;
    long size;
    kfwc_u32 pos = 0, nsets, si;
    if (argc < 3) {
        fprintf(stderr, "usage: %s FIXTURE.bin OUTPREFIX\n", argv[0]);
        return 2;
    }
    f = fopen(argv[1], "rb");
    if (!f) {
        fprintf(stderr, "cannot open %s\n", argv[1]);
        return 2;
    }
    fseek(f, 0, SEEK_END);
    size = ftell(f);
    fseek(f, 0, SEEK_SET);
    buf = (kfwc_u8 *)malloc((size_t)size);
    if (fread(buf, 1, (size_t)size, f) != (size_t)size)
        return 2;
    fclose(f);
    if (memcmp(buf, "KFXT", 4) || kfwc_get32(buf + 4) != 1)
        return 2;
    nsets = kfwc_get32(buf + 8);
    pos = 12;

    for (si = 0; si < nsets; si++) {
        kfwc_u32 layout = kfwc_get32(buf + pos), qbytes = kfwc_get32(buf + pos + 4), nmsg, i;
        const kfwc_u8 *txhdr = buf + pos + 8;
        fx_msg *m;
        harness h;
        static kfwc_u8 mine[70000], body[70000];
        char path[512];
        msgs got;
        int vi;
        pos += 8 + 32;
        nmsg = kfwc_get32(buf + pos);
        pos += 4;
        m = (fx_msg *)calloc(nmsg, sizeof(fx_msg));
        for (i = 0; i < nmsg; i++) {
            m[i].seq = kfwc_get32(buf + pos);
            m[i].function = kfwc_get32(buf + pos + 4);
            m[i].rpc_seq = kfwc_get32(buf + pos + 8);
            m[i].result = kfwc_get32(buf + pos + 12);
            m[i].payload_len = kfwc_get32(buf + pos + 16);
            m[i].run_len = kfwc_get32(buf + pos + 20);
            m[i].run = buf + pos + 24;
            pos += 24 + m[i].run_len;
        }

        /* (1) the replica in synth.h is byte-identical to kayfabe's encoder for every message */
        for (i = 0; i < nmsg; i++) {
            kfwc_u32 n;
            fx_payload(i, m[i].payload_len, body);
            n = rep_encode(layout, m[i].seq, m[i].function, m[i].rpc_seq, m[i].result, body, m[i].payload_len, mine);
            CHECK(n == m[i].run_len && memcmp(mine, m[i].run, n) == 0);
        }

        /* (2) the queue header kayfabe publishes is accepted by discovery, and equals what the
         *     synthetic producer writes */
        hn_init(&h, layout, 1, 0);
        {
            kfwc_qgeom g;
            kfwc_u8 mine_hdr[32];
            CHECK(qbytes == 0x40000);
            CHECK(kfwc_tx_header_check(txhdr, &g, 0) == 1 && g.count == 63 && g.size == qbytes);
            memcpy(mine_hdr, sy_region(&h.s, 4096), 32);
            CHECK(memcmp(mine_hdr, txhdr, 32) == 0);
        }

        /* (3) kayfabe's runs, placed raw in both rings in batches, are copied byte for byte */
        for (i = 0; i < nmsg; i++) {
            sy_put_run(&h.s, 0, m[i].run, m[i].run_len, 1);
            sy_put_run(&h.s, 1, m[i].run, m[i].run_len, 1);
            if (i % 4 == 3)
                hn_steps(&h, 1);
        }
        hn_steps(&h, 2);
        for (vi = 0; vi < 2; vi++) {
            got = sb_messages(&h.sb, (kfwc_u32)vi);
            CHECK(got.n == nmsg);
            for (i = 0; i < nmsg && i < got.n; i++) {
                CHECK(got.m[i].seq == m[i].seq && got.m[i].function == m[i].function);
                CHECK(got.m[i].elems * 4096 == m[i].run_len && memcmp(got.m[i].run, m[i].run, m[i].run_len) == 0);
            }
            msgs_free(&got);
        }
        CHECK(sb_losses(&h.sb) == 0 && sb_violations(&h.sb, 0) == 0);
        CHECK(h.e->stats.lost_messages_exact == 0);
        kfwc_engine_stop(h.e);
        CHECK(h.sb.r[h.sb.n - 1].h.kind == KFWC_K_END);

        snprintf(path, sizeof(path), "%s.%s.kfgspcap", argv[2], layout == KFWC_LAYOUT_PRE610 ? "pre610" : "610");
        CHECK(write_capture(path, &h, layout) == 0);
        printf("wrote %s (%u records)\n", path, h.sb.n);
        hn_free(&h);
        free(m);
    }
    /* A deliberately lossy run: the observer is overtaken by 150 messages. The decoder must see an
     * announced loss (capture_loss), and the sequence numbers around it must agree with the
     * count the engine kept. */
    {
        harness h;
        kfwc_u32 i;
        char path[512];
        hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
        for (i = 0; i < 10; i++)
            sy_post(&h.s, 1, (i & 1) ? 0x41 : 0, 100);
        hn_steps(&h, 2);
        for (i = 10; i < 160; i++)
            sy_post(&h.s, 1, (i & 1) ? 0x41 : 0, 100);
        hn_steps(&h, 2);
        kfwc_engine_stop(h.e);
        CHECK(h.e->stats.lapped_messages == 87);
        CHECK(sb_kind(&h.sb, KFWC_K_LOSS_UNKNOWN, 1) == 1);
        snprintf(path, sizeof(path), "%s.lapped.kfgspcap", argv[2]);
        CHECK(write_capture(path, &h, KFWC_LAYOUT_PRE610) == 0);
        printf("wrote %s (%u records)\n", path, h.sb.n);
        hn_free(&h);
    }
    printf("test_kayfabe_roundtrip: %d checks, %d failed\n", g_checks, g_fail);
    free(buf);
    return g_fail ? 1 : 0;
}
