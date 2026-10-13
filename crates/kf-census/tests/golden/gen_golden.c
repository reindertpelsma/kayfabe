/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * Writes tests/golden/conversation.kfgspcap with the Windows capture module's OWN engine and
 * record writer (tools/windows-capture/lib), from the message runs in conversation.runs
 * (kayfabe's transport encoder, see tests/golden.rs). Nothing here is kf-census code: it is a
 * driver for the module's library through the module's synthetic machine (tests/synth.h).
 *
 *   gen_golden CONVERSATION.runs OUT.kfgspcap
 *
 * Built and run by regen.sh against a checkout of the module (KFWC=.../tools/windows-capture).
 * The module's CI runs `regen.sh --check`: the bytes it produces must equal the committed file.
 */
#include "synth.h"

static int write_capture(const char *path, harness *h)
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
    kfwc_cap_set_string(fh.decode_tag, "580.82.07");
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
    kfwc_u32 pos = 8, count, i;
    harness h;
    if (argc < 3) {
        fprintf(stderr, "usage: %s CONVERSATION.runs OUT.kfgspcap\n", argv[0]);
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
    if (!buf || fread(buf, 1, (size_t)size, f) != (size_t)size)
        return 2;
    fclose(f);
    if (size < 8 || memcmp(buf, "KFRN", 4))
        return 2;
    count = kfwc_get32(buf + 4);
    hn_init(&h, KFWC_LAYOUT_PRE610, 1, 0);
    for (i = 0; i < count; i++) {
        kfwc_u32 dir = buf[pos], len = kfwc_get32(buf + pos + 4);
        sy_put_run(&h.s, dir, buf + pos + 8, len, 1);
        pos += 8 + len;
        hn_steps(&h, 1);
    }
    hn_steps(&h, 2);
    kfwc_engine_stop(h.e);
    if (write_capture(argv[2], &h))
        return 1;
    printf("wrote %s (%u records)\n", argv[2], h.sb.n);
    hn_free(&h);
    free(buf);
    return 0;
}
