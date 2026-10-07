/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#ifndef KF3_GUEST_RAM_H
#define KF3_GUEST_RAM_H
/* Use the flat section's effective permission, including read-only parents and
 * aliases. On region_del the backing mr may ALREADY have its new permissions;
 * checking memory_region_is_rom(mr) there would retain a stale writable entry.
 * QEMU flatrange_equal compares readonly and emits del/add on permission change.
 */
static inline bool kf3_is_guest_ram(MemoryRegionSection *sec)
{
    return memory_region_is_ram(sec->mr) && !sec->readonly &&
           !memory_region_is_ram_device(sec->mr) && sec->mr->ram_block != NULL;
}
#endif
