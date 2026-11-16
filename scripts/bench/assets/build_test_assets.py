#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Standalone builder for the bundle of open test assets (stdlib only).

The bundle is a set of redistributable files (raster and raw photos, vector
files, PDF, video, audio, 3D models) that GPU creative and 3D application
tests open. Every file keeps its own licence; ``test_assets.json`` records the
source URL, licence, attribution, sha256 and size of each one.

Subcommands: fetch, verify, licenses, iso, all, pin. See README.md.

Downloaded files are untrusted data: this program only hashes and copies them.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

SCHEMA = "kayfabe.test-assets/1"
HERE = Path(__file__).resolve().parent
DEFAULT_MANIFEST = HERE / "test_assets.json"

VOLUME_LABEL = "KFDATA"
MAX_FILE = 300 * 1024 * 1024
MAX_TOTAL = 600 * 1024 * 1024
USER_AGENT = "kayfabe-test-assets/1 (+standalone builder; stdlib urllib)"
GENERATED = ("MANIFEST.json", "LICENSES.txt")

# Only these licences may appear. No NC/ND variant is representable here.
ALLOWED_LICENSES = frozenset({
    "CC0-1.0", "Public-Domain",
    "CC-BY-2.0", "CC-BY-2.5", "CC-BY-3.0", "CC-BY-4.0",
    "CC-BY-SA-3.0", "CC-BY-SA-4.0",
})
KEYS = ("name", "url", "license", "license_url", "share_alike", "attribution",
        "source_page", "sha256", "size", "purpose")

NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}(/[A-Za-z0-9][A-Za-z0-9._-]{0,63})?$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")


class AssetError(Exception):
    """A manifest, path, download or bundle problem (never a programming error)."""


# --------------------------------------------------------------------------- manifest

def validate_name(name: object) -> str:
    if not isinstance(name, str) or not NAME_RE.match(name):
        raise AssetError(f"unsafe asset name {name!r}")
    parts = name.split("/")
    if any(p in (".", "..") or p.endswith(".") for p in parts):
        raise AssetError(f"unsafe asset name {name!r}")
    if name in GENERATED or name.lower() in {g.lower() for g in GENERATED}:
        raise AssetError(f"asset name {name!r} collides with a generated file")
    return name


