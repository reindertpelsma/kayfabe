/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include "kfwc_elem.h"

static const kfwc_layout k_pre610 = {
    KFWC_LAYOUT_PRE610, 48, 32, 36, 40, KFWC_NO_FIELD, KFWC_NO_FIELD, 0, 0};
/* 610: mctpHeader@0 = 0xC0000001 (SOM|EOM|version 1), nvdmHeader@4 = 0x2510DE7E (kf-abi). */
static const kfwc_layout k_610 = {
    KFWC_LAYOUT_610, 16, 8, 12, KFWC_NO_FIELD, 0, 4, 0xC0000001u, 0x2510DE7Eu};

const kfwc_layout *kfwc_layout_get(kfwc_u32 id)
{
    if (id == KFWC_LAYOUT_PRE610)
        return &k_pre610;
    if (id == KFWC_LAYOUT_610)
        return &k_610;
    return 0;
}

kfwc_u32 kfwc_fold32(const kfwc_u8 *p, kfwc_u32 len_rounded8)
{
    kfwc_u64 acc = 0;
    kfwc_u32 i;
    for (i = 0; i + 8 <= len_rounded8; i += 8)
        acc ^= kfwc_get64(p + i);
    return (kfwc_u32)(acc >> 32) ^ (kfwc_u32)acc;
}

int kfwc_elem_peek(const kfwc_layout *l, const kfwc_u8 *first, kfwc_elem_info *info)
{
    const kfwc_u8 *rpc = first + l->hdr_size;
    kfwc_u32 length, elems, i;
    if (kfwc_get32(rpc + KFWC_RPC_OFF_VERSION) != KFWC_RPC_VERSION ||
        kfwc_get32(rpc + KFWC_RPC_OFF_SIGNATURE) != KFWC_RPC_SIGNATURE)
        return KFWC_ELEM_INVALID;
    length = kfwc_get32(rpc + KFWC_RPC_OFF_LENGTH);
    if (length < KFWC_RPC_BYTES || length > KFWC_ELEM_SIZE_MAX - l->hdr_size)
        return KFWC_ELEM_INVALID;
    elems = (l->hdr_size + length + KFWC_PAGE - 1u) / KFWC_PAGE;
    if (elems == 0 || elems > KFWC_MAX_RUN_ELEMS)
        return KFWC_ELEM_INVALID;
    if (l->count_off != KFWC_NO_FIELD && kfwc_get32(first + l->count_off) != elems)
        return KFWC_ELEM_INVALID;
    if (l->mctp_off != KFWC_NO_FIELD &&
        (kfwc_get32(first + l->mctp_off) != l->mctp_word ||
         kfwc_get32(first + l->nvdm_off) != l->nvdm_word))
        return KFWC_ELEM_INVALID;
    info->layout = l->id;
    info->seq = kfwc_get32(first + l->seq_off);
    info->elems = elems;
    info->msg_bytes = l->hdr_size + length;
    info->rpc_version = kfwc_get32(rpc + KFWC_RPC_OFF_VERSION);
    info->rpc_length = length;
    info->rpc_function = kfwc_get32(rpc + KFWC_RPC_OFF_FUNCTION);
    info->rpc_result = kfwc_get32(rpc + KFWC_RPC_OFF_RESULT);
    info->rpc_sequence = kfwc_get32(rpc + KFWC_RPC_OFF_SEQUENCE);
    if (l->id == KFWC_LAYOUT_PRE610) {
        /* authTagBuffer / aadBuffer: zero unless Confidential Compute is on. */
        for (i = 0; i < 32; i++)
            if (first[i])
                return KFWC_ELEM_UNSUPPORTED;
    }
    return KFWC_ELEM_OK;
}

int kfwc_elem_checksum_ok(const kfwc_layout *l, const kfwc_u8 *run, const kfwc_elem_info *info)
{
    (void)l;
    return kfwc_fold32(run, (info->msg_bytes + 7u) & ~7u) == 0;
}
