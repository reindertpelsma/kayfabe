# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded ELF64 little-endian DT_NEEDED reader; no readelf dependency."""

import struct
from pathlib import Path


def needed(path):
    size = Path(path).stat().st_size
    with Path(path).open("rb") as stream:

        def read(offset, count):
            if offset < 0 or count < 0 or offset + count > size:
                raise ValueError("ELF range outside file")
            stream.seek(offset)
            data = stream.read(count)
            if len(data) != count:
                raise ValueError("truncated ELF")
            return data

        def unpack(fmt, offset):
            return struct.unpack(fmt, read(offset, struct.calcsize(fmt)))

        if size < 64 or read(0, 6) != b"\x7fELF\x02\x01":
            raise ValueError("expected ELF64 little-endian output")
        phoff = unpack("<Q", 32)[0]
        entsize, count = unpack("<HH", 54)
        if entsize < 56 or count > 4096:
            raise ValueError("invalid ELF program headers")
        loads = []
        dynamic = None
        for i in range(count):
            typ, flags, offset, va, pa, filesz, memsz, align = unpack(
                "<IIQQQQQQ", phoff + i * entsize
            )
            if offset + filesz > size:
                raise ValueError("ELF segment outside file")
            if typ == 1:
                loads.append((va, offset, filesz))
            if typ == 2:
                dynamic = (offset, filesz)
        if dynamic is None or dynamic[1] > 1024**2:
            raise ValueError("missing/oversized ELF dynamic segment")
        offsets = []
        strtab = None
        strsize = None
        for pos in range(dynamic[0], dynamic[0] + dynamic[1], 16):
            tag, value = unpack("<qQ", pos)
            if tag == 0:
                break
            if tag == 1:
                offsets.append(value)
            if tag == 5:
                strtab = value
            if tag == 10:
                strsize = value
        if strtab is None or strsize is None:
            raise ValueError("missing ELF string table")
        mapped = next(
            (
                off + strtab - va
                for va, off, length in loads
                if va <= strtab and strtab + strsize <= va + length
            ),
            None,
        )
        if mapped is None:
            raise ValueError("ELF string table outside load segment")
        result = []
        for offset in offsets:
            if offset >= strsize:
                raise ValueError("ELF DT_NEEDED outside table")
            data = read(mapped + offset, min(512, strsize - offset))
            end = data.find(b"\0")
            if end < 0:
                raise ValueError("unterminated ELF dependency")
            result.append(data[:end].decode("ascii"))
        return sorted(result)


def check_shared(path, libraries):
    deps = needed(path)
    for name in libraries:
        if not any(dep.startswith(name) for dep in deps):
            raise ValueError("missing shared runtime dependency: " + name)
    if any(dep.endswith(".a") for dep in deps):
        raise ValueError("static runtime dependency")
    return deps
