/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Queue element framing: peek, validate, checksum. Read-only, allocation-free.
 * Mirrors kf-gsp element.rs (the same arithmetic in the decoding direction).
 */
#ifndef KFWC_ELEM_H
#define KFWC_ELEM_H

#include "kfwc_wire.h"

typedef struct kfwc_elem_info {
    kfwc_u32 layout;       /* KFWC_LAYOUT_* that matched */
    kfwc_u32 seq;          /* transport seqNum */
    kfwc_u32 elems;        /* elements this message occupies */
    kfwc_u32 msg_bytes;    /* hdr_size + rpc.length (unpadded) */
    kfwc_u32 rpc_version, rpc_length, rpc_function, rpc_result, rpc_sequence;
} kfwc_elem_info;

#define KFWC_ELEM_OK 0
#define KFWC_ELEM_INVALID 1       /* not a well-formed element of this layout (stale, torn, zero) */
#define KFWC_ELEM_UNSUPPORTED 2   /* well-formed envelope but nonzero CC authentication bytes */

const kfwc_layout *kfwc_layout_get(kfwc_u32 id);

/* Examine the FIRST element of a run (4096 bytes at `first`). Does not check the checksum. */
int kfwc_elem_peek(const kfwc_layout *l, const kfwc_u8 *first, kfwc_elem_info *info);

/* XOR fold over the whole message (hdr+length rounded up to 8) must be zero.
 * `run` must hold info->elems * KFWC_PAGE bytes. Returns 1 when valid. */
int kfwc_elem_checksum_ok(const kfwc_layout *l, const kfwc_u8 *run, const kfwc_elem_info *info);

/* Fold used by the transport (64-bit XOR, hi ^ lo), exposed for tests. */
kfwc_u32 kfwc_fold32(const kfwc_u8 *p, kfwc_u32 len_rounded8);

#endif /* KFWC_ELEM_H */