def validate_manifest(m: object) -> list[dict]:
    """Return the asset list or raise AssetError listing every problem found."""
    problems: list[str] = []
    if not isinstance(m, dict):
        raise AssetError("manifest is not a JSON object")
    if m.get("schema") != SCHEMA:
        problems.append(f"schema must be {SCHEMA!r}, got {m.get('schema')!r}")
    extra_top = set(m) - {"schema", "comment", "assets"}
    if extra_top:
        problems.append(f"unknown top-level keys {sorted(extra_top)}")
    assets = m.get("assets")
    if not isinstance(assets, list) or not assets:
        raise AssetError("; ".join(problems + ["assets must be a non-empty list"]))
    seen: set[str] = set()
    for i, a in enumerate(assets):
        tag = f"assets[{i}]"
        if not isinstance(a, dict):
            problems.append(f"{tag} is not an object")
            continue
        tag = f"assets[{i}] {a.get('name')!r}"
        if set(a) != set(KEYS):
            problems.append(f"{tag}: keys must be exactly {list(KEYS)}, "
                            f"missing {sorted(set(KEYS) - set(a))}, extra {sorted(set(a) - set(KEYS))}")
            continue
        try:
            validate_name(a["name"])
        except AssetError as e:
            problems.append(f"{tag}: {e}")
        else:
            low = a["name"].lower()
            if low in seen:
                problems.append(f"{tag}: duplicate name (case-insensitive)")
            seen.add(low)
        for k in ("url", "license_url", "source_page"):
            v = a[k]
            if not isinstance(v, str) or urllib.parse.urlsplit(v).scheme != "https" \
                    or not urllib.parse.urlsplit(v).netloc:
                problems.append(f"{tag}: {k} must be an https URL, got {v!r}")
        if a["license"] not in ALLOWED_LICENSES:
            problems.append(f"{tag}: licence {a['license']!r} is not allowed "
                            f"(allowed: {sorted(ALLOWED_LICENSES)})")
        if not isinstance(a["share_alike"], bool):
            problems.append(f"{tag}: share_alike must be a boolean")
        elif isinstance(a["license"], str) and ("-SA-" in a["license"]) != a["share_alike"]:
            problems.append(f"{tag}: share_alike={a['share_alike']} disagrees with licence {a['license']}")
        for k in ("attribution", "purpose"):
            if not isinstance(a[k], str) or not a[k].strip():
                problems.append(f"{tag}: {k} must be a non-empty string")
        sha, size = a["sha256"], a["size"]
        if sha is not None and not (isinstance(sha, str) and SHA_RE.match(sha)):
            problems.append(f"{tag}: sha256 must be 64 lowercase hex digits or null")
        if size is not None and not (isinstance(size, int) and not isinstance(size, bool) and size > 0):
            problems.append(f"{tag}: size must be a positive integer or null")
        if (sha is None) != (size is None):
            problems.append(f"{tag}: sha256 and size must be pinned together")
        if isinstance(size, int) and not isinstance(size, bool) and size > MAX_FILE:
            problems.append(f"{tag}: size {size} exceeds the per-file limit {MAX_FILE}")
    total = sum(a["size"] for a in assets
                if isinstance(a, dict) and isinstance(a.get("size"), int) and not isinstance(a.get("size"), bool))
    if total > MAX_TOTAL:
        problems.append(f"total size {total} exceeds the bundle limit {MAX_TOTAL}")
    if problems:
        raise AssetError("manifest invalid:\n  " + "\n  ".join(problems))
    return assets


def load_manifest(path: Path) -> tuple[dict, list[dict]]:
    try:
        with open(path, "r", encoding="utf-8") as f:
            m = json.load(f)
    except (OSError, ValueError) as e:
        raise AssetError(f"cannot read manifest {path}: {e}") from e
    return m, validate_manifest(m)


def write_manifest(path: Path, m: dict) -> None:
    text = json.dumps(m, indent=2, ensure_ascii=False) + "\n"
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, path)


def safe_join(root: Path, name: str) -> Path:
    """Join an asset name under root; refuse anything that resolves outside it."""
    validate_name(name)
    rootr = Path(os.path.abspath(root))
    p = Path(os.path.abspath(rootr / name))
    if rootr != p and rootr not in p.parents:
        raise AssetError(f"path traversal refused: {name!r}")
    return p


# --------------------------------------------------------------------------- hashing / scanning

def sha256_file(path: Path) -> tuple[str, int]:
    h = hashlib.sha256()
    n = 0
    with open(path, "rb") as f:
        while True:
            b = f.read(1 << 20)
            if not b:
                break
            h.update(b)
            n += len(b)
    return h.hexdigest(), n


def scan_files(root: Path) -> list[str]:
    """Relative POSIX names of every non-directory entry under root. Symlinks are reported
    as-is (and then rejected by verify): they are never followed."""
    out: list[str] = []
    for dp, dns, fns in os.walk(root, followlinks=False):
        rel = Path(dp).relative_to(root)
        for d in list(dns):
            if (Path(dp) / d).is_symlink():
                out.append((rel / d).as_posix())
                dns.remove(d)
        for fn in fns:
            out.append((rel / fn).as_posix() if str(rel) != "." else fn)
    return sorted(out)


# --------------------------------------------------------------------------- fetch

class _HttpsOnlyRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise AssetError(f"redirect to non-https URL refused: {newurl}")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _opener() -> urllib.request.OpenerDirector:
    return urllib.request.build_opener(_HttpsOnlyRedirect())


