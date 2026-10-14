/*
 * kayfabe host patch tier, H5: the decision logic of the DMA window, kept free of kernel types so
 * that it can be tested in userspace (tools/host_patches/h5_userd_dma/tests).
 * SPDX-License-Identifier: MIT
 *
 * The question it answers: given the width the caller asks for, the device's current DMA mask, and
 * whether the device sits behind a translating IOMMU domain, may the mask be lowered for one call,
 * and to what?
 *
 * It only ever LOWERS the mask. A lower mask makes the IOMMU hand out smaller I/O virtual
 * addresses, which every consumer of a wider mask also accepts; it can never name an address, grant
 * access to memory the caller could not already describe, or widen a window.
 */

#ifndef NV_KF_DMA_WINDOW_H
#define NV_KF_DMA_WINDOW_H

#include "nv-kf-host-patch.h"

/* (1 << bits) - 1 for 1 <= bits <= 63 */
static inline NvU64 nv_kf_dma_mask_for_bits(NvU32 bits)
{
    return (((NvU64)1) << bits) - 1;
}

/* Number of bits of a (contiguous low-bits) DMA mask. */
static inline NvU32 nv_kf_dma_mask_bits(NvU64 mask)
{
    NvU32 n = 0;
    while (mask != 0)
    {
        n++;
        mask >>= 1;
    }
    return n;
}

/*
 * Returns an NV_KF_DMA_WINDOW_* verdict. On NV_KF_DMA_WINDOW_APPLIED, *new_mask is the mask to set
 * for the call; otherwise *new_mask is the current mask (unchanged).
 *
 *   BAD_ARGUMENT    bits outside [MIN, MAX]: refuse, run nothing
 *   DISABLED        the admin turned the feature off: refuse, run nothing
 *   NOT_TRANSLATED  no translating IOMMU domain (none, or identity): the DMA address is the
 *                   physical address or a bounce buffer, a lower mask cannot help and could
 *                   bounce: run the call unchanged
 *   ALREADY_NARROW  the mask is already at most the requested width: run the call unchanged
 *   APPLIED         lower the mask to the requested width for the call
 */
static inline NvU32 nv_kf_dma_window_plan(NvU32 bits, NvU64 cur_mask, int translated, int enabled,
                                          NvU64 *new_mask)
{
    *new_mask = cur_mask;
    if (bits < NV_KF_DMA_WINDOW_BITS_MIN || bits > NV_KF_DMA_WINDOW_BITS_MAX)
        return NV_KF_DMA_WINDOW_BAD_ARGUMENT;
    if (!enabled)
        return NV_KF_DMA_WINDOW_DISABLED;
    if (!translated)
        return NV_KF_DMA_WINDOW_NOT_TRANSLATED;
    if (nv_kf_dma_mask_bits(cur_mask) <= bits)
        return NV_KF_DMA_WINDOW_ALREADY_NARROW;
    *new_mask = nv_kf_dma_mask_for_bits(bits);
    return NV_KF_DMA_WINDOW_APPLIED;
}

#endif /* NV_KF_DMA_WINDOW_H */
