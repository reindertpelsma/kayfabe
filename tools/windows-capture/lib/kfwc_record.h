/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * The KFGSPCAP v1 capture format, as specified by docs/design/V3_GSP_CAPTURE_FORMAT.md and
 * implemented by the decoder's reference writer/reader (crates/kf-census/src/capture.rs,
 * branch claude/gsp-census-20261013, commit e2a4618bb). This header follows that format byte
 * for byte; it defines nothing of its own. Anything the decoder's FILE_FIELDS/RECORD_FIELDS
 * tables change must change here, and tests/test_kayfabe_roundtrip.c pins the layout against
 * golden bytes produced by that reference writer (tests/kf-fixture).
 *
 *   FILE   = FILE_HEADER(128 B) RECORD* [END]
 *   RECORD = HEADER(32 B) PAYLOAD(payload_bytes)
 *
 * The module never interprets queue entries for the sake of the format: ELEMENTS carries raw
 * ring entries. (The engine does read a few element fields to find message boundaries and to
 * account for loss; what it writes is still the raw entries.)
 *
 * No board identity exists in either header: vendor/device/revision of the die, the OS, the
 * clock, the driver version string and the decode tag. Nothing else.
 */
#ifndef KFWC_RECORD_H
#define KFWC_RECORD_H

#include "kfwc_wire.h"

#define KFWC_CAP_VERSION 1u
#define KFWC_CAP_FILE_HEADER_BYTES 128u
#define KFWC_CAP_REC_HEADER_BYTES 32u
#define KFWC_CAP_REC_MAGIC 0x4345524bu     /* "KREC" little-endian */
#define KFWC_CAP_MAX_PAYLOAD (1u << 20)    /* decoder-side bound (one ring) */
#define KFWC_CAP_STRING_BYTES 32u
#define KFWC_CAP_OS_WINDOWS 1u
#define KFWC_CAP_OS_LINUX 2u

#define KFWC_K_BIND 1u            /* payload: raw 32-byte msgqTxHeader, copied verbatim */
#define KFWC_K_ELEMENTS 2u        /* payload: entry_count consecutive raw ring entries from first_entry */
#define KFWC_K_LOSS_OVERFLOW 3u   /* module buffer full; first_entry/entry_count = entries dropped */
#define KFWC_K_LOSS_UNREADABLE 4u /* copy faulted or unstable; first_entry/entry_count = the range */
#define KFWC_K_LOSS_UNKNOWN 5u    /* lost track (late attach, overtaken, re-created); count may be unknown */
#define KFWC_K_END 6u             /* clean close; entry_count = records written before it */

#define KFWC_DIR_CMD 0u           /* CPU to GSP command queue */
#define KFWC_DIR_STAT 1u          /* GSP to CPU status queue */
#define KFWC_UNKNOWN_COUNT 0xFFFFFFFFu

typedef struct kfwc_cap_rec {
    kfwc_u32 kind, dir, queue_id;
    kfwc_u32 payload_bytes, counter;
    kfwc_u64 ts;
    kfwc_u32 first_entry, entry_count;
} kfwc_cap_rec;

typedef struct kfwc_cap_file {
    kfwc_u32 os_id, module_version;
    kfwc_u32 pci_vendor_id, pci_device_id, pci_revision_id;
    kfwc_u64 ts_hz, ts_start;
    char driver_version[KFWC_CAP_STRING_BYTES];  /* printable ASCII, NUL terminated, rest zero */
    char decode_tag[KFWC_CAP_STRING_BYTES];
} kfwc_cap_file;

/* Record writer. Returns bytes written, 0 when the buffer is too small or the record is
 * malformed (unknown kind/dir, payload over the bound, BIND not 32 bytes, END/LOSS with a payload). */
kfwc_u32 kfwc_cap_rec_encode(const kfwc_cap_rec *r, const void *payload, kfwc_u8 *out, kfwc_u32 cap);

/* Strict parser (same refusals as the reference reader). KFWC_PARSE_SHORT: need more bytes. */
#define KFWC_PARSE_OK 0
#define KFWC_PARSE_SHORT 1
#define KFWC_PARSE_BAD 2
int kfwc_cap_rec_parse(const kfwc_u8 *buf, kfwc_u32 len, kfwc_cap_rec *r, const kfwc_u8 **payload,
                       kfwc_u32 *consumed);

/* File header. encode returns 128 or 0 (bad string); parse validates reserved bytes and strings. */
kfwc_u32 kfwc_cap_file_encode(const kfwc_cap_file *f, kfwc_u8 out[KFWC_CAP_FILE_HEADER_BYTES]);
int kfwc_cap_file_parse(const kfwc_u8 *buf, kfwc_u32 len, kfwc_cap_file *f);

/* Copy a driver version string into a header field: printable ASCII only, at most 31 bytes.
 * Returns 0 when the input was clean, 1 when characters were dropped or the text truncated. */
int kfwc_cap_set_string(char dst[KFWC_CAP_STRING_BYTES], const char *src);

#endif /* KFWC_RECORD_H */
