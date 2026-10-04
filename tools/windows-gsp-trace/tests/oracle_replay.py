#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Replay published Linux capture payloads through the real observer core.

This reconstructs ring placement and request checksums; it is not a Windows
hardware test. The original Linux recorder hooks TX before checksumming.
"""
import ctypes as C
import importlib.util
from pathlib import Path
import struct
import sys

ROOT = Path(__file__).resolve().parents[3]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


class Cursor(C.Structure):
    _fields_ = [('last', C.c_uint32), ('initialized', C.c_uint32),
                ('gaps', C.c_uint64), ('invalid', C.c_uint64)]


def main():
    original = module('rpctrace', ROOT / 'scripts/rpctrace/decode_rpctrace.py')
    decoder = module('decode', ROOT / 'tools/windows-gsp-trace/decode.py')
    _, records = original.verify_and_parse((ROOT / 'traces/rpctrace_ga106_boot1.bin').read_bytes())
    library = C.CDLL(sys.argv[1])
    captured, frames = [], []
    callback_type = C.CFUNCTYPE(None, C.c_void_p, C.c_void_p, C.c_void_p)

    @callback_type
    def emit(context, record, payload):
        del context
        header = C.string_at(record, 64)
        size = struct.unpack_from('<I', header, 8)[0]
        data = C.string_at(payload, size)
        captured.append(data)
        frames.append(header + data)

    library.kf_queue_records.argtypes = [C.c_void_p, C.c_size_t, C.c_void_p, C.POINTER(Cursor),
                                        C.c_uint32, C.c_uint64, C.c_uint64, callback_type, C.c_void_p]
    rings = [C.create_string_buffer(64 * 4096) for _ in range(2)]
    cursors, slots = [Cursor(), Cursor()], [0, 0]
    scratch = C.create_string_buffer(65536)
    for index, record in enumerate(records):
        direction = record['dir']
        ring = rings[direction]
        if index == 0 or record['elem_seq'] == 0:
            C.memset(ring, 0, len(ring))
            slots[direction], cursors[direction] = 0, Cursor()
            for offset, value in ((4, 64 * 4096), (8, 4096), (12, 63), (24, 32), (28, 4096)):
                C.memmove(C.addressof(ring) + offset, struct.pack('<I', value), 4)
        body = record['body']
        pages = struct.unpack_from('<I', body, 40)[0]
        expected = bytearray(body.ljust((len(body) + 7) & ~7, b'\0'))
        if direction == 0:
            assert struct.unpack_from('<I', expected, 32)[0] == 0
            checksum = 0
            for (word,) in struct.iter_unpack('<I', expected):
                checksum ^= word
            struct.pack_into('<I', expected, 32, checksum)
        data = bytes(expected).ljust(pages * 4096, b'\0')
        for page in range(pages):
            address = C.addressof(ring) + 4096 + (slots[direction] + page) % 63 * 4096
            C.memmove(address, data[page * 4096:(page + 1) * 4096], 4096)
        slots[direction] = (slots[direction] + pages) % 63
        C.memmove(C.addressof(ring) + 16, struct.pack('<I', slots[direction]), 4)
        before = len(captured)
        result = library.kf_queue_records(ring, len(ring), scratch, C.byref(cursors[direction]),
                                           direction, 0x1000, index, emit, None)
        assert result and len(captured) == before + 1 and captured[-1] == expected, index
    header = decoder.FILE.pack(0x5457474b, 1, 64, 64, 10_000_000, 0, 1, 0, 0, 0, 0)
    parsed = decoder.parse(header + b''.join(frames))
    assert len(parsed['records']) == len(records)
    print(f'Linux captured-payload oracle: {len(records)} records recovered and decoded; '
          'both directions, two sessions, reconstructed TX checksums, wrapped and continued messages')


if __name__ == '__main__':
    main()
