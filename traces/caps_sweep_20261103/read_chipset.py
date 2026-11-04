#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Read-only identity probe for the caps sweep: NV_PMC_BOOT_0 (0x0) and NV_PMC_BOOT_42 (0xa00) of BAR0 through the
same sysfs `resource0` PROT_READ mapping that tools/read_display_caps_page.py uses. Prints the raw words and the
decoded (architecture, implementation) = (chipset & 0x1f0, chipset & 0xf), chipset = BOOT_42 bits 28:20 (a Turing+
die reports it there). Nothing is written; no serial, UUID or VBIOS is read.

usage: read_chipset.py <pci-bdf>
"""
import mmap
import os
import struct
import sys

bdf = sys.argv[1]
fd = os.open(f"/sys/bus/pci/devices/{bdf}/resource0", os.O_RDONLY | os.O_SYNC)
m = mmap.mmap(fd, 0x1000, mmap.MAP_SHARED, mmap.PROT_READ)
boot0 = struct.unpack_from("<I", m, 0)[0]
boot42 = struct.unpack_from("<I", m, 0xA00)[0]
chipset = (boot42 >> 20) & 0x1FF
print(f"BOOT_0 {boot0:#010x} BOOT_42 {boot42:#010x}")
print(f"CHIPSET {chipset:#05x} ARCH {chipset & 0x1F0:#05x} IMPL {chipset & 0xF:#x}")
