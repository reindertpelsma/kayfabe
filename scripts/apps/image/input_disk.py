# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Transient read-only official-input disk, never a distributed image or ISO.

No filesystem or mount is needed: a bounded JSON index precedes sector-aligned,
unchanged installer bytes. The guest reads its virtio block device with Python.
Only the per-run scratch/provisioning context may retain this disk.
"""

import hashlib
import json
from pathlib import Path
import struct
from common import SHA, canonical, validate_pin, verify_file

MAGIC = b"KF-OFFICIAL-v1\0\0"
HEADER_BYTES = 1024**2
MAX_TOTAL = 16 * 1024**3
DEVICE = "/dev/disk/by-id/virtio-KF-OFFICIAL"


def write(path, pins, sources):
    path = Path(path)
    if not pins or len(pins) > 256:
        raise ValueError("official disk needs 1..256 pinned inputs")
    entries = {}
    offset = HEADER_BYTES
    for pin in pins:
        validate_pin(pin)
        if pin["sha256"] in entries:
            if entries[pin["sha256"]]["size"] != pin["size"]:
                raise ValueError("conflicting official disk pin")
            continue
        entries[pin["sha256"]] = {"offset": offset, "size": pin["size"]}
        offset += pin["size"] + (-pin["size"] % 512)
    if offset > HEADER_BYTES + MAX_TOTAL:
        raise ValueError("official disk byte budget exceeded")
    index = canonical(entries)
    if len(index) > HEADER_BYTES - 24:
        raise ValueError("official disk index budget exceeded")
    created = False
    try:
        with path.open("xb") as output:
            created = True
            output.write(MAGIC + struct.pack("<Q", len(index)) + index)
            for sha, entry in entries.items():
                output.seek(entry["offset"])
                h = hashlib.sha256()
                count = 0
                with Path(sources[sha]).open("rb") as source:
                    while block := source.read(1024**2):
                        count += len(block)
                        if count > entry["size"]:
                            raise ValueError("official disk input size mismatch")
                        h.update(block)
                        output.write(block)
                if count != entry["size"] or h.hexdigest() != sha:
                    raise ValueError("official disk input digest mismatch")
                output.write(b"\0" * (-count % 512))
        return path
    except BaseException:
        if created:
            path.unlink(missing_ok=True)
        raise


def read(device, pin, cache):
    validate_pin(pin)
    cache = Path(cache)
    cache.mkdir(parents=True, exist_ok=True)
    target = cache / pin["sha256"]
    if target.exists():
        verify_file(target, pin)
        return target
    part = target.with_suffix(".part")
    try:
        with open(device, "rb", buffering=0) as source:
            total = source.seek(0, 2)
            source.seek(0)
            if source.read(16) != MAGIC:
                raise ValueError("invalid official disk magic")
            raw_length = source.read(8)
            if len(raw_length) != 8:
                raise ValueError("truncated official disk header")
            length = struct.unpack("<Q", raw_length)[0]
            if not 1 <= length <= HEADER_BYTES - 24 or total > HEADER_BYTES + MAX_TOTAL:
                raise ValueError("official disk size/index budget exceeded")
            entries = json.loads(source.read(length))
            if not isinstance(entries, dict) or not 1 <= len(entries) <= 256:
                raise ValueError("invalid official disk index")
            ranges = []
            for sha, entry in entries.items():
                if not SHA.fullmatch(sha) or set(entry) != {"offset", "size"}:
                    raise ValueError("invalid official disk entry")
                offset, size = entry["offset"], entry["size"]
                if (
                    type(offset) is not int
                    or type(size) is not int
                    or size <= 0
                    or offset < HEADER_BYTES
                    or offset % 512
                    or offset + size > total
                ):
                    raise ValueError("official disk entry outside device")
                ranges.append((offset, offset + size))
            ranges.sort()
            if any(a[1] > b[0] for a, b in zip(ranges, ranges[1:])):
                raise ValueError("overlapping official disk entries")
            if pin["sha256"] not in entries:
                raise ValueError("official pin missing from disk")
            entry = entries[pin["sha256"]]
            if entry["size"] != pin["size"]:
                raise ValueError("official disk declared size differs from pin")
            source.seek(entry["offset"])
            remaining = pin["size"]
            with part.open("wb") as output:
                while remaining:
                    block = source.read(min(1024**2, remaining))
                    if not block:
                        raise ValueError("truncated official disk payload")
                    remaining -= len(block)
                    output.write(block)
        verify_file(part, pin)
        part.replace(target)
        return target
    except (KeyError, TypeError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid official disk metadata") from error
    finally:
        part.unlink(missing_ok=True)
