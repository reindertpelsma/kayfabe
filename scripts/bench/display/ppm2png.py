#!/usr/bin/env python3
"""ppm2png.py in.ppm out.png [width] — P6 PPM to PNG, standard library only (the bench host has no
image tools), optional nearest-neighbour downscale to `width` (aspect kept). Used to bring host
screendumps back as PNG (traces/v3_display/dispsw_20261003/)."""
import struct
import sys
import zlib


def read_ppm(path):
    data = open(path, "rb").read()
    fields, pos = [], 0
    while len(fields) < 4:
        while data[pos:pos + 1].isspace():
            pos += 1
        if data[pos:pos + 1] == b"#":
            while data[pos:pos + 1] not in (b"\n", b""):
                pos += 1
            continue
        start = pos
        while not data[pos:pos + 1].isspace():
            pos += 1
        fields.append(data[start:pos])
    if fields[0] != b"P6":
        raise SystemExit("not a P6 PPM")
    w, h, maxv = int(fields[1]), int(fields[2]), int(fields[3])
    if maxv != 255:
        raise SystemExit("maxval != 255")
    pos += 1
    return w, h, data[pos:pos + w * h * 3]


def main():
    src, dst = sys.argv[1], sys.argv[2]
    w, h, px = read_ppm(src)
    tw = int(sys.argv[3]) if len(sys.argv) > 3 else w
    th = max(1, h * tw // w)
    rows = []
    for y in range(th):
        sy = y * h // th
        line = px[sy * w * 3:(sy + 1) * w * 3]
        if tw == w:
            rows.append(b"\x00" + line)
        else:
            out = bytearray(tw * 3)
            for x in range(tw):
                sx = x * w // tw
                out[x * 3:x * 3 + 3] = line[sx * 3:sx * 3 + 3]
            rows.append(b"\x00" + bytes(out))
    raw = zlib.compress(b"".join(rows), 9)

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", tw, th, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", raw) + chunk(b"IEND", b"")
    open(dst, "wb").write(png)
    print(f"{dst} {tw}x{th} {len(png)} bytes")


main()
