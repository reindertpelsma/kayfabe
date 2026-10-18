#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Linux guest boot-tail pattern/readback, only while BAR1 is proven physical.

This tests the reused boot range, not the rest of the store. The padding page at
G-4096 lies above the 1920*1080*4 visible pixels and inside the 64KiB-rounded seed.
Supply the BAR1-mode register/mask/value derived for this die from kf-chip's
bar1mode/hwref table. The script never programs a register or changes a mapping.
"""
import argparse
import hashlib
import json
import mmap
import os
import re
from pathlib import Path

G = 0x7F0000
OFFSET = G - 4096
PATTERN = hashlib.sha256(b"kayfabe bootlayer reboot probe 20261018").digest() * 128


def number(s):
    return int(s, 0)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=("fill", "read-zero"))
    p.add_argument("bdf")
    p.add_argument("--mode-register", required=True, type=number)
    p.add_argument("--mode-mask", required=True, type=number)
    p.add_argument("--physical-value", required=True, type=number)
    a = p.parse_args()
    if not re.fullmatch(r"[0-9a-fA-F]{4}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]", a.bdf):
        p.error("invalid PCI BDF")
    device = Path("/sys/bus/pci/devices") / a.bdf
    if (device / "vendor").read_text().strip() != "0x10de":
        p.error("device is not NVIDIA")
    if (device / "driver").exists():
        p.error("REFUSED: unbind/unload the assigned guest driver first")
    if not (0 <= a.mode_register < 16 * 1024**2 and a.mode_register % 4 == 0 and
            0 < a.mode_mask <= 0xFFFFFFFF and 0 <= a.physical_value <= 0xFFFFFFFF and
            a.physical_value & ~a.mode_mask == 0):
        p.error("invalid register descriptor")
    page = a.mode_register & ~4095
    with (device / "resource0").open("rb", buffering=0) as f:
        with mmap.mmap(f.fileno(), 4096, access=mmap.ACCESS_READ, offset=page) as bar0:
            off = a.mode_register - page
            mode = int.from_bytes(bar0[off:off + 4], "little")
    if mode & a.mode_mask != a.physical_value:
        p.error("REFUSED: BAR1 is not in the supplied physical mode")
    flags = os.O_RDWR if a.action == "fill" else os.O_RDONLY
    fd = os.open(device / "resource1", flags)
    try:
        access = mmap.ACCESS_WRITE if a.action == "fill" else mmap.ACCESS_READ
        with mmap.mmap(fd, 4096, access=access, offset=OFFSET) as fb:
            if a.action == "fill":
                fb[:] = PATTERN
            got = fb[:]
    finally:
        os.close(fd)
    expected = PATTERN if a.action == "fill" else bytes(4096)
    result = {"action": a.action, "bar1_offset": OFFSET, "length": len(got),
              "sha256": hashlib.sha256(got).hexdigest(), "matched": got == expected,
              "scope": "BAR1 physical boot tail; corroborate with store diagnostic"}
    print(json.dumps(result))
    return 0 if got == expected else 1


if __name__ == "__main__":
    raise SystemExit(main())
