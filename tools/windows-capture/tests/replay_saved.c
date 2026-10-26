/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Private-corpus regression: replay captured ring publications through the real engine.
 * Run on the trusted host only. Output is another private KFGSPCAP stream, never stdout.
 * Compare baseline and candidate outputs byte for byte; this is not hardware validation.
 */
#include "synth.h"

int main(int argc, char **argv)
{
    FILE *in, *out;
    kfwc_u8 fh[KFWC_CAP_FILE_HEADER_BYTES], rh[KFWC_CAP_REC_HEADER_BYTES];
    static kfwc_u8 payload[KFWC_CAP_REC_HEADER_BYTES + 1024u * 1024u];
    harness h;
    kfwc_u32 gen = 0, wrote = 0, i;
    int have = 0, rc = 1;
    if (argc != 3) return 2;
    in = fopen(argv[1], "rb");
    out = fopen(argv[2], "wb");
    if (!in || !out) return 2;
    if (fread(fh, 1, sizeof(fh), in) != sizeof(fh) ||
        fwrite(fh, 1, sizeof(fh), out) != sizeof(fh)) goto done;
    while (fread(rh, 1, sizeof(rh), in) == sizeof(rh)) {
        kfwc_cap_rec r;
        const kfwc_u8 *pl;
        kfwc_u32 len = kfwc_get32(rh + 8), used;
        if (len > sizeof(payload) - sizeof(rh)) goto done;
        memcpy(payload, rh, sizeof(rh));
        if (fread(payload + sizeof(rh), 1, len, in) != len ||
            kfwc_cap_rec_parse(payload, sizeof(rh) + len, &r, &pl, &used)) goto done;
        if (r.kind != KFWC_K_BIND && r.kind != KFWC_K_ELEMENTS) continue;
        if (!have || r.queue_id != gen) {
            if (have) hn_free(&h);
            hn_init(&h, 0, 1, 0);
            gen = r.queue_id;
            wrote = 0;
            have = 1;
        }
        if (r.kind == KFWC_K_BIND) {
            /* Corpus supports the default 256 KiB ring, a named refusal otherwise. */
            if (kfwc_get32(pl + KFWC_TXH_SIZE) != h.s.q[r.dir].size) goto done;
            continue;
        }
        for (i = 0; i < r.entry_count; i++) {
            kfwc_u32 slot = (r.first_entry + i) % h.s.q[r.dir].count;
            memcpy(sy_region(&h.s, h.s.q[r.dir].off + 4096u + slot * 4096u),
                   pl + i * 4096u, 4096u);
        }
        sy_hdr(&h.s, r.dir, (r.first_entry + r.entry_count) % h.s.q[r.dir].count);
        hn_steps(&h, 2);
        for (; wrote < h.sb.n; wrote++) {
            kfwc_u32 bytes = kfwc_cap_rec_encode(&h.sb.r[wrote].h, h.sb.r[wrote].payload,
                                               payload, sizeof(payload));
            if (!bytes || fwrite(payload, 1, bytes, out) != bytes) goto done;
        }
    }
    if (!feof(in)) goto done;
    rc = 0;
done:
    if (have) hn_free(&h);
    fclose(in);
    if (fclose(out)) rc = 1;
    return rc;
}
