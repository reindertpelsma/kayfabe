# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Minimal deterministic ISO9660 + Joliet NoCloud seed, no mkisofs or root.

Only the three NoCloud files are supported. Joliet supplies the exact lower-case,
long names cloud-init expects. There is one root directory and no extensions.
"""

import struct

BLOCK = 2048
STAMP = b"2026110700000000\x00"
DATE = bytes([126, 11, 7, 0, 0, 0, 0])


def both16(n):
    return struct.pack("<H", n) + struct.pack(">H", n)


def both32(n):
    return struct.pack("<I", n) + struct.pack(">I", n)


def record(name, extent, size, directory=False):
    length = 33 + len(name) + (len(name) % 2 == 0)
    return (
        bytes([length, 0])
        + both32(extent)
        + both32(size)
        + DATE
        + bytes([2 if directory else 0, 0, 0])
        + both16(1)
        + bytes([len(name)])
        + name
        + (b"\x00" if len(name) % 2 == 0 else b"")
    )


def path_table(extent, big=False):
    fmt = ">" if big else "<"
    return (
        b"\x01\x00"
        + struct.pack(fmt + "I", extent)
        + struct.pack(fmt + "H", 1)
        + b"\x00\x00"
    )


def descriptor(kind, total, extent, little, big):
    b = bytearray(BLOCK)
    b[:7] = bytes([kind]) + b"CD001\x01"
    b[8:40] = b"KAYFABE".ljust(32)
    b[40:72] = b"cidata".ljust(32)
    b[80:88] = both32(total)
    if kind == 2:
        b[8:40] = "KAYFABE".ljust(16).encode("utf-16-be")
        b[40:72] = "cidata".ljust(16).encode("utf-16-be")
        b[88:91] = b"%/E"
    b[120:124] = both16(1)
    b[124:128] = both16(1)
    b[128:132] = both16(BLOCK)
    b[132:140] = both32(10)
    b[140:144] = struct.pack("<I", little)
    b[148:152] = struct.pack(">I", big)
    root = record(b"\x00", extent, BLOCK, True)
    b[156 : 156 + len(root)] = root
    # All descriptive identifier fields are fixed spaces, with a matching timestamp.
    b[190:813] = b" " * (813 - 190)
    for off in (813, 830, 847, 864):
        b[off : off + 17] = STAMP
    b[881] = 1
    return bytes(b)


def make_iso(files):
    if set(files) != {"user-data", "meta-data", "network-config"}:
        raise ValueError("seed requires exactly the three NoCloud files")
    contents = []
    extents = {}
    next_block = 26
    for name, data in sorted(files.items()):
        if not isinstance(data, bytes):
            raise ValueError("seed contents must be bytes")
        extents[name] = (next_block, len(data))
        padded = data + b"\x00" * (-len(data) % BLOCK)
        contents.append(padded)
        next_block += len(padded) // BLOCK
    blocks = [bytes(BLOCK) for _ in range(26)]
    blocks[16] = descriptor(1, next_block, 24, 20, 21)
    blocks[17] = descriptor(2, next_block, 25, 22, 23)
    blocks[18] = (b"\xffCD001\x01").ljust(BLOCK, b"\x00")
    for block, extent, big in [
        (20, 24, False),
        (21, 24, True),
        (22, 25, False),
        (23, 25, True),
    ]:
        blocks[block] = path_table(extent, big).ljust(BLOCK, b"\x00")
    for index, joliet in [(24, False), (25, True)]:
        directory = record(b"\x00", index, BLOCK, True) + record(
            b"\x01", index, BLOCK, True
        )
        for name, (extent, size) in sorted(extents.items()):
            encoded = (
                name.encode("utf-16-be")
                if joliet
                else (name.upper().replace("-", "_") + ";1").encode("ascii")
            )
            directory += record(encoded, extent, size)
        if len(directory) > BLOCK:
            raise ValueError("seed directory overflow")
        blocks[index] = directory.ljust(BLOCK, b"\x00")
    return b"".join(blocks + contents)
