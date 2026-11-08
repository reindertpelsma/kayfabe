#!/usr/bin/env python3
"""Keep GPU board UUIDs out of the tree (a ratchet: the existing hits may only go down).

A GPU UUID (`GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, as `nvidia-smi -L` prints it, the value
of a `gpu-uuid=` / `gpu_uuid:` assignment, or the `deviceUUID = ` line of `vulkaninfo`, which on
NVIDIA is the same value) identifies one physical board. The repository is published, so a board's
identifier does not belong in it.

Synthetic placeholders are not hits: the all-zero UUID, and every UUID that starts with
`fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e` ("fa4e" for "fake"). Use those in tests, comments and scrubbed
traces; give each distinct board a different last group so equalities between boards survive.

How the ratchet works. `gpu-uuids-baseline.json` records, per file, a fingerprint of each UUID
found there and how many times (the fingerprint is a truncated SHA-256 of the lower-cased UUID;
the UUIDs themselves are never written to the baseline, to a log or to a message).

* a UUID, or an extra copy of one, that is not in the baseline for that same file fails: this is
  what a newly added or changed file with a UUID looks like;
* a baseline entry that is no longer found also fails, with the instruction to run `--shrink`,
  so the baseline can only get smaller (the count may only go down);
* `--shrink` rewrites the baseline to what is still found and never adds an entry;
* `--init` writes the first baseline, and refuses to overwrite one.

Archive members are inspected by magic (gzip, xz, zstd, tar, zip), without extraction, with
shared depth/member/expanded-byte limits. Text, ASCII hex/base64 and ASCII UTF-16
representations are checked. Corrupt/over-limit archives fail closed without
changing the baseline. See the constants below for the exact bounds.

All tracked regular files and symlink target texts are inspected; symlinks are
never followed and submodule gitlinks are excluded as separate repositories.
Explicit GPU fields also admit compact hex and bounded whitespace wrapping.
This gate does not classify arbitrary short aliases or physical serial numbers.

Exit codes: 0 clean; 1 a new hit or a stale baseline; 2 usage/incomplete scan.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import gzip
import io
import lzma
import tempfile
from contextlib import contextmanager
import tarfile
import zipfile
from dataclasses import dataclass
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).resolve().parent / "gpu-uuids-baseline.json"
SYNTHETIC_PREFIX = "fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e"

_UUID = rb"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
# A complete marked field has a size limit, checked before accepting a match.
# Over-limit formatting refuses rather than disappearing from the scan. Only marked
# fields admit compact numbers; unmarked counters and binary values stay opaque.
WS = rb"[ \t\r\n]*"
MAX_MARKED_BYTES = 4096


def hex_group(length):
    return rb"[0-9a-f](?:" + WS + rb"[0-9a-f]){" + str(length - 1).encode() + rb"}"


WRAPPED_UUID = (WS + b"-" + WS).join(hex_group(n) for n in (8, 4, 4, 4, 12))
COMPACT_UUID = hex_group(32)
PATTERN = re.compile(
    rb"(?:gpu-|gpu[-_]uuid[ \t]*[=:]|deviceUUID[ \t]*[=:])"
    + WS
    + rb"[\"']?("
    + WRAPPED_UUID
    + b"|"
    + COMPACT_UUID
    + rb")(?![0-9a-f])",
    re.IGNORECASE,
)
NOTE = "Existing hits, fingerprinted. Shrink-only: no_gpu_uuids.py --shrink."


def canonical_uuid(value):
    compact = re.sub(rb"[- \t\r\n]", b"", value).lower()
    return b"-".join(
        compact[a:b] for a, b in ((0, 8), (8, 12), (12, 16), (16, 20), (20, 32))
    )


def fingerprint(uuid: bytes) -> str:
    """A stable identity for one UUID that does not reveal it."""
    return hashlib.sha256(uuid.lower()).hexdigest()[:16]


def synthetic(uuid: bytes) -> bool:
    """A placeholder, not a board: all zero, or the documented fake prefix."""
    low = uuid.lower().decode()
    return set(low.replace("-", "")) <= {"0"} or low.startswith(SYNTHETIC_PREFIX)


# Bounds apply per tracked file and are shared across every nested member. Archive
# errors/limits refuse the scan, including --init/--shrink; none means "clean".
MAX_FILE_BYTES = 64 * 1024 * 1024
MAX_MEMBER_BYTES = 64 * 1024 * 1024
MAX_EXPANDED_BYTES = 256 * 1024 * 1024
MAX_MEMBERS = 10000
MAX_DEPTH = 6
HEX = re.compile(
    rb"(?<![0-9a-f])[0-9a-f][0-9a-f \t\r\n]{22,}[0-9a-f](?![0-9a-f])", re.I
)
B64 = re.compile(
    rb"(?<![A-Za-z0-9+/])[A-Za-z0-9+/]{16,}={0,2}"
    rb"|(?m:^[ \t]*(?P<short>[A-Za-z0-9+/]{2,15}={0,2})(?=[ \t]*\r?$))"
)
B64_TAIL = re.compile(
    rb"[ \t]*\r?\n[ \t]*([A-Za-z0-9+/]{2,15}={0,2})(?![A-Za-z0-9+/=])"
)


class ScanError(ValueError):
    """An incomplete scan; messages never contain input bytes/member names."""


@dataclass
class Budget:
    expanded: int = 0
    members: int = 0

    def member(self):
        self.members += 1
        if self.members > MAX_MEMBERS:
            raise ScanError("archive member count limit")

    def read(self, stream, declared=None):
        remaining = min(MAX_MEMBER_BYTES, MAX_EXPANDED_BYTES - self.expanded)
        if declared is not None and declared > remaining:
            raise ScanError("archive expansion limit")
        data = stream.read(remaining + 1)
        if len(data) > remaining:
            raise ScanError("archive expansion limit")
        if declared is not None and len(data) != declared:
            raise ScanError("truncated archive member")
        self.expanded += len(data)
        return data


def encoded_views(data, *, marked_only=False):
    """(span, encoding, decoded bytes) for ASCII hex/base64 and UTF-16 text.

    Each decoded view is bounded by the input size. Base64 considers four fixed
    character alignments, never every possible suffix or recursive decodings.
    """
    low = data.lower()
    # Compact assignments need not contain a hyphen (2d). Short marked aliases
    # are also exposed to the scrubber, which resolves them against known IDs.
    hex_candidates = HEX.finditer(data)
    for match in hex_candidates:
        try:
            decoded = bytes.fromhex(match.group().decode())
        except ValueError:
            continue
        yield match.span(), "hex", decoded
    # Join line-wrapped base64, retaining the full span for format-preserving
    # rewriting. Ordinary words separated by spaces are separate candidates.
    pending = None
    for match in B64.finditer(data):
        start, end = (
            match.span("short") if match.group("short") is not None else match.span()
        )
        token = data[start:end]
        if pending is not None:
            first, previous_end, tokens = pending
            gap = data[previous_end:start]
            if (
                b"\n" in gap
                and not gap.strip(b" \t\r\n")
                and not tokens[-1].endswith(b"=")
            ):
                tokens.append(token)
                pending = (first, end, tokens)
                continue
            yield from decode_base64(pending, data)
        pending = (start, end, [token])
    if pending is not None:
        yield from decode_base64(pending, data)
    # Decode only aligned ASCII UTF-16 runs; do not remove arbitrary NUL bytes.
    for encoding, pattern in (
        ("utf-16le", rb"(?:[\x09\x0a\x0d\x20-\x7e]\x00){36,}"),
        ("utf-16be", rb"(?:\x00[\x09\x0a\x0d\x20-\x7e]){36,}"),
    ):
        if marked_only:
            prefixes = (
                (b"g\x00p\x00u\x00", b"d\x00e\x00v\x00")
                if encoding == "utf-16le"
                else (b"\x00g\x00p\x00u", b"\x00d\x00e\x00v")
            )
            if not any(prefix in low for prefix in prefixes):
                continue
        for match in re.finditer(pattern, data):
            yield match.span(), encoding, match.group().decode(encoding).encode()


def decode_base64(candidate, data):
    """Cover ambiguous label/block boundaries in at most four alignments.

    A label made of base64 characters can prepend any number of characters.
    Its length modulo four determines the payload alignment, so a constant four
    decodes cover all starts without quadratic suffix enumeration. A trailing
    label can invalidate padding; retain the complete-quartet prefix in that
    case. Spans refer to the original bytes, including wrapping whitespace.
    """
    start, end, tokens = candidate
    token = b"".join(tokens)
    tail = B64_TAIL.match(data, end)
    if tail is not None and not token.endswith(b"="):
        token += tail.group(1)
        end = tail.end()
    for alignment in range(4):
        encoded = token[alignment:]
        if len(encoded) < 16:
            continue
        used = len(encoded)
        try:
            decoded = base64.b64decode(
                encoded + b"=" * (-len(encoded) % 4), validate=True
            )
        except binascii.Error:
            # Complete quartets preserve embedded bytes even if an unrelated
            # suffix made the final partial quartet/padding invalid.
            encoded = encoded.rstrip(b"=")
            used = len(encoded) // 4 * 4
            if used < 16:
                continue
            decoded = base64.b64decode(encoded[:used], validate=True)
        left, right = start, end
        remaining = alignment
        while remaining:
            remaining -= data[left] not in b" \t\r\n"
            left += 1
        remaining = len(token) - alignment - used
        while remaining:
            right -= 1
            remaining -= data[right] not in b" \t\r\n"
        yield (left, right), "base64", decoded


def plain_scan(data, identities=None):
    low = data.lower()
    if not any(marker in low for marker in (b"gpu-", b"gpu_uuid", b"deviceuuid")):
        return Counter()
    hits = Counter()
    for match in PATTERN.finditer(data):
        if match.end() - match.start() > MAX_MARKED_BYTES:
            raise ScanError("marked UUID formatting limit")
        uuid = canonical_uuid(match.group(1))
        if synthetic(uuid):
            continue
        hits[fingerprint(uuid)] += 1
        if identities is not None:
            identities.add(uuid.lower())
    return hits


def text_scan(data, identities=None):
    hits = plain_scan(data, identities)
    for _span, _encoding, decoded in encoded_views(data, marked_only=True):
        hits.update(plain_scan(decoded, identities))
    return hits


def archive_kind(data):
    """Magic, not suffix: renaming a bundle cannot bypass the gate."""
    if data.startswith(b"\xfd7zXZ\x00"):
        return "xz"
    if data.startswith(b"\x28\xb5\x2f\xfd") or (
        len(data) >= 4
        and 0x184D2A50 <= int.from_bytes(data[:4], "little") <= 0x184D2A5F
    ):
        return "zstd"
    if data.startswith(b"\x1f\x8b"):
        return "gzip"
    if data.startswith((b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08")):
        return "zip"
    if data[257:262] == b"ustar":
        return "tar"
    # Old V7 tar has no ustar magic; a valid first header still identifies it.
    if len(data) >= 512 and data[:512].strip(b"\0"):
        try:
            tarfile.TarInfo.frombuf(data[:512], "utf-8", "surrogateescape")
            return "tar"
        except tarfile.HeaderError:
            pass
    # An empty TAR has no identifying header. A recognized archive after whole
    # zero blocks is an ambiguous empty-TAR carrier, not an excuse to skip it.
    # Opaque zero-prefixed binary captures are not identified as TAR by zeros
    # alone. The suffix below begins with a nonzero block, so this recurses once.
    offset = 0
    while len(data) - offset >= 512 and not any(
        memoryview(data)[offset : offset + 512]
    ):
        offset += 512
    if offset and offset < len(data) and archive_kind(data[offset:]) is not None:
        return "tar"
    # ZIP permits a prepended executable/stub, but an explicit TAR header owns
    # the framing before a ZIP footer. Otherwise a polyglot can hide TAR members.
    if b"PK\x05\x06" in data[-65557:] and zipfile.is_zipfile(io.BytesIO(data)):
        return "zip"
    return None


def check_tar_tail(data, archive):
    """After iteration, validate every byte beyond the reader's consumed extent.

    TarFile.offset follows header/payload framing, including extended headers;
    searching for zero blocks would mistake member payload bytes for EOF.
    Invalid headers and nonzero post-EOF streams must never become clean scans.
    Return the accepted zero-padding length for lossless rewrite bookkeeping.
    """
    offset = archive.offset
    if offset < 0 or offset > len(data) or any(memoryview(data)[offset:]):
        raise ScanError("nonzero or incomplete tar tail")
    return len(data) - offset


@contextmanager
def xz_reader(data):
    """Bound output and decoder dictionary memory, including concatenated XZ."""
    remaining, output = data, bytearray()
    while remaining.strip(b"\0"):
        decoder = lzma.LZMADecompressor(memlimit=128 * 1024 * 1024)
        output.extend(
            decoder.decompress(remaining, max_length=MAX_MEMBER_BYTES - len(output) + 1)
        )
        if len(output) > MAX_MEMBER_BYTES:
            raise ScanError("xz expansion limit")
        if not decoder.eof:
            raise ScanError("truncated xz stream")
        remaining = decoder.unused_data
        # XZ stream padding is a multiple of four zero bytes.
        while remaining.startswith(b"\0\0\0\0"):
            remaining = remaining[4:]
    yield io.BytesIO(output)


@contextmanager
def zstd_reader(data):
    """Bounded caller read; Python 3.14 stdlib, or the zstd CLI on older Python."""
    try:
        from compression import zstd
    except ImportError:
        with tempfile.TemporaryFile() as source:
            source.write(data)
            source.seek(0)
            with subprocess.Popen(
                ["zstd", "-q", "-dc", "--memory=128MB"],
                stdin=source,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            ) as proc:
                try:
                    yield proc.stdout
                    if proc.wait() != 0:
                        raise ScanError("invalid zstd stream")
                finally:
                    if proc.poll() is None:
                        proc.kill()
                    proc.stdout.close()
                    proc.wait()
    else:
        try:
            with zstd.ZstdFile(
                io.BytesIO(data),
                options={zstd.DecompressionParameter.window_log_max: 27},
            ) as stream:
                yield stream
        except zstd.ZstdError:
            raise ScanError("invalid or over-limit zstd stream") from None


def scan(data: bytes, *, _depth=0, _budget=None, _uuids=None) -> Counter:
    """Fingerprint -> occurrences, including bounded gzip/xz/zstd/tar/zip members.

    No archive paths are extracted. Retain the larger of raw-container and
    logical-member counts, so the old raw-byte coverage (including trailing
    bytes) cannot regress and payloads are not added twice.
    """
    budget = _budget if _budget is not None else Budget()
    kind = archive_kind(data)
    if kind is None:
        return text_scan(data, _uuids)
    if _depth >= MAX_DEPTH:
        raise ScanError("archive recursion limit")
    hits = Counter()

    def child(body):
        hits.update(scan(body, _depth=_depth + 1, _budget=budget, _uuids=_uuids))

    try:
        if kind in ("gzip", "xz", "zstd"):
            budget.member()
            # Header names/comments may carry identifiers too. Scanning the raw
            # gzip bytes is cheap and preserves the raw-byte ratchet's coverage.
            hits.update(text_scan(data, _uuids))
            opener = {
                "gzip": lambda: gzip.GzipFile(fileobj=io.BytesIO(data)),
                "xz": lambda: xz_reader(data),
                "zstd": lambda: zstd_reader(data),
            }[kind]
            with opener() as stream:
                child(budget.read(stream))
        elif kind == "tar":
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
                for member in archive:
                    budget.member()
                    hits.update(
                        text_scan(
                            member.name.encode("utf-8", "surrogateescape"), _uuids
                        )
                    )
                    hits.update(
                        text_scan(
                            member.linkname.encode("utf-8", "surrogateescape"), _uuids
                        )
                    )
                    for key, value in member.pax_headers.items():
                        if key not in ("path", "linkpath"):
                            hits.update(
                                text_scan(
                                    value.encode("utf-8", "surrogateescape"), _uuids
                                )
                            )
                    if member.isfile():
                        with archive.extractfile(member) as stream:
                            child(budget.read(stream, member.size))
                check_tar_tail(data, archive)
        elif kind == "zip":
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                hits.update(text_scan(archive.comment, _uuids))
                for member in archive.infolist():
                    budget.member()
                    hits.update(
                        text_scan(
                            member.filename.encode("utf-8", "surrogateescape"), _uuids
                        )
                    )
                    hits.update(text_scan(member.comment + member.extra, _uuids))
                    with archive.open(member) as stream:
                        child(budget.read(stream, member.file_size))
    except (
        OSError,
        EOFError,
        tarfile.TarError,
        zipfile.BadZipFile,
        RuntimeError,
        NotImplementedError,
        ValueError,
        lzma.LZMAError,
    ) as exc:
        if isinstance(exc, ScanError):
            raise
        raise ScanError("invalid or unsupported archive") from None
    if kind in ("tar", "zip"):
        hits |= text_scan(data, _uuids)
    return hits


def tracked(root: Path) -> list[str]:
    """All tracked regular files and symlink texts; gitlinks are separate repos."""
    if (root / ".git").exists():
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--stage"],
            check=True,
            capture_output=True,
        ).stdout
        found = []
        for entry in out.split(b"\0"):
            if not entry:
                continue
            header, path = entry.split(b"\t", 1)
            mode, _oid, stage = header.split()
            if stage != b"0":
                raise ScanError("unmerged tracked file")
            if mode == b"160000":
                continue
            if mode not in (b"100644", b"100755", b"120000"):
                raise ScanError("unsupported tracked file mode")
            found.append(os.fsdecode(path))
        return sorted(found)
    found = []
    for base, dirs, names in os.walk(root, followlinks=False):
        dirs[:] = [name for name in dirs if name != ".git"]
        links = [name for name in dirs if (Path(base) / name).is_symlink()]
        found += [str((Path(base) / n).relative_to(root)) for n in names + links]
    return sorted(found)


def read_tracked(root, rel):
    """Read without following leaf or ancestor symlinks, including rename races."""
    parts = Path(rel).parts
    if not parts or Path(rel).is_absolute() or any(p in ("..", ".") for p in parts):
        raise ScanError("invalid tracked path")
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in parts[:-1]:
            child = os.open(
                part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory
            )
            os.close(directory)
            directory = child
        info = os.stat(parts[-1], dir_fd=directory, follow_symlinks=False)
        if stat.S_ISLNK(info.st_mode):
            return os.fsencode(os.readlink(parts[-1], dir_fd=directory))
        if stat.S_ISDIR(info.st_mode):
            return None
        if not stat.S_ISREG(info.st_mode):
            raise ScanError("nonregular tracked file")
        fd = os.open(
            parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory
        )
        with os.fdopen(fd, "rb") as stream:
            if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                raise ScanError("nonregular tracked file")
            data = stream.read(MAX_FILE_BYTES + 1)
        if len(data) > MAX_FILE_BYTES:
            raise ScanError("tracked file size limit")
        return data
    finally:
        os.close(directory)


def scan_tree(root: Path, rels: list[str]) -> dict[str, Counter]:
    """Relative path -> fingerprints found there (files without a hit are left out)."""
    found = {}
    for rel in rels:
        data = read_tracked(root, rel)
        if data is None:
            continue
        hits = scan(data)
        if hits:
            found[rel] = hits
    return found


def compare(
    actual: dict[str, Counter], baseline: dict[str, Counter]
) -> tuple[list[tuple[str, int]], list[tuple[str, int]]]:
    """(new, stale): per file, how many occurrences exceed the baseline / are gone from it."""
    new, stale = [], []
    for rel in sorted(actual.keys() | baseline.keys()):
        have, allowed = actual.get(rel, Counter()), baseline.get(rel, Counter())
        extra = sum((have - allowed).values())
        gone = sum((allowed - have).values())
        if extra:
            new.append((rel, extra))
        if gone:
            stale.append((rel, gone))
    return new, stale


def load_baseline(path: Path) -> dict[str, Counter]:
    data = json.loads(path.read_text())
    return {rel: Counter(fps) for rel, fps in data["files"].items()}


def dump_baseline(path: Path, files: dict[str, Counter]) -> None:
    body = {
        "note": NOTE,
        "files": {rel: dict(sorted(files[rel].items())) for rel in sorted(files)},
    }
    path.write_text(json.dumps(body, indent=1, sort_keys=False) + "\n")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--root", type=Path, default=ROOT)
    ap.add_argument("--baseline", type=Path, default=BASELINE)
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--init", action="store_true", help="write the first baseline")
    mode.add_argument(
        "--shrink", action="store_true", help="drop baseline entries that are gone"
    )
    args = ap.parse_args(argv)

    try:
        actual = scan_tree(args.root, tracked(args.root))
    except (ScanError, OSError):
        print(
            "gpu-uuids: incomplete scan (archive invalid, unreadable or over limits)",
            file=sys.stderr,
        )
        return 2
    if args.init:
        if args.baseline.exists():
            print(
                f"{args.baseline} exists; the baseline is never rewritten upwards",
                file=sys.stderr,
            )
            return 2
        dump_baseline(args.baseline, actual)
        print(f"baseline written: {len(actual)} file(s)")
        return 0
    if not args.baseline.exists():
        print(
            f"no baseline at {args.baseline} (a gate that cannot compare must not pass)"
        )
        return 2

    baseline = load_baseline(args.baseline)
    new, stale = compare(actual, baseline)
    if args.shrink:
        kept = {
            rel: have & baseline[rel] for rel, have in actual.items() if rel in baseline
        }
        dump_baseline(args.baseline, {rel: c for rel, c in kept.items() if c})
        stale = []
    for rel, n in new:
        print(f"NEW   {rel}: {n} GPU UUID occurrence(s) not in the baseline")
    for rel, n in stale:
        print(f"STALE {rel}: {n} baselined occurrence(s) no longer found")
    print(
        f"gpu-uuids: files={len(actual)} baselined={len(baseline)} "
        f"new={len(new)} stale={len(stale)}"
    )
    if new:
        print(
            "A GPU UUID identifies one board. Scrub it (for example "
            f"GPU-{SYNTHETIC_PREFIX}0001) instead of adding it to the baseline."
        )
    if stale:
        print(
            "The baseline lists UUIDs that are gone: run scripts/ci/no_gpu_uuids.py --shrink"
        )
    return 1 if new or stale else 0


if __name__ == "__main__":
    sys.exit(main())
