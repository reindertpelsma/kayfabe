/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * KFGSPCAP v1 writer and strict parser (see kfwc_record.h). Field-wise little endian.
 */
#include "kfwc_record.h"
#include <string.h>

static void put16(kfwc_u8 *p, kfwc_u32 v)
{
    p[0] = (kfwc_u8)v;
    p[1] = (kfwc_u8)(v >> 8);
}
static kfwc_u32 get16(const kfwc_u8 *p) { return (kfwc_u32)p[0] | ((kfwc_u32)p[1] << 8); }

static const kfwc_u8 k_magic[8] = {'K', 'F', 'G', 'S', 'P', 'C', 'A', 'P'};

static int kind_ok(kfwc_u32 k) { return k >= KFWC_K_BIND && k <= KFWC_K_END; }

kfwc_u32 kfwc_cap_rec_encode(const kfwc_cap_rec *r, const void *payload, kfwc_u8 *out, kfwc_u32 cap)
{
    if (!kind_ok(r->kind) || r->dir > KFWC_DIR_STAT || r->queue_id > 0xFFFFu ||
        r->payload_bytes > KFWC_CAP_MAX_PAYLOAD)
        return 0;
    if (r->kind == KFWC_K_BIND && r->payload_bytes != KFWC_TXH_BYTES)
        return 0;
    if (r->kind >= KFWC_K_LOSS_OVERFLOW && r->payload_bytes != 0)
        return 0;
    if (cap < KFWC_CAP_REC_HEADER_BYTES + r->payload_bytes)
        return 0;
    kfwc_put32(out + 0, KFWC_CAP_REC_MAGIC);
    out[4] = (kfwc_u8)r->kind;
    out[5] = (kfwc_u8)r->dir;
    put16(out + 6, r->queue_id);
    kfwc_put32(out + 8, r->payload_bytes);
    kfwc_put32(out + 12, r->counter);
    kfwc_put64(out + 16, r->ts);
    kfwc_put32(out + 24, r->first_entry);
    kfwc_put32(out + 28, r->entry_count);
    if (r->payload_bytes)
        memcpy(out + KFWC_CAP_REC_HEADER_BYTES, payload, r->payload_bytes);
    return KFWC_CAP_REC_HEADER_BYTES + r->payload_bytes;
}

int kfwc_cap_rec_parse(const kfwc_u8 *b, kfwc_u32 len, kfwc_cap_rec *r, const kfwc_u8 **payload,
                       kfwc_u32 *consumed)
{
    if (len < KFWC_CAP_REC_HEADER_BYTES)
        return KFWC_PARSE_SHORT;
    if (kfwc_get32(b) != KFWC_CAP_REC_MAGIC)
        return KFWC_PARSE_BAD;
    r->kind = b[4];
    r->dir = b[5];
    r->queue_id = get16(b + 6);
    r->payload_bytes = kfwc_get32(b + 8);
    r->counter = kfwc_get32(b + 12);
    r->ts = kfwc_get64(b + 16);
    r->first_entry = kfwc_get32(b + 24);
    r->entry_count = kfwc_get32(b + 28);
    if (!kind_ok(r->kind) || r->dir > KFWC_DIR_STAT || r->payload_bytes > KFWC_CAP_MAX_PAYLOAD)
        return KFWC_PARSE_BAD;
    if (len - KFWC_CAP_REC_HEADER_BYTES < r->payload_bytes)
        return KFWC_PARSE_SHORT;
    *payload = b + KFWC_CAP_REC_HEADER_BYTES;
    *consumed = KFWC_CAP_REC_HEADER_BYTES + r->payload_bytes;
    return KFWC_PARSE_OK;
}

static int string_ok(const char *s)
{
    kfwc_u32 i, n = 0;
    while (n < KFWC_CAP_STRING_BYTES && s[n])
        n++;
    if (n == KFWC_CAP_STRING_BYTES)
        return 0; /* no terminating NUL */
    for (i = 0; i < n; i++)
        if ((unsigned char)s[i] < 0x20 || (unsigned char)s[i] >= 0x7f)
            return 0;
    for (i = n; i < KFWC_CAP_STRING_BYTES; i++)
        if (s[i])
            return 0;
    return 1;
}