def download(url: str, dest: Path, expect_sha: str | None, expect_size: int | None,
             budget: list[int], opener=None, retries: int = 6) -> tuple[str, int]:
    """Download url to dest (atomically) under the size limits.

    budget is a one-element list holding the remaining bundle bytes. Returns (sha256, size).
    A mismatch against expect_* removes the partial file and raises AssetError.
    """
    if urllib.parse.urlsplit(url).scheme != "https":
        raise AssetError(f"refusing non-https URL {url}")
    opener = opener or _opener()
    limit = min(MAX_FILE, expect_size if expect_size is not None else MAX_FILE)
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    last: Exception | None = None
    for attempt in range(retries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with opener.open(req, timeout=60) as r:
                clen = r.headers.get("Content-Length")
                if clen is not None and clen.isdigit():
                    if int(clen) > min(limit, budget[0]):
                        raise AssetError(f"{url}: Content-Length {clen} exceeds the limit")
                    if expect_size is not None and int(clen) != expect_size:
                        raise AssetError(f"{url}: Content-Length {clen} != manifest size {expect_size}")
                h = hashlib.sha256()
                n = 0
                with open(part, "wb") as f:
                    while True:
                        b = r.read(1 << 20)
                        if not b:
                            break
                        n += len(b)
                        if n > limit or n > budget[0]:
                            raise AssetError(f"{url}: exceeds the size limit while downloading")
                        h.update(b)
                        f.write(b)
            break
        except urllib.error.HTTPError as e:
            last = e
            if e.code in (429, 500, 502, 503, 504) and attempt + 1 < retries:
                ra = e.headers.get("Retry-After") if e.headers else None
                time.sleep(min(660, int(ra)) if ra and ra.isdigit() else 10 * (attempt + 1))
                continue
            _rm(part)
            raise AssetError(f"{url}: HTTP {e.code}") from e
        except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
            last = e
            if attempt + 1 < retries:
                time.sleep(3 * (attempt + 1))
                continue
            _rm(part)
            raise AssetError(f"{url}: {e}") from e
        except AssetError:
            _rm(part)
            raise
    else:  # pragma: no cover - loop always breaks or raises
        raise AssetError(f"{url}: {last}")
    digest = h.hexdigest()
    if expect_size is not None and n != expect_size:
        _rm(part)
        raise AssetError(f"{url}: size {n} != manifest size {expect_size}")
    if expect_sha is not None and digest != expect_sha:
        _rm(part)
        raise AssetError(f"{url}: sha256 {digest} != manifest {expect_sha}")
    os.replace(part, dest)
    budget[0] -= n
    return digest, n


def _rm(p: Path) -> None:
    try:
        p.unlink()
    except FileNotFoundError:
        pass


def cmd_fetch(manifest: Path, dest: Path, allow_unpinned: bool = False, opener=None) -> int:
    _, assets = load_manifest(manifest)
    unpinned = [a["name"] for a in assets if a["sha256"] is None]
    if unpinned and not allow_unpinned:
        raise AssetError(f"unpinned assets (run fetch --allow-unpinned then pin): {unpinned}")
    dest.mkdir(parents=True, exist_ok=True)
    budget = [MAX_TOTAL]
    for a in assets:
        target = safe_join(dest, a["name"])
        if target.is_symlink():
            raise AssetError(f"{a['name']}: destination is a symlink")
        if a["sha256"] is not None and target.is_file():
            sha, n = sha256_file(target)
            if (sha, n) == (a["sha256"], a["size"]):
                budget[0] -= n
                print(f"ok    {a['name']} (already present)")
                continue
        if a["sha256"] is None and allow_unpinned and target.is_file():
            budget[0] -= target.stat().st_size
            print(f"keep  {a['name']} (unpinned, already present; delete it to fetch again)")
            continue
        print(f"fetch {a['name']} <- {a['url']}")
        sha, n = download(a["url"], target, a["sha256"], a["size"], budget, opener=opener)
        print(f"      {n} bytes sha256 {sha}")
    return 0


# --------------------------------------------------------------------------- verify

def verify_dir(manifest: Path, root: Path) -> list[str]:
    """Return a list of problems (empty means the directory matches the manifest exactly)."""
    _, assets = load_manifest(manifest)
    problems: list[str] = []
    if not root.is_dir():
        return [f"{root} is not a directory"]
    names = {a["name"] for a in assets}
    for a in assets:
        if a["sha256"] is None:
            problems.append(f"{a['name']}: not pinned in the manifest")
            continue
        p = safe_join(root, a["name"])
        if p.is_symlink() or not p.is_file():
            problems.append(f"{a['name']}: missing (or not a regular file)")
            continue
        sha, n = sha256_file(p)
        if n != a["size"]:
            problems.append(f"{a['name']}: size {n} != manifest {a['size']}")
        elif sha != a["sha256"]:
            problems.append(f"{a['name']}: sha256 {sha} != manifest {a['sha256']}")
    for rel in scan_files(root):
        if rel in names or rel in GENERATED:
            continue
        problems.append(f"{rel}: not in the manifest")
    return problems


def cmd_verify(manifest: Path, root: Path) -> int:
    problems = verify_dir(manifest, root)
    for p in problems:
        print(f"FAIL {p}", file=sys.stderr)
    if problems:
        print(f"verify: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print(f"verify: ok ({len(load_manifest(manifest)[1])} assets)")
    return 0


# --------------------------------------------------------------------------- licences

def licenses_text(assets: list[dict]) -> str:
    lines = [
        "LICENSES for the test asset bundle",
        "==================================",
        "",
        "Every file below keeps its own licence. This bundle only collects them.",
        "Attribution texts are reproduced as the sources require. Files marked",
        "SHARE-ALIKE must stay under their licence when redistributed or adapted.",
        "",
    ]
    for a in sorted(assets, key=lambda x: x["name"]):
        lines += [
            f"File:         {a['name']}",
            f"Licence:      {a['license']}",
            f"Licence URL:  {a['license_url']}",
            f"Attribution:  {a['attribution']}",
            f"Source page:  {a['source_page']}",
            "Share-alike:  " + ("YES - derivatives and redistribution must use the same licence"
                                if a["share_alike"] else "no"),
            f"Used for:     {a['purpose']}",
            "",
        ]
    return "\n".join(lines)


def write_bundle_meta(manifest: Path, root: Path) -> None:
    m, assets = load_manifest(manifest)
    (root / "LICENSES.txt").write_text(licenses_text(assets), encoding="utf-8")
    (root / "MANIFEST.json").write_text(json.dumps(m, indent=2, ensure_ascii=False) + "\n",
                                        encoding="utf-8")


def cmd_licenses(manifest: Path, root: Path) -> int:
    if not root.is_dir():
        raise AssetError(f"{root} is not a directory")
    write_bundle_meta(manifest, root)
    print(f"wrote {root / 'LICENSES.txt'} and {root / 'MANIFEST.json'}")
    return 0


# --------------------------------------------------------------------------- pin

def cmd_pin(manifest: Path, root: Path) -> int:
    m, assets = load_manifest(manifest)
    errors: list[str] = []
    updates: list[tuple[dict, str, int]] = []
    for a in assets:
        p = safe_join(root, a["name"])
        if p.is_symlink() or not p.is_file():
            if a["sha256"] is None:
                errors.append(f"{a['name']}: unpinned and not present in {root}")
            continue
        sha, n = sha256_file(p)
        if a["sha256"] is None:
            updates.append((a, sha, n))
        elif (a["sha256"], a["size"]) != (sha, n):
            errors.append(f"{a['name']}: pinned {a['sha256']}/{a['size']} differs from file {sha}/{n}; "
                          "refusing to overwrite a pin")
    if errors:
        for e in errors:
            print(f"pin: {e}", file=sys.stderr)
        return 1
    for a, sha, n in updates:
        a["sha256"], a["size"] = sha, n
        print(f"pinned {a['name']} {n} {sha}")
    validate_manifest(m)
    if updates:
        write_manifest(manifest, m)
    print(f"pin: {len(updates)} newly pinned")
    return 0


# --------------------------------------------------------------------------- ISO (tool)

def find_iso_tool() -> list[str] | None:
    for tool, pre in (("genisoimage", []), ("mkisofs", []), ("xorriso", ["-as", "mkisofs"])):
        p = shutil.which(tool)
        if p:
            return [p] + pre
    return None


def bundle_entries(manifest: Path, root: Path) -> list[tuple[str, Path]]:
    """(iso path, real path) for the assets plus the generated files, after a full verify."""
    problems = verify_dir(manifest, root)
    if problems:
        raise AssetError("refusing to build an ISO from an unverified directory:\n  " + "\n  ".join(problems))
    write_bundle_meta(manifest, root)
    _, assets = load_manifest(manifest)
    out = [(a["name"], safe_join(root, a["name"])) for a in assets]
    out += [(g, root / g) for g in GENERATED]
    return sorted(out)


def _check_out(root: Path, out: Path) -> None:
    ro, oo = Path(os.path.abspath(root)), Path(os.path.abspath(out))
    if ro == oo or ro in oo.parents:
        raise AssetError("the ISO output file must not be inside the asset directory")


def build_iso_tool(tool: list[str], entries: list[tuple[str, Path]], out: Path) -> None:
    grafts = []
    for name, real in entries:
        if "=" in str(real) or "\\" in str(real):
            raise AssetError(f"path {real} contains '=' or a backslash; cannot pass it to {tool[0]}")
        grafts.append(f"{name}={real}")
    tmp = out.with_name(out.name + ".part")
    cmd = tool + ["-quiet", "-V", VOLUME_LABEL, "-r", "-J", "-joliet-long", "-iso-level", "3",
                  "-input-charset", "utf-8", "-graft-points", "-o", str(tmp)] + grafts
    try:
        subprocess.run(cmd, check=True, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.CalledProcessError) as e:
        _rm(tmp)
        raise AssetError(f"{tool[0]} failed: {e}") from e
    os.replace(tmp, out)


# --------------------------------------------------------------------------- ISO (pure Python)
# Minimal ISO 9660 writer: one primary tree (8.3 upper-case names) and one Joliet tree
# (UCS-2 names, so the real file names survive), sharing the same file extents. Files
# live in the root or in one directory level, which is all the manifest allows.

SECTOR = 2048


def _both32(n: int) -> bytes:
    return struct.pack("<I", n) + struct.pack(">I", n)


def _both16(n: int) -> bytes:
    return struct.pack("<H", n) + struct.pack(">H", n)


def _rec_date(ts: int) -> bytes:
    t = time.gmtime(ts)
    return bytes([t.tm_year - 1900, t.tm_mon, t.tm_mday, t.tm_hour, t.tm_min, t.tm_sec, 0])


def _vol_date(ts: int) -> bytes:
    t = time.gmtime(ts)
    return (time.strftime("%Y%m%d%H%M%S", t) + "00").encode() + b"\x00"


def _dir_record(ident: bytes, extent: int, length: int, is_dir: bool, ts: int) -> bytes:
    n = 33 + len(ident)
    pad = 1 if len(ident) % 2 == 0 else 0
    return (bytes([n + pad, 0]) + _both32(extent) + _both32(length) + _rec_date(ts)
            + bytes([2 if is_dir else 0, 0, 0]) + _both16(1) + bytes([len(ident)]) + ident
            + b"\x00" * pad)


def _primary_id(name: str, is_dir: bool, taken: set[bytes]) -> bytes:
    def clean(s: str) -> str:
        return re.sub(r"[^A-Z0-9_]", "_", s.upper())
    if is_dir:
        base, ext = clean(name)[:8], ""
    else:
        stem, dot, e = name.rpartition(".")
        if not dot:
            stem, e = name, ""
        base, ext = clean(stem)[:8], clean(e)[:3]
    n = 0
    while True:
        b = base if n == 0 else base[:max(1, 8 - len(str(n)))] + str(n)
        ident = (b + ("." + ext if ext else "") + ("" if is_dir else ";1")).encode("ascii")
        if ident not in taken:
            taken.add(ident)
            return ident
        n += 1


def _joliet_id(name: str) -> bytes:
    if len(name) > 64:
        raise AssetError(f"name too long for Joliet: {name!r}")
    return name.encode("utf-16-be")


class _Tree:
    """One directory tree (primary or Joliet) with ids, sorted children and extents."""

    def __init__(self, entries: list[tuple[str, Path, int]], joliet: bool):
        self.joliet = joliet
        dirs = sorted({n.split("/")[0] for n, _, _ in entries if "/" in n})
        taken: dict[str, set[bytes]] = {"": set()}
        self.dir_ids: dict[str, bytes] = {}
        for d in dirs:
            self.dir_ids[d] = _joliet_id(d) if joliet else _primary_id(d, True, taken[""])
            taken[d] = set()
        # content[dirname] = list of (id, is_dir, ref) where ref is dirname or (name, path, size)
        self.content: dict[str, list] = {"": [(self.dir_ids[d], True, d) for d in dirs]}
        for d in dirs:
            self.content[d] = []
        for name, path, size in entries:
            d, _, base = name.rpartition("/")
            ident = _joliet_id(base) if joliet else _primary_id(base, False, taken[d])
            self.content[d].append((ident, False, (name, path, size)))
        for d in self.content:
            self.content[d].sort(key=lambda c: c[0])
        # directory numbers follow path table order: root, then subdirectories by id
        self.order = [""] + sorted(dirs, key=lambda d: self.dir_ids[d])
        self.extent: dict[str, int] = {}
        self.length: dict[str, int] = {}
        for d in self.order:
            self.length[d] = self._size(d)

    def _size(self, d: str) -> int:
        off = 0
        for rl in [34, 34] + [self._rl(c[0]) for c in self.content[d]]:
            if off % SECTOR + rl > SECTOR:
                off += SECTOR - off % SECTOR
            off += rl
        return -(-off // SECTOR) * SECTOR

    @staticmethod
    def _rl(ident: bytes) -> int:
        return 33 + len(ident) + (1 if len(ident) % 2 == 0 else 0)

    def path_table(self, big: bool) -> bytes:
        e = ">" if big else "<"
        out = b""
        for i, d in enumerate(self.order):
            ident = b"\x00" if d == "" else self.dir_ids[d]
            out += (bytes([len(ident), 0]) + struct.pack(e + "I", self.extent.get(d, 0))
                    + struct.pack(e + "H", 1) + ident + (b"\x00" if len(ident) % 2 else b""))
        return out

    def render_dir(self, d: str, file_extent: dict[str, int], ts: int) -> bytes:
        recs = [_dir_record(b"\x00", self.extent[d], self.length[d], True, ts),
                _dir_record(b"\x01", self.extent[""], self.length[""], True, ts)]
        for ident, is_dir, ref in self.content[d]:
            if is_dir:
                recs.append(_dir_record(ident, self.extent[ref], self.length[ref], True, ts))
            else:
                recs.append(_dir_record(ident, file_extent[ref[0]], ref[2], False, ts))
        out = bytearray()
        for r in recs:
            if len(out) % SECTOR + len(r) > SECTOR:
                out += b"\x00" * (SECTOR - len(out) % SECTOR)
            out += r
        out += b"\x00" * (self.length[d] - len(out))
        return bytes(out)


def _text_field(s: str, size: int, joliet: bool) -> bytes:
    if joliet:
        b = s.encode("utf-16-be")
        return b + " ".encode("utf-16-be") * ((size - len(b)) // 2)
    return s.encode("ascii").ljust(size, b" ")


def _descriptor(kind: int, joliet: bool, label: str, total: int, ptsize: int, ptl: int, ptm: int,
                root: bytes, ts: int) -> bytes:
    d = bytearray(SECTOR)
    d[0] = kind
    d[1:6] = b"CD001"
    d[6] = 1
    d[8:40] = _text_field("", 32, joliet)
    d[40:72] = _text_field(label, 32, joliet)
    d[80:88] = _both32(total)
    if joliet:
        d[88:91] = b"%/E"
    d[120:124] = _both16(1)
    d[124:128] = _both16(1)
    d[128:132] = _both16(SECTOR)
    d[132:140] = _both32(ptsize)
    d[140:144] = struct.pack("<I", ptl)
    d[148:152] = struct.pack(">I", ptm)
    d[156:190] = root
    for off, size in ((190, 128), (318, 128), (446, 128), (574, 128)):
        d[off:off + size] = _text_field("", size, joliet)
    for off in (702, 739, 776):
        d[off:off + 37] = b" " * 37 if not joliet else b"\x00" * 37
    d[813:830] = _vol_date(ts)
    d[830:847] = _vol_date(ts)
    d[847:864] = b"0" * 16 + b"\x00"
    d[864:881] = _vol_date(ts)
    d[881] = 1
    return bytes(d)


def build_iso_python(entries: list[tuple[str, Path]], out: Path, label: str = VOLUME_LABEL,
                     timestamp: int | None = None) -> None:
    """Write a Joliet + ISO 9660 image of the given (iso name, real path) files."""
    if timestamp is None:
        timestamp = int(os.environ.get("SOURCE_DATE_EPOCH", "1767225600"))
    sized = []
    for name, path in entries:
        validate_name_iso(name)
        sized.append((name, path, os.stat(path).st_size))
    if len({n for n, _, _ in sized}) != len(sized):
        raise AssetError("duplicate names in ISO entries")
    prim, jol = _Tree(sized, False), _Tree(sized, True)
    pt_size = {t: len(t.path_table(False)) for t in (prim, jol)}  # sizes do not depend on extents
    pt_sectors = {t: -(-pt_size[t] // SECTOR) for t in (prim, jol)}
    pos = 19
    ptpos = {}
    for t in (prim, jol):
        ptpos[t] = (pos, pos + pt_sectors[t])
        pos += 2 * pt_sectors[t]
    for t in (prim, jol):
        for d in t.order:
            t.extent[d] = pos
            pos += t.length[d] // SECTOR
    file_extent: dict[str, int] = {}
    for name, _, size in sized:
        file_extent[name] = pos
        pos += max(1, -(-size // SECTOR)) if size else 0
    total = pos

    def root_rec(t: _Tree) -> bytes:
        return _dir_record(b"\x00", t.extent[""], t.length[""], True, timestamp)

    tmp = out.with_name(out.name + ".part")
    with open(tmp, "wb") as f:
        f.write(b"\x00" * (16 * SECTOR))
        f.write(_descriptor(1, False, label, total, pt_size[prim], ptpos[prim][0], ptpos[prim][1],
                            root_rec(prim), timestamp))
        f.write(_descriptor(2, True, label, total, pt_size[jol], ptpos[jol][0], ptpos[jol][1],
                            root_rec(jol), timestamp))
        f.write(b"\xff" + b"CD001" + b"\x01" + b"\x00" * (SECTOR - 7))
        for t in (prim, jol):
            for big in (False, True):
                pt = t.path_table(big)
                f.write(pt + b"\x00" * (pt_sectors[t] * SECTOR - len(pt)))
        for t in (prim, jol):
            for d in t.order:
                f.write(t.render_dir(d, file_extent, timestamp))
        for name, path, size in sorted(sized, key=lambda s: file_extent[s[0]]):
            if f.tell() != file_extent[name] * SECTOR:
                raise AssetError("internal error: ISO extent mismatch")
            with open(path, "rb") as src:
                copied = 0
                while True:
                    b = src.read(1 << 20)
                    if not b:
                        break
                    copied += len(b)
                    f.write(b)
            if copied != size:
                raise AssetError(f"{path} changed while building the ISO")
            f.write(b"\x00" * (-size % SECTOR))
        if f.tell() != total * SECTOR:
            raise AssetError("internal error: ISO size mismatch")
    os.replace(tmp, out)


def validate_name_iso(name: str) -> None:
    if name in GENERATED:
        return
    validate_name(name)


def cmd_iso(manifest: Path, root: Path, out: Path, engine: str = "auto") -> int:
    _check_out(root, out)
    entries = bundle_entries(manifest, root)
    out.parent.mkdir(parents=True, exist_ok=True)
    tool = find_iso_tool() if engine in ("auto", "tool") else None
    if tool is None:
        if engine == "tool":
            raise AssetError("no genisoimage, mkisofs or xorriso found on PATH")
        if engine == "auto":
            print("note: none of genisoimage, mkisofs, xorriso found; using the built-in "
                  "pure-Python ISO writer (ISO 9660 + Joliet)", file=sys.stderr)
        build_iso_python(entries, out)
        how = "python"
    else:
        build_iso_tool(tool, entries, out)
        how = Path(tool[0]).name
    print(f"iso: {out} ({out.stat().st_size} bytes, label {VOLUME_LABEL}, engine {how})")
    return 0


# --------------------------------------------------------------------------- CLI

def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="build_test_assets.py", description=__doc__.splitlines()[0])
    p.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST,
                   help="manifest JSON (default: test_assets.json beside this script)")
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("fetch", help="download every asset and check it against the manifest")
    s.add_argument("--dest", type=Path, required=True)
    s.add_argument("--allow-unpinned", action="store_true",
                   help="maintainer: download entries whose sha256/size are still null (then run pin)")
    s = sub.add_parser("verify", help="check hashes, sizes and that no other file is present")
    s.add_argument("--dir", type=Path, required=True)
    s = sub.add_parser("licenses", help="write LICENSES.txt (and MANIFEST.json) into the directory")
    s.add_argument("--dir", type=Path, required=True)
    s = sub.add_parser("iso", help="build the read-only ISO (label KFDATA)")
    s.add_argument("--dir", type=Path, required=True)
    s.add_argument("--out", type=Path, required=True)
    s.add_argument("--engine", choices=("auto", "tool", "python"), default="auto")
    s = sub.add_parser("all", help="fetch, verify, licenses, iso")
    s.add_argument("--dest", type=Path, required=True)
    s.add_argument("--iso", type=Path, required=True)
    s.add_argument("--engine", choices=("auto", "tool", "python"), default="auto")
    s = sub.add_parser("pin", help="maintainer: fill null sha256/size from downloaded files")
    s.add_argument("--dir", type=Path, required=True)
    return p


def main(argv: list[str] | None = None) -> int:
    a = build_parser().parse_args(argv)
    try:
        if a.cmd == "fetch":
            return cmd_fetch(a.manifest, a.dest, a.allow_unpinned)
        if a.cmd == "verify":
            return cmd_verify(a.manifest, a.dir)
        if a.cmd == "licenses":
            return cmd_licenses(a.manifest, a.dir)
        if a.cmd == "iso":
            return cmd_iso(a.manifest, a.dir, a.out, a.engine)
        if a.cmd == "pin":
            return cmd_pin(a.manifest, a.dir)
        if a.cmd == "all":
            for step in (lambda: cmd_fetch(a.manifest, a.dest),
                         lambda: cmd_verify(a.manifest, a.dest),
                         lambda: cmd_licenses(a.manifest, a.dest),
                         lambda: cmd_iso(a.manifest, a.dest, a.iso, a.engine)):
                rc = step()
                if rc:
                    return rc
            return 0
    except AssetError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 2  # pragma: no cover


if __name__ == "__main__":
    sys.exit(main())
