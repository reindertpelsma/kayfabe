/*
 * GPU-free test of the H5 address-constraint logic (include/nv-kf-dma-window.h) against a model of
 * the two things it has to satisfy:
 *
 *   1. the host RM's USERD address check (kchannelIsUserdAddrSizeValid_*, read from ogkm 595.84:
 *      GV100 HAL = Turing, GA100 HAL = Ampere and Ada: Lo = bits 31:8 of the shifted address,
 *      Hi = 8 bits, 40 address bits in all; GH100 HAL = Hopper and newer: Hi = 20 bits),
 *   2. the Linux IOMMU layer's allocation rule (drivers/iommu/dma-iommu.c, v7.0, read:
 *      iommu_dma_alloc_iova takes dma_get_mask(dev), then alloc_iova_fast(limit, true) = the
 *      highest free range at or below the limit).
 *
 * The model is a model: it proves the logic and the arithmetic, not the kernel. The measured case
 * it must reproduce is run 61 (traces/windows_code43_walls_20261007/README.md): USERD at IOVA
 * 0x7ff9_f969_b000, "userdAddrHi=0x00007ff9, userAddrLo=0x007cb4d8 is incorrect".
 */
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "nv-kf-dma-window.h"

#define USERD_SHIFT 9u /* 512-byte USERD: run 61's Lo 0x7cb4d8 == 0xf969b000 >> 9 */

static int valid_ga100(NvU32 lo, NvU32 hi) /* SF_MASK(PTR_LO)=24 bits, SF_MASK(PTR_HI_HW)=8 bits */
{
    return ((lo & 0x00ffffffu) == lo) && ((hi & 0xffu) == hi);
}
static int valid_gh100(NvU32 lo, NvU32 hi) /* Hi = 20 bits */
{
    return ((lo & 0x00ffffffu) == lo) && ((hi & 0xfffffu) == hi);
}
static int userd_ok(NvU64 addr, int (*valid)(NvU32, NvU32))
{
    return valid((NvU32)(addr & 0xffffffffu) >> USERD_SHIFT, (NvU32)(addr >> 32));
}

/* Top-down allocator: the highest page-aligned range of `pages` pages that ends at or below mask. */
static NvU64 next_free_top;
static void alloc_reset(NvU64 limit_mask) { next_free_top = (limit_mask & ~0xfffull) + 0x1000; }
static NvU64 alloc_top_down(NvU64 pages, NvU64 mask)
{
    NvU64 top = next_free_top;
    NvU64 limit_top = (mask & ~0xfffull) + 0x1000;
    if (top > limit_top) top = limit_top;
    top -= pages * 0x1000;
    next_free_top = top;
    return top;
}

