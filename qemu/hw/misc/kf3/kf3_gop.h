/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
/*
 * kf3_gop.h — the boot display's one rule about QEMU's own ROM properties (docs/design/V3_DISPLAY.md
 * §4.11.12). Pure C, no QEMU types, so crates/kf-qemu/tests/gop_rom_knobs.rs compiles and runs it
 * alone: kf3.c itself is not compiled by CI.
 *
 * ⊘ CORRECTED 2026-10-03 (v3-gop, the review of v3-gop-kf3). kf3.c used to WARN and carry on when
 * gop=on met romfile= or rombar=0: it registered no ROM, while Rust — already realized with gop=1 —
 * zeroed the store, seeded BAR1, charged the budget, armed the boot layer, carved fn 65's region 0 and
 * logged "boot display ON — option ROM N bytes" for a ROM nobody served. An empty romfile="" (QEMU's
 * idiom for "no option ROM", hw/pci/pci.c pci_add_option_rom) was not even seen, and kf3's ROM was
 * registered against it. Now each is refused by name BEFORE kf3_realize, so the two halves cannot
 * disagree: gop=on means kf3's ROM is the ROM BAR, or the device does not realize.
 */
#ifndef KF3_GOP_H
#define KF3_GOP_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/*
 * NULL when the device may serve its boot-display ROM (or gop is off); otherwise why not.
 *   romfile: PCIDevice.romfile (NULL unless the user set romfile=; kf3's class sets none).
 *   rom_bar: PCIDevice.rom_bar (QEMU 10.2.4: int32, default -1; 0 = no ROM BAR).
 */
static inline const char *kf3_gop_rom_conflict(bool gop, const char *romfile, int32_t rom_bar)
{
    if (!gop) {
        return NULL;
    }
    if (romfile && romfile[0]) {
        return "romfile= names another option ROM, and a device has one ROM BAR";
    }
    if (romfile) {
        return "romfile=\"\" asks for no option ROM";
    }
    if (rom_bar == 0) {
        return "rombar=0 asks for no ROM BAR";
    }
    return NULL;
}

#endif /* KF3_GOP_H */
