#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""T3 MEASUREMENT TOOL (privileged, read-only): print the display capabilities page of the GPU a stock driver
has up, as `<byte offset> <value>` for every non-zero 32-bit word.

The page is NV_PDISP_FE_SW (BAR0 0x640000..0x640fff, class NV*73_DISP_CAPABILITIES); the open RM exposes it
only through a PRIVILEGED display object (`RS_FLAGS_ALLOC_PRIVILEGED`, ogkm resource_list.h), so the one
unprivileged-host route (tier 2) does not exist and the page is a tier-3 measurement: root, one aligned
32-bit read per word through sysfs `resource0`. Nothing is written, no driver is touched.

usage: read_display_caps_page.py 0000:01:00.0 > caps_<die>_<driver>.txt     (run as root, on a box that is ours)
The output carries no serial, UUID or board identity: capability words only (OWNER_RULINGS §AF).
"""
import mmap, os, struct, sys

BASE, SIZE = 0x640000, 0x1000


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    path = f"/sys/bus/pci/devices/{argv[1]}/resource0"
    fd = os.open(path, os.O_RDONLY | os.O_SYNC)
    try:
        m = mmap.mmap(fd, BASE + SIZE, mmap.MAP_SHARED, mmap.PROT_READ)
    except OSError as e:
        print(f"cannot map {path}: {e}", file=sys.stderr)
        return 1
    dev = open(f"/sys/bus/pci/devices/{argv[1]}/device").read().strip()
    print(f"# display caps page, PCI device id {dev}, BAR0 {BASE:#x}+{SIZE:#x}, read-only 32-bit reads")
    for off in range(0, SIZE, 4):
        (v,) = struct.unpack_from("<I", m, BASE + off)
        if v:
            print(f"{off:#05x} {v:#010x}")
    return 0


sys.exit(main(sys.argv))
