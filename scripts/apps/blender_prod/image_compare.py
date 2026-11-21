"""Compare uncompressed RGB8 TGA renders without external dependencies."""

from contextlib import ExitStack
import itertools
import math
import struct


def raster(stream):
    header = stream.read(18)
    if len(header) != 18:
        raise ValueError("short TGA header")
    label, colormap, kind, _, _, _, _, _, width, height, depth, flags = struct.unpack(
        "<BBBHHBHHHHBB", header)
    if (colormap, kind, depth) != (0, 2, 24) or flags & ~0x30 or not width or not height:
        raise ValueError("expected uncompressed RGB8 TGA")
    return width, height, 18 + label, flags


def validate_image(path):
    with path.open("rb") as stream:
        width, height, offset, _ = raster(stream)
        if stream.seek(0, 2) < offset + width * height * 3:
            raise ValueError("truncated TGA pixels")


def max_abs(left, right):
    """Maximum absolute RGB channel error in [0, 1], independent of TGA origin."""
    with ExitStack() as stack:
        streams = [stack.enter_context(path.open("rb")) for path in (left, right)]
        headers = [raster(stream) for stream in streams]
        width, height = headers[0][:2]
        if headers[1][:2] != (width, height):
            raise ValueError("image dimensions differ")
        peak = 0
        for y in range(height):
            rows = []
            for stream, (_, _, offset, flags) in zip(streams, headers):
                row = y if flags & 0x20 else height - 1 - y
                stream.seek(offset + row * width * 3)
                data = stream.read(width * 3)
                if len(data) != width * 3:
                    raise ValueError("truncated TGA pixels")
                if flags & 0x10:
                    data = b"".join(data[x:x + 3] for x in range(len(data) - 3, -1, -3))
                rows.append(data)
            if rows[0] != rows[1]:
                peak = max(peak, max(abs(a - b) for a, b in zip(*rows)))
        return peak / 255


def compare_images(hosts, guest, margin):
    """Use every host pair for the noise floor and every host as a guest reference."""
    if len(hosts) < 2 or not math.isfinite(margin) or not 0 <= margin <= 1:
        raise ValueError("two host runs and a finite margin between zero and one required")
    noise = max(max_abs(a, b) for a, b in itertools.combinations(hosts, 2))
    distance = max(max_abs(host, guest) for host in hosts)
    limit = min(1, noise + margin)
    return dict(match=distance <= limit, distance=distance, noise=noise, limit=limit)
