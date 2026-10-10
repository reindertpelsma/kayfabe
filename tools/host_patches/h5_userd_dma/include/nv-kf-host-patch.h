/*
 * kayfabe host patch tier: ioctl ABI of the kayfabe additions to nvidia.ko.
 * SPDX-License-Identifier: MIT
 *
 * Two escapes, both on the nvidia character devices, both outside the RM
 * escape range (RM uses 0x27..0x5F, the frontend uses NV_IOCTL_BASE+0..+18):
 *
 *   NV_ESC_KF_QUERY                    what this build provides (H0).
 *                                      Stock builds do not know the escape;
 *                                      nvidia_ioctl() answers it with -EINVAL
 *                                      (nv_validate_ioctls(), "unknown NVRM
 *                                      ioctl command").
 *   NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW  H5: NV_ESC_RM_ALLOC_MEMORY with the
 *                                      device DMA mask lowered for the duration
 *                                      of that one call, so the DMA address that
 *                                      RM gives an OS descriptor fits N bits.
 *
 * Neither escape adds an RM verb, a mapping, or any way to name an address.
 */

#ifndef NV_KF_HOST_PATCH_H
#define NV_KF_HOST_PATCH_H

#include <nv-ioctl-numbers.h>
#include <nvtypes.h>

/* NV_IOCTL_BASE + 55 is the last number an ioctl can have (see nv-ioctl-numbers.h). */
#define NV_ESC_KF_QUERY                    (NV_IOCTL_BASE + 40)
#define NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW  (NV_IOCTL_BASE + 41)

/*
 * The one RM escape the window may wrap: NV_ESC_RM_ALLOC_MEMORY (0x27, a userspace ABI constant of
 * src/nvidia/arch/nvalloc/unix/include/nv_escape.h, which is RM-core-private and not in the
 * packaged kernel-open tree).
 */
#define NV_KF_ESC_RM_ALLOC_MEMORY          0x27

/* ABI of the query struct itself. */
#define NV_KF_QUERY_ABI                    1

/* Feature bits of nv_ioctl_kf_query_t.features; feature_abi[bit] is that feature's own ABI. */
#define NV_KF_FEATURE_H5_DMA_WINDOW        0x1u
#define NV_KF_FEATURE_H5_DMA_WINDOW_BIT    0
#define NV_KF_H5_DMA_WINDOW_ABI            1

typedef struct nv_ioctl_kf_query
{
    NvU32 flags;            /* in:  must be 0                               */
    NvU32 abi;              /* out: NV_KF_QUERY_ABI                         */
    NvU32 features;         /* out: NV_KF_FEATURE_* present AND enabled     */
    NvU32 reserved;         /* out: 0                                       */
    NvU8  feature_abi[8];   /* out: per-feature ABI, indexed by bit number  */
} nv_ioctl_kf_query_t;      /* 24 bytes */

/* Verdicts of nv_ioctl_kf_alloc_mem_dma_window_t.verdict (see nv-kf-dma-window.h). */
#define NV_KF_DMA_WINDOW_APPLIED           0
#define NV_KF_DMA_WINDOW_BAD_ARGUMENT      1
#define NV_KF_DMA_WINDOW_DISABLED          2
#define NV_KF_DMA_WINDOW_NOT_TRANSLATED    3
#define NV_KF_DMA_WINDOW_ALREADY_NARROW    4
#define NV_KF_DMA_WINDOW_MASK_REFUSED      5

/* The USERD pointer is 40 bits wide on Turing, Ampere and Ada; nothing narrower is ever needed,
 * and a floor of 2^40 (1 TiB) of I/O virtual address space cannot be exhausted by one client. */
#define NV_KF_DMA_WINDOW_BITS_MIN          40
#define NV_KF_DMA_WINDOW_BITS_MAX          63
#define NV_KF_DMA_WINDOW_INNER_MAX         512

typedef struct nv_ioctl_kf_alloc_mem_dma_window
{
    NvU32 abi;              /* in:  NV_KF_H5_DMA_WINDOW_ABI                           */
    NvU32 dma_bits;         /* in:  address bits the mapping must fit, MIN..MAX       */
    NvU32 flags;            /* in:  must be 0                                         */
    NvU32 inner_size;       /* in:  size of the NV_ESC_RM_ALLOC_MEMORY argument       */
    NvU64 inner_ptr  NV_ALIGN_BYTES(8); /* in: user VA of that argument (in/out)      */
    NvU32 applied;          /* out: 1 if the mask was lowered for the call            */
    NvU32 verdict;          /* out: NV_KF_DMA_WINDOW_*                                */
    NvU32 mask_bits;        /* out: DMA mask width in effect while RM ran             */
    NvU32 reserved;         /* out: 0                                                 */
} nv_ioctl_kf_alloc_mem_dma_window_t; /* 40 bytes */

#endif /* NV_KF_HOST_PATCH_H */