static void test_plan_table(void)
{
    NvU64 m;
    const NvU64 m47 = nv_kf_dma_mask_for_bits(47), m40 = nv_kf_dma_mask_for_bits(40);

    assert(nv_kf_dma_mask_bits(m47) == 47 && nv_kf_dma_mask_bits(m40) == 40);
    assert(nv_kf_dma_mask_for_bits(63) == 0x7fffffffffffffffull);

    /* the case the patch exists for: 47-bit device, translated, asks for 40 */
    assert(nv_kf_dma_window_plan(40, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_APPLIED && m == m40);
    /* only ever lowers */
    assert(nv_kf_dma_window_plan(47, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_ALREADY_NARROW && m == m47);
    assert(nv_kf_dma_window_plan(52, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_ALREADY_NARROW && m == m47);
    assert(nv_kf_dma_window_plan(40, m40, 1, 1, &m) == NV_KF_DMA_WINDOW_ALREADY_NARROW && m == m40);
    assert(nv_kf_dma_window_plan(40, nv_kf_dma_mask_for_bits(36), 1, 1, &m) == NV_KF_DMA_WINDOW_ALREADY_NARROW
           && m == nv_kf_dma_mask_for_bits(36));
    /* no translation (none / identity / bounce): never touch the mask */
    assert(nv_kf_dma_window_plan(40, m47, 0, 1, &m) == NV_KF_DMA_WINDOW_NOT_TRANSLATED && m == m47);
    /* admin switch */
    assert(nv_kf_dma_window_plan(40, m47, 1, 0, &m) == NV_KF_DMA_WINDOW_DISABLED && m == m47);
    /* argument bounds: 40..63 (the USERD pointer width is the floor) */
    assert(nv_kf_dma_window_plan(39, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_BAD_ARGUMENT && m == m47);
    assert(nv_kf_dma_window_plan(32, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_BAD_ARGUMENT && m == m47);
    assert(nv_kf_dma_window_plan(0, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_BAD_ARGUMENT);
    assert(nv_kf_dma_window_plan(64, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_BAD_ARGUMENT);
    assert(nv_kf_dma_window_plan(0xffffffffu, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_BAD_ARGUMENT);
    assert(nv_kf_dma_window_plan(63, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_ALREADY_NARROW && m == m47);
    assert(nv_kf_dma_window_plan(63, nv_kf_dma_mask_for_bits(63), 1, 1, &m) == NV_KF_DMA_WINDOW_ALREADY_NARROW);
    assert(nv_kf_dma_window_plan(41, m47, 1, 1, &m) == NV_KF_DMA_WINDOW_APPLIED && m == nv_kf_dma_mask_for_bits(41));
    /* bad argument wins over disabled, disabled over translation state */
    assert(nv_kf_dma_window_plan(5, m47, 0, 0, &m) == NV_KF_DMA_WINDOW_BAD_ARGUMENT);
    assert(nv_kf_dma_window_plan(40, m47, 0, 0, &m) == NV_KF_DMA_WINDOW_DISABLED);
    puts("plan table: ok");
}

static void test_property_never_widens(void)
{
    /* For every mask width and request, the planned mask is never wider than the current one. */
    for (NvU32 cur = 1; cur <= 63; cur++)
        for (NvU32 req = 0; req <= 70; req++)
            for (int tr = 0; tr <= 1; tr++)
                for (int en = 0; en <= 1; en++) {
                    NvU64 cm = nv_kf_dma_mask_for_bits(cur), nm;
                    NvU32 v = nv_kf_dma_window_plan(req, cm, tr, en, &nm);
                    assert(nm <= cm);
                    if (v == NV_KF_DMA_WINDOW_APPLIED) {
                        assert(tr && en && req >= 40 && req <= 63 && req < cur);
                        assert(nm == nv_kf_dma_mask_for_bits(req));
                    } else {
                        assert(nm == cm);
                    }
                }
    puts("never widens: ok");
}

static void test_run61_reproduction(void)
{
    /* USERD slot 0 of a page mapped at the measured IOVA, GA100-family predicate. */
    NvU64 addr = 0x7ff9f969b000ull;
    NvU32 lo = (NvU32)(addr & 0xffffffffu) >> USERD_SHIFT, hi = (NvU32)(addr >> 32);
    assert(lo == 0x007cb4d8u && hi == 0x00007ff9u); /* the numbers in the host's dmesg */
    assert(!valid_ga100(lo, hi));                   /* refused */
    assert(valid_gh100(lo, hi));                    /* Hopper+ field is 20 bits: no wall there */
    puts("run 61 numbers reproduced (refused on 40-bit families, fits on Hopper+): ok");
}

static void test_guest_ram_descriptor(void)
{
    /* A whole-RAM descriptor, 64 GiB, mapped under the 47-bit mask: the high slots are refused;
     * under the planned 40-bit mask every 512-byte slot of every page is accepted. */
    const NvU64 pages = (64ull << 30) / 4096;
    NvU64 m;
    int bad47 = 0, bad40 = 0;

    alloc_reset(nv_kf_dma_mask_for_bits(47));
    NvU64 base47 = alloc_top_down(pages, nv_kf_dma_mask_for_bits(47));
    assert(nv_kf_dma_window_plan(40, nv_kf_dma_mask_for_bits(47), 1, 1, &m) == NV_KF_DMA_WINDOW_APPLIED);
    alloc_reset(m);
    NvU64 base40 = alloc_top_down(pages, m);

    for (NvU64 p = 0; p < pages; p += 997) /* sample */
        for (NvU32 slot = 0; slot < 8; slot++) {
            bad47 += !userd_ok(base47 + p * 4096 + slot * 512, valid_ga100);
            bad40 += !userd_ok(base40 + p * 4096 + slot * 512, valid_ga100);
        }
    assert(bad47 > 0);
    assert(bad40 == 0);
    assert(base40 + pages * 4096 - 1 <= m);
    printf("64 GiB descriptor: refused slots at 47 bits = %d, at 40 bits = %d: ok\n", bad47, bad40);
}

static void test_random(void)
{
    srand(12345);
    for (int i = 0; i < 100000; i++) {
        NvU32 devbits = 40 + (rand() % 12);          /* 40..51-bit capable device (>= req or not) */
        NvU32 req = 40 + (rand() % 24);
        NvU64 cur = nv_kf_dma_mask_for_bits(devbits), m;
        NvU32 v = nv_kf_dma_window_plan(req, cur, 1, 1, &m);
        NvU64 pages = 1 + (rand() % 100000);
        alloc_reset(m);
        NvU64 a = alloc_top_down(pages, m);
        assert(a + pages * 4096 - 1 <= m);           /* the mapping honours the mask */
        if (v == NV_KF_DMA_WINDOW_APPLIED && req == 40) /* the USERD case: 40 bits => always valid */
            assert(userd_ok(a + (pages - 1) * 4096 + 7 * 512, valid_ga100));
    }
    puts("random: ok");
}

static void test_abi_sizes(void)
{
    assert(sizeof(nv_ioctl_kf_query_t) == 24);
    assert(sizeof(nv_ioctl_kf_alloc_mem_dma_window_t) == 40);
    assert(offsetof(nv_ioctl_kf_alloc_mem_dma_window_t, inner_ptr) == 16);
    assert(NV_ESC_KF_QUERY == 240 && NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW == 241);
    assert(NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW - NV_IOCTL_BASE <= 55); /* ioctl numbers above 255 overflow */
    puts("abi sizes/numbers: ok");
}

int main(void)
{
    test_abi_sizes();
    test_plan_table();
    test_property_never_widens();
    test_run61_reproduction();
    test_guest_ram_descriptor();
    test_random();
    puts("ALL OK");
    return 0;
}
