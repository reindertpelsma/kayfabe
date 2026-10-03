/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
/*
 * kf3_dma.h — the device's DMA-regime rules (docs/design/V3_VIOMMU.md §4.2). Pure C, no QEMU types,
 * so crates/kf-qemu/tests/dma_regime.rs compiles and runs them alone: kf3.c itself is not compiled
 * by CI. kf3.c feeds them what QEMU shows (the sections of the device's DMA address space, the
 * amd-iommu object's properties) and publishes the result with kf3_dma_regime().
 *
 * Every rule here fails closed: when the state cannot be read, the answer refuses.
 */
#ifndef KF3_DMA_H
#define KF3_DMA_H

#include <stdbool.h>
#include <stdint.h>
#include "kf3.h"

/*
 * The regime of a device whose DMA address space is NOT system memory (a vIOMMU is in front of it),
 * from the sections that address space shows:
 *   - untracked (kf3_dma_amd_untracked) wins: QEMU's model does not follow the guest there;
 *   - any IOMMU section: the guest translates this device -> TRANSLATING;
 *   - RAM and no IOMMU section: a pass-through view -> IDENTITY. Identity needs this positive evidence;
 *   - neither: BLOCKED. Every vIOMMU switches a device by disabling one region and then enabling the
 *     other, in two commits, so a real switch passes through this state, which refuses.
 */
static inline uint32_t kf3_dma_classify(bool untracked, uint32_t iommu_sections, uint32_t ram_sections)
{
    if (untracked) {
        return KF3_DMA_UNTRACKED;
    }
    if (iommu_sections > 0) {
        return KF3_DMA_TRANSLATING;
    }
    return ram_sections > 0 ? KF3_DMA_IDENTITY : KF3_DMA_BLOCKED;
}

/*
 * Whether an amd-iommu in the machine leaves this device's address kind unknowable. QEMU 10.2.4 never
 * enables that model's IOMMU region while dma-remap is off (its default), yet with DMA translation
 * still advertised nothing tells the guest, which may translate this device anyway
 * (hw/i386/amd_iommu.c:1109-1122, :1259-1261, :2657).
 *   found:      an amd-iommu object exists
 *   ambiguous:  more than one does (refused rather than picked)
 *   readable:   both properties were read (a property this file does not know refuses)
 */
static inline bool kf3_dma_amd_untracked(bool found, bool ambiguous, bool readable, bool dma_remap,
                                         bool dma_translation)
{
    if (ambiguous) {
        return true;
    }
    if (!found) {
        return false;
    }
    if (!readable) {
        return true;
    }
    return !dma_remap && dma_translation;
}

/* The regime's name, as Rust's status line spells it (any other value is Unset). */
static inline const char *kf3_dma_name(uint32_t r)
{
    switch (r) {
    case KF3_DMA_DIRECT: return "Direct";
    case KF3_DMA_IDENTITY: return "Identity";
    case KF3_DMA_TRANSLATING: return "Translating";
    case KF3_DMA_UNTRACKED: return "Untracked";
    case KF3_DMA_BLOCKED: return "Blocked";
    default: return "Unset";
    }
}

#endif /* KF3_DMA_H */
