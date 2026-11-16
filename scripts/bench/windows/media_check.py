#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-side readers for the files an application check produces (stdlib only; every reader is bounded and never executes anything).

STATUS: LIVE, 2026-11-16 (GPU-free tests: scripts/ci/test_media_check.py).
png_info(path)      -> dict(width, height, channels, mean, std, distinct) from the decoded pixels (8-bit, non-interlaced PNG)
jpeg_size(path)     -> (width, height)
psd_info(path)      -> dict(width, height, channels, depth)
pdf_info(path)      -> dict(pages, version, bytes)
mp4_info(path)      -> dict(duration_s, width, height, codec, brand) from the ISO base media boxes
Raise ValueError on a file that is not what its name says.
"""
import struct
import zlib
from pathlib import Path

MAX_PNG = 64 * 1024 * 1024
PNG_SIG = b"\x89PNG\r\n\x1a\n"


def _paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)


def png_info(path, max_pixels=40_000_000):
    raw = Path(path).read_bytes()
    if len(raw) > MAX_PNG or raw[:8] != PNG_SIG:
        raise ValueError("not a PNG")
    pos, idat, hdr = 8, [], None
    while pos + 8 <= len(raw):
        n, typ = struct.unpack(">I4s", raw[pos:pos + 8])
        body = raw[pos + 8:pos + 8 + n]
        if len(body) != n:
            raise ValueError("truncated PNG")
        if typ == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif typ == b"IDAT":
            idat.append(body)
        elif typ == b"IEND":
            break
        pos += 12 + n
    if not hdr or not idat:
        raise ValueError("PNG without header or data")
    w, h, depth, ctype, _, _, interlace = hdr
    ch = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}.get(ctype)
    if ch is None or depth != 8 or interlace != 0:
        return {"width": w, "height": h, "channels": ch, "mean": None, "std": None, "distinct": None, "decoded": False}
    if w * h > max_pixels:
        raise ValueError("PNG too large to decode")
    data = zlib.decompress(b"".join(idat))
    stride = w * ch
    if len(data) != h * (stride + 1):
        raise ValueError("PNG data length mismatch")
    prev = bytearray(stride)
    total = total2 = 0
    seen = set()
    step = max(1, h // 400)
    count = 0
    for y in range(h):
        f = data[y * (stride + 1)]
        line = bytearray(data[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        if f == 1:
            for i in range(ch, stride):
                line[i] = (line[i] + line[i - ch]) & 255
        elif f == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                left = line[i - ch] if i >= ch else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(stride):
                left = line[i - ch] if i >= ch else 0
                ul = prev[i - ch] if i >= ch else 0
                line[i] = (line[i] + _paeth(left, prev[i], ul)) & 255
        elif f != 0:
            raise ValueError("bad PNG filter")
        prev = line
        if y % step == 0:
            sample = line[::ch] if ch in (1, 3, 4) else line[::2]
            for v in sample:
                total += v
                total2 += v * v
            count += len(sample)
            seen.update(sample[:: max(1, len(sample) // 256)])
    mean = total / count
    var = max(0.0, total2 / count - mean * mean)
    return {"width": w, "height": h, "channels": ch, "mean": round(mean, 2), "std": round(var ** 0.5, 2), "distinct": len(seen), "decoded": True}


def jpeg_size(path):
    raw = Path(path).read_bytes()
    if raw[:2] != b"\xff\xd8":
        raise ValueError("not a JPEG")
    i = 2
    while i + 4 <= len(raw):
        if raw[i] != 0xFF:
            i += 1
            continue
        m = raw[i + 1]
        if m in (0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF):
            h, w = struct.unpack(">HH", raw[i + 5:i + 9])
            return w, h
        if m in (0xD8, 0x01) or 0xD0 <= m <= 0xD7:
            i += 2
            continue
        i += 2 + struct.unpack(">H", raw[i + 2:i + 4])[0]
    raise ValueError("JPEG without frame header")


def tiff_info(path):
    """(width, height) of the first image in a classic TIFF, plus the compressibility of its first megabyte (flat images compress far)."""
    raw = Path(path).read_bytes()
    if raw[:4] not in (b"II*\x00", b"MM\x00*"):
        raise ValueError("not a TIFF")
    e = "<" if raw[:2] == b"II" else ">"
    ifd = struct.unpack(e + "I", raw[4:8])[0]
    n = struct.unpack(e + "H", raw[ifd:ifd + 2])[0]
    w = h = None
    for i in range(n):
        tag, typ, cnt = struct.unpack(e + "HHI", raw[ifd + 2 + 12 * i:ifd + 10 + 12 * i])
        val = raw[ifd + 10 + 12 * i:ifd + 14 + 12 * i]
        v = struct.unpack(e + ("H" if typ == 3 else "I"), val[:2] if typ == 3 else val)[0]
        if tag == 256:
            w = v
        elif tag == 257:
            h = v
    if not w or not h:
        raise ValueError("TIFF without dimensions")
    head = raw[:1 << 20]
    return {"width": w, "height": h, "bytes": len(raw), "ratio": round(len(zlib.compress(head)) / max(1, len(head)), 3)}


def psd_info(path):
    with open(path, "rb") as f:
        head = f.read(26)
    if len(head) < 26 or head[:4] != b"8BPS":
        raise ValueError("not a PSD")
    ver, _res, ch, h, w, depth, _mode = struct.unpack(">H6sHIIHH", head[4:26])
    return {"version": ver, "channels": ch, "height": h, "width": w, "depth": depth}


def pdf_info(path):
    raw = Path(path).read_bytes()
    if not raw.startswith(b"%PDF-") or b"%%EOF" not in raw[-2048:]:
        raise ValueError("not a complete PDF")
    import re
    pages = len(re.findall(rb"/Type\s*/Page(?![s\w])", raw))
    counts = [int(x) for x in re.findall(rb"/Type\s*/Pages[^>]*?/Count\s+(\d+)", raw)] + [int(x) for x in re.findall(rb"/Count\s+(\d+)[^>]*?/Type\s*/Pages", raw)]
    if counts:
        pages = max(pages, max(counts)) if pages == 0 else pages
    return {"pages": pages, "version": raw[5:8].decode("ascii", "replace"), "bytes": len(raw)}


def _boxes(buf, start, end):
    pos = start
    while pos + 8 <= end:
        size, typ = struct.unpack(">I4s", buf[pos:pos + 8])
        hdr = 8
        if size == 1:
            size = struct.unpack(">Q", buf[pos + 8:pos + 16])[0]
            hdr = 16
        elif size == 0:
            size = end - pos
        if size < hdr or pos + size > end:
            return
        yield typ, pos + hdr, pos + size
        pos += size


def mp4_info(path, max_bytes=512 * 1024 * 1024):
    p = Path(path)
    if p.stat().st_size > max_bytes:
        raise ValueError("too large")
    buf = p.read_bytes()
    info = {"duration_s": None, "width": None, "height": None, "codec": None, "brand": buf[8:12].decode("ascii", "replace") if len(buf) > 12 else ""}
    moov = [(a, b) for t, a, b in _boxes(buf, 0, len(buf)) if t == b"moov"]
    if not moov:
        raise ValueError("no moov box")
    a, b = moov[0]
    for t, s, e in _boxes(buf, a, b):
        if t == b"mvhd":
            ver = buf[s]
            if ver == 1:
                ts, dur = struct.unpack(">IQ", buf[s + 20:s + 32])
            else:
                ts, dur = struct.unpack(">II", buf[s + 12:s + 20])
            info["duration_s"] = round(dur / ts, 3) if ts else None
        elif t == b"trak":
            for t2, s2, e2 in _boxes(buf, s, e):
                if t2 == b"tkhd":
                    ver = buf[s2]
                    off = s2 + (88 if ver == 1 else 76)
                    w, h = struct.unpack(">II", buf[off:off + 8])
                    if w >> 16 and h >> 16 and info["width"] is None:
                        info["width"], info["height"] = w >> 16, h >> 16
                elif t2 == b"mdia":
                    for t3, s3, e3 in _boxes(buf, s2, e2):
                        if t3 == b"minf":
                            for t4, s4, e4 in _boxes(buf, s3, e3):
                                if t4 == b"stbl":
                                    for t5, s5, e5 in _boxes(buf, s4, e4):
                                        if t5 == b"stsd" and info["codec"] is None:
                                            fmt = buf[s5 + 12:s5 + 16].decode("ascii", "replace")
                                            if fmt in ("avc1", "avc3", "hvc1", "hev1", "av01", "vp09", "mp4v", "apch", "apcn", "apco", "apcs", "ap4h", "ap4x"):
                                                info["codec"] = fmt
    if info["codec"] is None:
        raise ValueError("no video sample entry")
    return info
