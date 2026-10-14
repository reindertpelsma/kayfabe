#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Reject known-invalid x64 driver image flags before signing or staging."""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def inspect(path):
    data = path.read_bytes()

    def unpack(fmt, offset):
        if offset < 0 or offset + struct.calcsize(fmt) > len(data):
            raise ValueError('truncated PE structure')
        return struct.unpack_from(fmt, data, offset)

    if data[:2] != b'MZ':
        raise ValueError('missing DOS signature')
    pe, = unpack('<I', 0x3c)
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('missing PE signature')
    machine, count, _, _, _, optional_size, characteristics = unpack('<HHIIIHH', pe + 4)
    optional = pe + 24
    magic, = unpack('<H', optional)
    if machine != 0x8664 or magic != 0x20b or optional_size < 240 or not 1 <= count <= 96:
        raise ValueError('expected bounded AMD64 PE32+ image')
    if characteristics & 1:
        raise ValueError('driver base relocations are stripped')
    entry, = unpack('<I', optional + 16)
    image_base, = unpack('<Q', optional + 24)
    image_size, = unpack('<I', optional + 56)
    subsystem, flags = unpack('<HH', optional + 68)
    if subsystem != 1 or flags & 0x8000:
        raise ValueError('native driver required; TSAWARE causes kernel loader rejection')
    if flags & 0x1c0 != 0x1c0:
        raise ValueError('ASLR, NX and integrity checking must remain enabled')
    sections = []
    for index in range(count):
        name, virtual_size, rva, raw_size, raw, _, _, _, _, attributes = unpack('<8sIIIIIIHHI', optional + optional_size + index * 40)
        name = name.rstrip(b'\0').decode('ascii', errors='strict')
        if rva + virtual_size > image_size or raw + raw_size > len(data):
            raise ValueError(f'out-of-bounds section {name}')
        if virtual_size and not attributes & 0x40000000:
            raise ValueError(f'unreadable mapped section {name}; loader may fault before entry')
        if name in ('.text', '.rdata', '.data', '.pdata') and not attributes & 0x08000000:
            raise ValueError(f'kernel section {name} must be marked nonpageable')
        sections.append(dict(name=name, rva=rva, size=virtual_size, raw=raw, raw_size=raw_size, attributes=attributes))

    def offset(rva, size):
        for section in sections:
            displacement = rva - section['rva']
            if displacement >= 0 and displacement + size <= section['raw_size']:
                return section['raw'] + displacement
        raise ValueError('data directory does not fit a backed section')

    if not any(s['rva'] <= entry < s['rva'] + s['size'] and s['attributes'] & 0x20000000 for s in sections):
        raise ValueError('entry point must be in executable code')
    reloc, reloc_size = unpack('<II', optional + 112 + 5 * 8)
    if reloc_size < 8:
        raise ValueError('missing base relocations')
    offset(reloc, reloc_size)
    config, config_size = unpack('<II', optional + 112 + 10 * 8)
    if config_size < 0x60:
        raise ValueError('missing GS load configuration')
    config_offset = offset(config, config_size)
    cookie, = unpack('<Q', config_offset + 0x58)
    cookie_rva = cookie - image_base
    if not any(s['rva'] <= cookie_rva and cookie_rva + 8 <= s['rva'] + s['size'] and s['attributes'] & 0x80000000 for s in sections):
        raise ValueError('GS security cookie must reference writable image data')
    return dict(schema='kayfabe-driver-pe-check/1', image=path.name,
                sha256=hashlib.sha256(data).hexdigest(), dll_characteristics=flags,
                sections=sections, static_checks_passed=True, windows_runtime_tested=False)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image', type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(inspect(args.image), indent=2))
    except (OSError, ValueError, struct.error) as error:
        parser.exit(1, f'REFUSED: {error}\n')
