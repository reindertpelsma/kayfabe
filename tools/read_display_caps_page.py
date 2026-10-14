#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""T3 MEASUREMENT TOOL (privileged, read-only): read the display capabilities page of an NVIDIA GPU that a
stock driver has up, and print it as text.

The page is NV_PDISP_FE_SW: BAR0 0x640000..0x640fff (class NV*73_DISP_CAPABILITIES). The open RM exposes it
only through a PRIVILEGED display object (RS_FLAGS_ALLOC_PRIVILEGED, ogkm resource_list.h) and no unprivileged
control carries it, so tier 2 does not exist for it and the page is a tier-3 measurement: root, one aligned
32-bit load per word through sysfs `resource0`. The mapping is PROT_READ over 0x641000 bytes of BAR0, nothing
is written, no driver is unbound, no ioctl is issued. A box that is ours, root; the output carries capability
words only (no serial, UUID, VBIOS, MAC: OWNER_RULINGS section AF).

usage:   read_display_caps_page.py <pci-bdf>        e.g. 0000:01:00.0
         read_display_caps_page.py --list           the NVIDIA display functions on this box

output (text, one record per line; `#` lines are comments):
  # kf-display-caps-page v1
  # pci 0000:01:00.0 vendor 0x10de device 0x2786 bar0 0x640000+0x1000 (PROT_READ, 32-bit loads)
  DECISIVE 0x00c 0x00000100            the one unnamed word the Windows display driver was bisected to need
  EVENCAPA <window> <offset> <value>   PRECOMP_WIN_PIPE_HDR_CAPA(window) of windows 0,2,4,6 (bit 19 = SCLR_PRESENT)
  WORD <offset> <value>                every non-zero word of the page, ascending
  # end words=<n>
Exit 0 on success, 1 when the BAR cannot be mapped or the page reads back empty/all-ones, 2 on usage.

A row for kayfabe: take the DECISIVE value, the die's (architecture, implementation) as the RM reports them
(kf3 logs them at realize) and the guest-driver branch, and add
  <arch hex>  <impl hex>  <branch>  <caps class hex>  0x00c  <value>  measured:<die pci id>,<how>,<date>
to crates/kf-disp/data/caps_measured.tsv (docs/design/V3_DISPLAY_CAPS_DERIVATION_20261014.md section 6).
"""
import mmap
import os
import struct
import sys

BASE, SIZE = 0x640000, 0x1000
CAPA = 0x780  # PRECOMP_WIN_PIPE_HDR_CAPA(i) = 0x780 + 0x20 * i, the same in every family's caps class


def read(path):
    with open(path) as f:
        return f.read().strip()


def nvidia_display_functions():
    out = []
    base = "/sys/bus/pci/devices"
    for bdf in sorted(os.listdir(base)):
        try:
            vendor = read(f"{base}/{bdf}/vendor")
            cls = read(f"{base}/{bdf}/class")
        except OSError:
            continue
        if vendor == "0x10de" and cls.startswith("0x03"):
            out.append((bdf, read(f"{base}/{bdf}/device")))
    return out


def main(argv):
    if len(argv) == 2 and argv[1] == "--list":
        for bdf, dev in nvidia_display_functions():
            print(bdf, dev)
        return 0
    if len(argv) != 2 or argv[1].startswith("-"):
        print(__doc__, file=sys.stderr)
        return 2
    bdf = argv[1]
    dev_dir = f"/sys/bus/pci/devices/{bdf}"
    try:
        vendor, dev = read(f"{dev_dir}/vendor"), read(f"{dev_dir}/device")
    except OSError as e:
        print(f"no such PCI function {bdf}: {e}", file=sys.stderr)
        return 2
    if vendor != "0x10de":
        print(f"{bdf} is vendor {vendor}, not NVIDIA", file=sys.stderr)
        return 2
    path = f"{dev_dir}/resource0"
    try:
        fd = os.open(path, os.O_RDONLY | os.O_SYNC)
        m = mmap.mmap(fd, BASE + SIZE, mmap.MAP_SHARED, mmap.PROT_READ)
    except OSError as e:
        print(f"cannot map {path} read-only (root? BAR0 >= 6.5 MiB?): {e}", file=sys.stderr)
        return 1
    words = {}
    for off in range(0, SIZE, 4):
        (v,) = struct.unpack_from("<I", m, BASE + off)
        words[off] = v
    nz = {o: v for o, v in words.items() if v}
    if not nz or all(v == 0xFFFFFFFF for v in words.values()):
        print(f"{bdf}: the page reads back empty or all-ones (display engine not powered / not a display GPU)", file=sys.stderr)
        return 1
    print("# kf-display-caps-page v1")
    print(f"# pci {bdf} vendor {vendor} device {dev} bar0 {BASE:#x}+{SIZE:#x} (PROT_READ, 32-bit loads)")
    print(f"DECISIVE 0x00c {words[0x00c]:#010x}")
    for w in (0, 2, 4, 6):
        print(f"EVENCAPA {w} {CAPA + 0x20 * w:#05x} {words[CAPA + 0x20 * w]:#010x}")
    for off in sorted(nz):
        print(f"WORD {off:#05x} {nz[off]:#010x}")
    print(f"# end words={len(nz)}")
    return 0


sys.exit(main(sys.argv))
