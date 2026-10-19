/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * The ONLY door from the library to the machine, and it has no write method.
 *
 * Read-only guarantee, layer 1 (type): kfwc_platform offers read_ram, ram_contains,
 * ram_range, read_reg32 and now. There is no function pointer that stores to RAM, to a
 * register, or to PCI config space, so no library code path can express a store.
 * Layer 2 (whitelist): kfwc_reg_read() refuses every BAR0 offset not listed in
 * kfwc_reg_allowed(), so even the read surface is three registers.
 * Layer 3 (source guard): tests/guard_readonly.sh fails the build if the driver or the
 * library contains a store through a pointer derived from a mapped device range.
 * Layer 4 (hardware): the driver maps BAR0 PAGE_READONLY, so a stray store faults instead of
 * reaching the device.
 */
#ifndef KFWC_PLATFORM_H
#define KFWC_PLATFORM_H

#include "kfwc_wire.h"

typedef struct kfwc_platform {
    void *ctx;
    /* Copy len bytes of ordinary system RAM at physical address pa. Returns 0 on success.
     * MUST refuse (nonzero) any range not entirely inside RAM and never touch MMIO. */
    int (*read_ram)(void *ctx, kfwc_u64 pa, void *dst, kfwc_u32 len);
    /* 1 when [pa, pa+len) lies entirely inside one RAM range, else 0. */
    int (*ram_contains)(void *ctx, kfwc_u64 pa, kfwc_u64 len);
    /* RAM ranges for the scan; returns 0 and fills base/len, nonzero when i is out of range. */
    kfwc_u32 (*ram_range_count)(void *ctx);
    int (*ram_range)(void *ctx, kfwc_u32 i, kfwc_u64 *base, kfwc_u64 *len);
    /* 32-bit BAR0 register read. Returns 0 on success; nonzero when BAR0 is not mapped or
     * the device is not in a readable state. Only whitelisted offsets are ever passed. */
    int (*read_reg32)(void *ctx, kfwc_u32 off, kfwc_u32 *out);
    /* Monotonic tick counter (QueryPerformanceCounter on Windows). */
    kfwc_u64 (*now)(void *ctx);
} kfwc_platform;

/* The complete BAR0 read surface of this module. */
static __inline int kfwc_reg_allowed(kfwc_u32 off)
{
    return off == KFWC_REG_PMC_BOOT_0 || off == KFWC_REG_GSP_MAILBOX0 ||
           off == KFWC_REG_GSP_MAILBOX1 ||
           (off >= KFWC_DISPLAY_CAPS_BASE &&
            off < KFWC_DISPLAY_CAPS_BASE + KFWC_DISPLAY_CAPS_BYTES && (off & 3u) == 0);
}

/* Whitelisted register read. Returns 0 on success, 1 on refusal/failure.
 * An all-ones value (0xFFFFFFFF, bus error) is reported as failure. */
static __inline int kfwc_reg_read(const kfwc_platform *p, kfwc_u32 off, kfwc_u32 *out)
{
    if (!p->read_reg32 || !kfwc_reg_allowed(off))
        return 1;
    if (p->read_reg32(p->ctx, off, out))
        return 1;
    return *out == 0xFFFFFFFFu;
}

/* One fixed page, never a caller-selected physical range. Reject an undersized output,
 * unreadable register or empty page. The driver copies no partial snapshot on failure. */
static __inline int kfwc_display_caps_read(const kfwc_platform *p, kfwc_u32 *words, kfwc_u32 capacity)
{
    kfwc_u32 i, nonzero = 0;
    if (!p || !words || capacity < KFWC_DISPLAY_CAPS_WORDS)
        return 1;
    for (i = 0; i < KFWC_DISPLAY_CAPS_WORDS; i++) {
        if (kfwc_reg_read(p, KFWC_DISPLAY_CAPS_BASE + i * 4u, &words[i]))
            return 1;
        nonzero |= words[i];
    }
    return nonzero == 0;
}

#endif /* KFWC_PLATFORM_H */
