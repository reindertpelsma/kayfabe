#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Replace board identifiers with keyed synthetic UUIDs, including archive members.

STATUS: LIVE, 2026-11-08. The key and mapping must never be committed. Mapping is
in memory only: HMAC-SHA256(lower-case UUID), truncated into the existing reserved
synthetic namespace. Collisions refuse the operation before any write. The caller
must supply a private random key file OUTSIDE the repository (at least 32 bytes).

`inventory --root ROOT` prints counts/paths only; `scrub --root ROOT --key-file KEY`
rewrites tracked files and checksum manifests. No history, external archive or
network mutation. gzip headers, tar/zip order, member names and mtimes are retained
where possible. Untouched containers are byte-identical. Container compression may
change for rewritten containers. Every input is validated by the bounded CI scanner
before writes; all transformed files are staged in memory and rescanned first.
Known eight-hex GPU aliases expand to the same full synthetic identity. Ambiguous
prefixes refuse before writes; unknown aliases are unchanged, never guessed.
"""

from __future__ import annotations

import argparse
import base64
import copy
import gzip
import hashlib
import hmac
import io
import lzma
import re
import struct
import subprocess
import sys
import tarfile
import zipfile
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ci"))
import no_gpu_uuids as gate


class ScrubError(ValueError):
    pass


def pseudonym(key, original):
    suffix = hmac.new(key, original.lower(), hashlib.sha256).hexdigest()[:4]
    return gate.SYNTHETIC_PREFIX.encode() + suffix.encode()


class Pseudonymizer:
    def __init__(self, key, originals):
        if not key:
            raise ScrubError("empty key")
        self.values = {u.lower(): pseudonym(key, u.lower()) for u in originals}
        self.logical_hashes = {}
        self.logical_sizes = {}
        if len(set(self.values.values())) != len(self.values):
            raise ScrubError("synthetic UUID collision; no writes made")

    def uuid(self, original):
        return self.values[original.lower()]


BARE = re.compile(gate._UUID, re.I)
SHORT_ALIAS = re.compile(rb"gpu-([0-9a-f]{8})(?![0-9a-f-])", re.I)


def replace_known(data, mapper):
    # Replace canonical UUID text, including decoded text encodings. Unmarked
    # 16-byte blobs and 32-digit hex numbers may be ordinary record counters;
    # their equality to a UUID value supplies no authority to rewrite them.
    def marked(match):
        original = gate.canonical_uuid(match.group(1))
        replacement = mapper.values.get(original)
        if replacement is None:
            return match.group()
        token = match.group(1)
        if b"-" not in token:
            replacement = replacement.replace(b"-", b"")
        digits = iter(replacement)
        rendered = bytes(byte if byte in b" \t\r\n" else next(digits) for byte in token)
        start, end = match.span(1)
        return (
            match.group()[: start - match.start()]
            + rendered
            + match.group()[end - match.start() :]
        )

    data = gate.PATTERN.sub(marked, data)
    data = BARE.sub(lambda m: mapper.values.get(m.group().lower(), m.group()), data)

    def alias(match):
        prefix = match.group(1).lower()
        candidates = {
            new for old, new in mapper.values.items() if old.startswith(prefix)
        }
        if len(candidates) > 1:
            raise ScrubError("ambiguous short GPU alias; no writes made")
        if candidates:
            return b"GPU-" + candidates.pop()
        return match.group()

    data = SHORT_ALIAS.sub(alias, data)
    return data


def preserve_encoding_layout(source, encoded):
    if len(source.translate(None, b" \t\r\n")) != len(encoded):
        # Expanding a short alias to its full identity changes the token width.
        return encoded
    digits = iter(encoded)
    return bytes(byte if byte in b" \t\r\n" else next(digits) for byte in source)


def rewrite_text(data, mapper):
    edits = []
    patched, written = None, None
    for (start, end), encoding, decoded in gate.encoded_views(data):
        new = replace_known(decoded, mapper)
        if new == decoded:
            continue
        source = data[start:end]
        changed_width = len(new) != len(decoded)
        if encoding == "hex":
            # Preserve whitespace and hex case when the token length survives.
            upper = any(65 <= x <= 70 for x in source) and not any(
                97 <= x <= 102 for x in source
            )
            encoded = new.hex().encode()
            new = preserve_encoding_layout(
                source, encoded.upper() if upper else encoded
            )
        elif encoding == "base64":
            original = source.translate(None, b" \t\r\n").rstrip(b"=")
            if base64.b64encode(decoded).rstrip(b"=") != original:
                raise ScrubError("noncanonical base64 framing; no writes made")
            new = base64.b64encode(new)
            if not changed_width and not source.rstrip().endswith(b"="):
                new = new.rstrip(b"=")
            new = preserve_encoding_layout(source, new)
        else:
            new = new.decode().encode(encoding)
        if len(new) == end - start:
            if patched is None:
                patched, written = bytearray(data), bytearray(len(data))
            # Alignment views overlap. Apply only actual byte changes, never
            # an unchanged prefix/suffix that could restore another identity.
            for offset, (old_byte, new_byte) in enumerate(zip(source, new), start):
                if old_byte == new_byte:
                    continue
                if written[offset] and patched[offset] != new_byte:
                    raise ScrubError("conflicting encoded views; no writes made")
                patched[offset], written[offset] = new_byte, 1
        else:
            # Width-changing alias edits cannot silently override another view.
            prefix = 0
            while prefix < min(len(source), len(new)) and source[prefix] == new[prefix]:
                prefix += 1
            suffix = 0
            while (
                suffix < min(len(source), len(new)) - prefix
                and source[-suffix - 1] == new[-suffix - 1]
            ):
                suffix += 1
            edits.append(
                (start + prefix, end - suffix, new[prefix : len(new) - suffix])
            )
    if patched is not None:
        data = bytes(patched)
    previous = len(data)
    for start, end, new in sorted(edits, reverse=True):
        if end > previous or (written is not None and any(written[start:end])):
            raise ScrubError("overlapping variable-width views; no writes made")
        data = data[:start] + new + data[end:]
        previous = start
    return replace_known(data, mapper)


def unpack_stream(data, kind):
    opener = {
        "gzip": lambda: gzip.GzipFile(fileobj=io.BytesIO(data)),
        "xz": lambda: gate.xz_reader(data),
        "zstd": lambda: gate.zstd_reader(data),
    }[kind]
    with opener() as stream:
        return gate.Budget().read(stream)


def discover(data):
    """Collect identities in memory during the same complete, bounded scan.

    The gate's public CLI only emits fingerprints/counts. This private collector
    is for rewriting; its contents are never logged or serialized.
    """
    identities = set()
    gate.scan(data, _uuids=identities)
    return identities


def repair_manifest(data, changes, _tables=None):
    """Refresh exact MD5/SHA256 references and adjacent TSV byte counts.

    Hash substitutions are simultaneous, from original content to final content;
    newly written hashes can never be remapped during the same pass.
    """
    hashes, sizes = _tables if _tables is not None else manifest_tables(changes)
    lines = []
    for line in data.splitlines(keepends=True):
        fields = line.rstrip(b"\r\n").split(b"\t")
        if len(fields) >= 3 and fields[-2].isdigit() and fields[-1].lower() in sizes:
            fields[-2] = sizes[fields[-1].lower()]
            ending = line[len(line.rstrip(b"\r\n")) :]
            line = b"\t".join(fields) + ending
        lines.append(line)
    data = b"".join(lines)
    if hashes:
        data = re.sub(
            rb"(?<![0-9a-f])(?:[0-9a-f]{64}|[0-9a-f]{32})(?![0-9a-f])",
            lambda m: hashes.get(m.group().lower(), m.group()),
            data,
            flags=re.I,
        )
    return data


def manifest_tables(changes):
    hashes, sizes = {}, {}
    for _name, (before, after) in changes.items():
        if before == after:
            continue
        for algorithm in (hashlib.md5, hashlib.sha256):
            old, new = (
                algorithm(before).hexdigest().encode(),
                algorithm(after).hexdigest().encode(),
            )
            hashes[old] = new
            sizes[old] = str(len(after)).encode()
    return hashes, sizes


def repair_entries(originals, rewritten, extra_tables=None):
    # Child checksums may themselves be covered by a parent manifest. Recompute
    # from the original text on every iteration, until that dependency DAG settles.
    current = dict(rewritten)
    for _ in range(len(originals) + 1):
        changes = {
            n: (originals[n], current[n])
            for n in originals
            if originals[n] != current[n]
        }
        tables = manifest_tables(changes)
        if extra_tables is not None:
            tables = (extra_tables[0] | tables[0], extra_tables[1] | tables[1])
        new = {n: repair_manifest(rewritten[n], changes, tables) for n in rewritten}
        if new == current:
            return current
        current = new
    raise ScrubError("checksum dependencies did not converge; no writes made")


def gzip_members(data):
    """Yield header/body/original frame, preserving concatenated gzip ordering."""
    offset = 0
    while offset < len(data):
        # RFC1952 permits zero padding after the last member.
        if not data[offset:].strip(b"\0"):
            return
        start = offset
        if data[offset : offset + 3] != b"\x1f\x8b\x08":
            raise ScrubError("invalid gzip member")
        flags = data[offset + 3]
        offset += 10
        if flags & 4:
            length = struct.unpack_from("<H", data, offset)[0]
            offset += 2 + length
        for bit in (8, 16):
            if flags & bit:
                offset = data.index(b"\0", offset) + 1
        if flags & 2:
            offset += 2
        header = data[start:offset]
        inflater = zlib.decompressobj(-15)
        body = inflater.decompress(data[offset:], gate.MAX_MEMBER_BYTES + 1)
        if len(body) > gate.MAX_MEMBER_BYTES or not inflater.eof:
            raise ScrubError("gzip expansion limit or truncated member")
        consumed = len(data[offset:]) - len(inflater.unused_data)
        offset += consumed + 8
        yield header, body, data[start:offset]


def rewrite(data, mapper):
    result = _rewrite(data, mapper)
    # A checksum can name a decompressed stream that has no loose counterpart
    # in the checkout (for example the archived C-reference MD5SUMS).
    hashes, sizes = manifest_tables({0: (data, result)})
    mapper.logical_hashes.update(hashes)
    mapper.logical_sizes.update(sizes)
    return result


def _rewrite(data, mapper):
    kind = gate.archive_kind(data)
    if kind == "gzip":
        out = []
        consumed = 0
        for header, body, raw in gzip_members(data):
            new = rewrite(body, mapper)
            new_header = rewrite_text(header, mapper)
            if new == body and new_header == header:
                out.append(raw)
            else:
                if header[3] & 2:
                    new_header = new_header[:-2] + struct.pack(
                        "<H", zlib.crc32(new_header[:-2]) & 0xFFFF
                    )
                level = {2: 9, 4: 1}.get(header[8], 6)
                compressor = zlib.compressobj(level, zlib.DEFLATED, -15)
                out.append(
                    new_header
                    + compressor.compress(new)
                    + compressor.flush()
                    + struct.pack("<II", zlib.crc32(new), len(new) & 0xFFFFFFFF)
                )
            consumed += len(raw)
        return b"".join(out) + data[consumed:]
    if kind in ("xz", "zstd"):
        body = unpack_stream(data, kind)
        new = rewrite(body, mapper)
        if body == new:
            return data
        if kind == "xz":
            check = lzma.CHECK_CRC64 if len(data) < 8 else data[7] & 0x0F
            return lzma.compress(new, check=check)
        try:
            from compression import zstd
        except ImportError:
            return subprocess.run(
                ["zstd", "-q", "-c", "-19", "-T1"],
                input=new,
                capture_output=True,
                check=True,
            ).stdout
        return zstd.compress(new)
    if kind == "tar":
        infos, originals = [], {}
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
            for i, member in enumerate(archive):
                infos.append(copy.copy(member))
                infos[-1].pax_headers = member.pax_headers.copy()
                if member.isfile():
                    with archive.extractfile(member) as stream:
                        originals[i] = gate.Budget().read(stream, member.size)
            padding_bytes = gate.check_tar_tail(data, archive)
        rewritten = {i: rewrite(body, mapper) for i, body in originals.items()}
        rewritten = repair_entries(originals, rewritten)
        metadata_changed = False
        for info in infos:
            for field in ("name", "linkname", "uname", "gname"):
                old = getattr(info, field)
                new = rewrite_text(
                    old.encode("utf-8", "surrogateescape"), mapper
                ).decode("utf-8", "surrogateescape")
                metadata_changed |= old != new
                setattr(info, field, new)
            for key, old in list(info.pax_headers.items()):
                new = rewrite_text(old.encode(), mapper).decode()
                metadata_changed |= old != new
                info.pax_headers[key] = new
        if originals == rewritten and not metadata_changed:
            return data
        out = io.BytesIO()
        fmt = (
            tarfile.PAX_FORMAT
            if any(m.pax_headers for m in infos)
            else tarfile.GNU_FORMAT
        )
        with tarfile.open(fileobj=out, mode="w", format=fmt) as archive:
            for i, member in enumerate(infos):
                body = rewritten.get(i)
                if body is not None:
                    member.size = len(body)
                    if "size" in member.pax_headers:
                        member.pax_headers["size"] = str(len(body))
                archive.addfile(member, io.BytesIO(body) if body is not None else None)
            logical_end = archive.offset
        return out.getvalue()[:logical_end] + b"\0" * padding_bytes
    if kind == "zip":
        out = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            infos = archive.infolist()
            originals = {i: archive.read(info) for i, info in enumerate(infos)}
            rewritten = repair_entries(
                originals, {i: rewrite(body, mapper) for i, body in originals.items()}
            )
            comment = rewrite_text(archive.comment, mapper)
            metadata_changed = comment != archive.comment
            new_infos = []
            for member in infos:
                member = copy.copy(member)
                name = rewrite_text(member.filename.encode(), mapper).decode()
                extra, note = rewrite_text(member.extra, mapper), rewrite_text(
                    member.comment, mapper
                )
                metadata_changed |= (name, extra, note) != (
                    member.filename,
                    member.extra,
                    member.comment,
                )
                member.filename, member.extra, member.comment = name, extra, note
                new_infos.append(member)
            if originals == rewritten and not metadata_changed:
                return data
            with zipfile.ZipFile(out, "w") as target:
                target.comment = comment
                for i, info in enumerate(new_infos):
                    target.writestr(info, rewritten[i])
        return out.getvalue()
    return rewrite_text(data, mapper)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("command", choices=["inventory", "scrub"])
    ap.add_argument("--root", type=Path, default=gate.ROOT)
    ap.add_argument("--key-file", type=Path)
    args = ap.parse_args(argv)
    root = args.root.resolve()
    paths = (
        subprocess.check_output(["git", "-C", str(root), "ls-files", "-z"])
        .decode()
        .split("\0")
    )
    originals, identities, hit_paths = {}, set(), []
    for index, rel in enumerate(filter(None, paths)):
        if index % 2000 == 0:
            print(
                f"uuid inventory progress: files={index}", file=sys.stderr, flush=True
            )
        path = root / rel
        if path.is_symlink() or not path.is_file():
            continue
        with path.open("rb") as stream:
            data = stream.read(gate.MAX_FILE_BYTES + 1)
        if len(data) > gate.MAX_FILE_BYTES:
            raise ScrubError("tracked file size limit")
        found = discover(data)
        if found:
            identities.update(found)
            hit_paths.append(rel)
        originals[rel] = data
    print(f"uuid inventory: files={len(hit_paths)} distinct={len(identities)}")
    for rel in hit_paths:
        print(rel)
    if args.command == "inventory":
        return 0
    if args.key_file is None or args.key_file.resolve().is_relative_to(root):
        raise ScrubError("private key must be outside repository")
    key = args.key_file.read_bytes()
    if len(key) < 32:
        raise ScrubError("private key must contain at least 32 random bytes")
    mapper = Pseudonymizer(key, identities)
    print("uuid scrub phase: rewriting", file=sys.stderr, flush=True)
    rewritten = {rel: rewrite(data, mapper) for rel, data in originals.items()}
    # Filesystem manifests and checksum cross-references follow the same fixed
    # point rule as members. The external archive index stays unchanged unless
    # its actual referenced archive changes; this command never rewrites those.
    print("uuid scrub phase: checksum repair", file=sys.stderr, flush=True)
    rewritten = repair_entries(
        originals, rewritten, (mapper.logical_hashes, mapper.logical_sizes)
    )
    print("uuid scrub phase: verify before writes", file=sys.stderr, flush=True)
    for protected in ("README.md", "docs/OWNER_RULINGS.md"):
        if rewritten.get(protected) != originals.get(protected):
            raise ScrubError("protected owner document would change; no writes made")
    for rel, data in rewritten.items():
        if gate.scan(data):
            raise ScrubError("UUID remains after rewrite; no writes made")
    changed = []
    for rel, data in rewritten.items():
        if data != originals[rel]:
            (root / rel).write_bytes(data)
            changed.append(rel)
    print(f"uuid scrub: changed={len(changed)} remaining=0")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (
        ScrubError,
        gate.ScanError,
        OSError,
        ValueError,
        tarfile.TarError,
        zipfile.BadZipFile,
    ):
        print(
            "UUID scrub refused; input invalid, incomplete, or private key invalid",
            file=sys.stderr,
        )
        sys.exit(2)