int kfwc_cap_set_string(char dst[KFWC_CAP_STRING_BYTES], const char *src)
{
    kfwc_u32 n = 0;
    int dirty = 0;
    memset(dst, 0, KFWC_CAP_STRING_BYTES);
    for (; *src; src++) {
        unsigned char c = (unsigned char)*src;
        if (c < 0x20 || c >= 0x7f) {
            dirty = 1;
            continue;
        }
        if (n == KFWC_CAP_STRING_BYTES - 1) {
            dirty = 1;
            break;
        }
        dst[n++] = (char)c;
    }
    return dirty;
}

kfwc_u32 kfwc_cap_file_encode(const kfwc_cap_file *f, kfwc_u8 out[KFWC_CAP_FILE_HEADER_BYTES])
{
    if (!string_ok(f->driver_version) || !string_ok(f->decode_tag) ||
        (f->os_id != KFWC_CAP_OS_WINDOWS && f->os_id != KFWC_CAP_OS_LINUX))
        return 0;
    memset(out, 0, KFWC_CAP_FILE_HEADER_BYTES);
    memcpy(out, k_magic, 8);
    kfwc_put32(out + 8, KFWC_CAP_VERSION);
    kfwc_put32(out + 12, KFWC_CAP_FILE_HEADER_BYTES);
    kfwc_put32(out + 16, KFWC_CAP_REC_HEADER_BYTES);
    kfwc_put32(out + 24, f->os_id);
    kfwc_put32(out + 28, f->module_version);
    put16(out + 32, f->pci_vendor_id);
    put16(out + 34, f->pci_device_id);
    out[36] = (kfwc_u8)f->pci_revision_id;
    kfwc_put64(out + 40, f->ts_hz);
    kfwc_put64(out + 48, f->ts_start);
    memcpy(out + 56, f->driver_version, KFWC_CAP_STRING_BYTES);
    memcpy(out + 88, f->decode_tag, KFWC_CAP_STRING_BYTES);
    return KFWC_CAP_FILE_HEADER_BYTES;
}

int kfwc_cap_file_parse(const kfwc_u8 *b, kfwc_u32 len, kfwc_cap_file *f)
{
    kfwc_u32 i;
    if (len < KFWC_CAP_FILE_HEADER_BYTES)
        return KFWC_PARSE_SHORT;
    if (memcmp(b, k_magic, 8) || kfwc_get32(b + 8) != KFWC_CAP_VERSION ||
        kfwc_get32(b + 12) != KFWC_CAP_FILE_HEADER_BYTES || kfwc_get32(b + 16) != KFWC_CAP_REC_HEADER_BYTES)
        return KFWC_PARSE_BAD;
    if (kfwc_get32(b + 20)) /* flags */
        return KFWC_PARSE_BAD;
    for (i = 37; i < 40; i++)
        if (b[i])
            return KFWC_PARSE_BAD;
    for (i = 120; i < 128; i++)
        if (b[i])
            return KFWC_PARSE_BAD;
    f->os_id = kfwc_get32(b + 24);
    f->module_version = kfwc_get32(b + 28);
    f->pci_vendor_id = get16(b + 32);
    f->pci_device_id = get16(b + 34);
    f->pci_revision_id = b[36];
    f->ts_hz = kfwc_get64(b + 40);
    f->ts_start = kfwc_get64(b + 48);
    memcpy(f->driver_version, b + 56, KFWC_CAP_STRING_BYTES);
    memcpy(f->decode_tag, b + 88, KFWC_CAP_STRING_BYTES);
    if ((f->os_id != KFWC_CAP_OS_WINDOWS && f->os_id != KFWC_CAP_OS_LINUX) || !string_ok(f->driver_version) ||
        !string_ok(f->decode_tag))
        return KFWC_PARSE_BAD;
    return KFWC_PARSE_OK;
}
