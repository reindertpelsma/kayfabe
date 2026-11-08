# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Read bounded USTAR app exports. Never extract guest paths on the host."""

import gzip
import hashlib
from pathlib import Path, PurePosixPath
import posixpath
import tempfile
from common import relative_path
import elf

MAX_EXPORT = 2 * 1024**3


def validate_export(path, recipe, inventory):
    files = {}
    links = {}
    total = 0
    shared = {
        p: libs
        for b in recipe["builds"]
        if b["requires"] == "open-layer"
        for p, libs in b.get("shared_runtime_outputs", {}).items()
    }

    def number(value):
        text = value.rstrip(b"\0 ").lstrip(b" ")
        if text and any(c not in b"01234567" for c in text):
            raise ValueError("non-USTAR numeric field")
        return int(text or b"0", 8)

    with gzip.open(path, "rb") as stream:

        def read(size):
            data = stream.read(size)
            if len(data) != size:
                raise ValueError("truncated app export")
            return data

        while True:
            header = read(512)
            if header == b"\0" * 512:
                if read(512) != b"\0" * 512:
                    raise ValueError("invalid export terminator")
                trailing = stream.read(10240)
                if any(trailing) or stream.read(1):
                    raise ValueError("export trailing payload")
                break
            checksum = number(header[148:156])
            if (
                checksum != sum(header[:148]) + 8 * 32 + sum(header[156:])
                or header[257:263] != b"ustar\0"
            ):
                raise ValueError("invalid USTAR header")
            name = header[:100].split(b"\0", 1)[0].decode("utf-8")
            prefix = header[345:500].split(b"\0", 1)[0].decode("utf-8")
            if prefix:
                name = prefix + "/" + name
            relative_path(name)
            if not name.startswith("opt/apps/bundle/") or name.startswith(
                ("opt/apps/bundle/lib/", "opt/apps/bundle/cuda/")
            ):
                raise ValueError("export outside approved app bundle")
            if name in files or name in links or len(files) + len(links) >= 8192:
                raise ValueError("duplicate/oversized export index")
            size = number(header[124:136])
            total += size
            if total > MAX_EXPORT:
                raise ValueError("export byte budget exceeded")
            typ = header[156:157]
            if typ == b"2":
                if size:
                    raise ValueError("symlink body refused")
                link = header[157:257].split(b"\0", 1)[0].decode("utf-8")
                if PurePosixPath(link).is_absolute():
                    raise ValueError("absolute export link")
                target = posixpath.normpath(str(PurePosixPath(name).parent / link))
                if not target.startswith("opt/apps/bundle/"):
                    raise ValueError("export link escapes bundle")
                links[name] = target
                continue
            if typ not in (b"0", b"\0"):
                raise ValueError("PAX, special or non-file export refused")
            h = hashlib.sha256()
            remaining = size
            with tempfile.TemporaryFile() as candidate:
                while remaining:
                    block = read(min(1024**2, remaining))
                    remaining -= len(block)
                    h.update(block)
                    if "/" + name in shared:
                        candidate.write(block)
                if "/" + name in shared:
                    candidate.flush()
                    # /proc/self/fd names refer only to the owned temporary file; no guest path is opened.
                    elf.check_shared(
                        "/proc/self/fd/" + str(candidate.fileno()), shared["/" + name]
                    )
            files[name] = h.hexdigest()
            read(-size % 512)
    for name, target in links.items():
        for _ in range(64):
            if target in files:
                files[name] = files[target]
                break
            if target not in links:
                raise ValueError("dangling export symlink")
            target = links[target]
        else:
            raise ValueError("cyclic export symlink")
    actual = {"/" + name: sha for name, sha in files.items()}
    if actual != inventory["files"]:
        raise ValueError("app export differs from builder inventory")
    for b in recipe["builds"]:
        for output in b["outputs"]:
            present = output["path"] in actual
            if b["requires"] == "open-layer" and not present:
                raise ValueError("missing open export")
            if b["requires"] == "nvidia-layer-static" and present:
                raise ValueError("static NVIDIA-layer output exported")
    return actual
