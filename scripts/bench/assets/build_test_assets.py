#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Standalone builder for the bundle of open test assets (stdlib only).

The bundle is a set of redistributable files (raster and raw photos, vector
files, PDF, video, audio, 3D models) that GPU creative and 3D application
tests open. Every file keeps its own licence; ``test_assets.json`` records the
source URL, licence, attribution, sha256 and size of each one.

Subcommands: fetch, verify, licenses, iso, all, pin, synth, pin-synth. See README.md.

Every asset has a ``source_kind``: ``origin`` (a stable source URL, no copy needed),
``public-mirror`` (open licence, unstable origin: also kept in the public bucket under
``mirror_path``) or ``private`` (redistribution prohibited: never in the ISO, never in the
published licence list). A ``synthetic`` section lists deterministic generated files.

Downloaded files are untrusted data: this program only hashes and copies them.
"""

from __future__ import annotations

import argparse
import array
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
import zlib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

SCHEMA = "kayfabe.test-assets/2"
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
    # Permissive software licences, admitted for the project files (project/) whose
    # source repository carries one of them. Copyleft (GPL family) stays out.
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib",
})
KEYS = ("name", "url", "source_kind", "license", "license_url", "share_alike", "attribution",
        "source_page", "sha256", "size", "purpose")
OPTIONAL_KEYS = ("mirror_path", "fallback_url", "value")
SOURCE_KINDS = ("origin", "public-mirror", "private")
PRIVATE_LICENSE = "Proprietary"
PRIVATE_MAX_FILE = 2 * 1024 * 1024 * 1024
# Hosts whose URLs are not stable (files are re-uploaded or renamed in place): an entry
# from one of them must be a public-mirror, never an origin.
UNSTABLE_HOST_SUFFIXES = ("wikimedia.org", "wikipedia.org")
GITHUB_HOSTS = ("raw.githubusercontent.com", "media.githubusercontent.com", "github.com")
MIRROR_RE = re.compile(r"^test-assets/\d{8}/")
COMMIT_RE = re.compile(r"/[0-9a-f]{40}/")
SYNTH_KEYS = ("name", "generator", "output", "description", "requires",
              "sha256", "size", "sha256_prefix", "prefix_bytes")
PREFIX_BYTES_DEFAULT = 65536

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


def _is_https(v: object) -> bool:
    return isinstance(v, str) and urllib.parse.urlsplit(v).scheme == "https" \
        and bool(urllib.parse.urlsplit(v).netloc)


def _host(url: str) -> str:
    return (urllib.parse.urlsplit(url).hostname or "").lower()


def _validate_asset(a: dict, tag: str, problems: list[str]) -> None:
    kind = a["source_kind"]
    if kind not in SOURCE_KINDS:
        problems.append(f"{tag}: source_kind must be one of {list(SOURCE_KINDS)}, got {kind!r}")
        return
    try:
        validate_name(a["name"])
    except AssetError as e:
        problems.append(f"{tag}: {e}")
    allowed_extra = {"origin": set(), "public-mirror": {"mirror_path"},
                     "private": {"fallback_url", "value"}}[kind]
    keys = set(a)
    if not set(KEYS) <= keys or keys - set(KEYS) - allowed_extra:
        problems.append(f"{tag}: keys must be exactly {list(KEYS)} plus "
                        f"{sorted(allowed_extra)} for a {kind} entry; missing {sorted(set(KEYS) - keys)}, "
                        f"extra {sorted(keys - set(KEYS) - allowed_extra)}")
        return
    private = kind == "private"
    # URLs: a private entry may have no origin URL at all and keeps an optional fallback.
    for k in ("url", "license_url", "source_page"):
        v = a[k]
        if private and v is None:
            continue
        if not _is_https(v):
            problems.append(f"{tag}: {k} must be an https URL, got {v!r}")
    if private:
        fb = a.get("fallback_url")
        if "fallback_url" in a and fb is not None and not _is_https(fb):
            problems.append(f"{tag}: fallback_url must be an https URL or null, got {fb!r}")
        v = a.get("value")
        if not isinstance(v, str) or len(v.strip()) < 20:
            problems.append(f"{tag}: a private entry needs a 'value' text (at least 20 characters) "
                            "saying why the file is worth having and why no open equivalent works")
        if a["license"] != PRIVATE_LICENSE:
            problems.append(f"{tag}: a private entry has licence {PRIVATE_LICENSE!r}, got {a['license']!r}")
        if a["share_alike"] is not False:
            problems.append(f"{tag}: a private entry has share_alike false")
    else:
        if a["license"] not in ALLOWED_LICENSES:
            problems.append(f"{tag}: licence {a['license']!r} is not allowed "
                            f"(allowed: {sorted(ALLOWED_LICENSES)})")
        if not isinstance(a["share_alike"], bool):
            problems.append(f"{tag}: share_alike must be a boolean")
        elif isinstance(a["license"], str) and ("-SA-" in a["license"]) != a["share_alike"]:
            problems.append(f"{tag}: share_alike={a['share_alike']} disagrees with licence {a['license']}")
    if kind == "origin" and _is_https(a["url"]):
        host = _host(a["url"])
        if any(host == h or host.endswith("." + h) for h in UNSTABLE_HOST_SUFFIXES):
            problems.append(f"{tag}: {host} is not a stable origin; use source_kind public-mirror")
        if host in GITHUB_HOSTS and not COMMIT_RE.search(a["url"]):
            problems.append(f"{tag}: a GitHub origin URL must be pinned to a 40-hex commit, not a branch")
    if kind == "public-mirror":
        mp = a.get("mirror_path")
        name = a["name"] if isinstance(a["name"], str) else ""
        if not (isinstance(mp, str) and MIRROR_RE.match(mp) and mp.split("/", 2)[2:] == [name]):
            problems.append(f"{tag}: mirror_path must be 'test-assets/<YYYYMMDD>/{name}', got {mp!r}")
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
    cap = PRIVATE_MAX_FILE if private else MAX_FILE
    if isinstance(size, int) and not isinstance(size, bool) and size > cap:
        problems.append(f"{tag}: size {size} exceeds the per-file limit {cap}")


def _validate_synthetic(entries: object, problems: list[str]) -> None:
    if not isinstance(entries, list):
        problems.append("synthetic must be a list")
        return
    seen_names: set[str] = set()
    seen_out: set[str] = set()
    for i, s in enumerate(entries):
        tag = f"synthetic[{i}]"
        if not isinstance(s, dict):
            problems.append(f"{tag} is not an object")
            continue
        tag = f"synthetic[{i}] {s.get('name')!r}"
        if set(s) != set(SYNTH_KEYS):
            problems.append(f"{tag}: keys must be exactly {list(SYNTH_KEYS)}, "
                            f"missing {sorted(set(SYNTH_KEYS) - set(s))}, extra {sorted(set(s) - set(SYNTH_KEYS))}")
            continue
        for k in ("name", "output"):
            try:
                validate_name(s[k])
            except AssetError as e:
                problems.append(f"{tag}: {k}: {e}")
        if isinstance(s["name"], str):
            if s["name"].lower() in seen_names:
                problems.append(f"{tag}: duplicate synthetic name")
            seen_names.add(s["name"].lower())
        if isinstance(s["output"], str):
            if s["output"].lower() in seen_out:
                problems.append(f"{tag}: duplicate synthetic output")
            seen_out.add(s["output"].lower())
        g = s["generator"]
        if isinstance(g, str):
            if g not in BUILTIN_GENERATORS:
                problems.append(f"{tag}: unknown built-in generator {g!r} (known: {sorted(BUILTIN_GENERATORS)})")
        elif isinstance(g, list) and g and all(isinstance(x, str) and x for x in g):
            if sum(x == OUTPUT_TOKEN for x in g) != 1:
                problems.append(f"{tag}: an argv generator needs exactly one {OUTPUT_TOKEN!r} element")
            if isinstance(s["requires"], list) and g[0] not in s["requires"]:
                problems.append(f"{tag}: the tool {g[0]!r} must be listed in requires")
        else:
            problems.append(f"{tag}: generator must be a built-in name or a non-empty argv list of strings")
        if not isinstance(s["requires"], list) or not all(isinstance(x, str) and x for x in s["requires"]):
            problems.append(f"{tag}: requires must be a list of tool names")
        if not isinstance(s["description"], str) or not s["description"].strip():
            problems.append(f"{tag}: description must be a non-empty string")
        sha, size, pre, pb = s["sha256"], s["size"], s["sha256_prefix"], s["prefix_bytes"]
        for k, v in (("sha256", sha), ("sha256_prefix", pre)):
            if v is not None and not (isinstance(v, str) and SHA_RE.match(v)):
                problems.append(f"{tag}: {k} must be 64 lowercase hex digits or null")
        for k, v in (("size", size), ("prefix_bytes", pb)):
            if v is not None and not (isinstance(v, int) and not isinstance(v, bool) and v > 0):
                problems.append(f"{tag}: {k} must be a positive integer or null")
        full, prefix = (sha is not None, size is not None), (pre is not None, pb is not None)
        if len(set(full)) != 1 or len(set(prefix)) != 1:
            problems.append(f"{tag}: sha256 and size (or sha256_prefix and prefix_bytes) must be pinned together")
        elif all(full) and all(prefix):
            problems.append(f"{tag}: pin the whole file (sha256 + size) or a prefix, not both")
        if isinstance(size, int) and not isinstance(size, bool) and size > MAX_FILE:
            problems.append(f"{tag}: size {size} exceeds the per-file limit {MAX_FILE}")


def validate_manifest(m: object) -> list[dict]:
    """Return the asset list or raise AssetError listing every problem found."""
    problems: list[str] = []
    if not isinstance(m, dict):
        raise AssetError("manifest is not a JSON object")
    if m.get("schema") != SCHEMA:
        problems.append(f"schema must be {SCHEMA!r}, got {m.get('schema')!r}")
    extra_top = set(m) - {"schema", "comment", "assets", "synthetic"}
    if extra_top:
        problems.append(f"unknown top-level keys {sorted(extra_top)}")
    assets = m.get("assets")
    if not isinstance(assets, list) or not assets:
        raise AssetError("; ".join(problems + ["assets must be a non-empty list"]))
    if "synthetic" in m:
        _validate_synthetic(m["synthetic"], problems)
    seen: set[str] = set()
    for i, a in enumerate(assets):
        tag = f"assets[{i}]"
        if not isinstance(a, dict):
            problems.append(f"{tag} is not an object")
            continue
        tag = f"assets[{i}] {a.get('name')!r}"
        if "source_kind" not in a:
            problems.append(f"{tag}: source_kind is required (one of {list(SOURCE_KINDS)})")
            continue
        _validate_asset(a, tag, problems)
        if isinstance(a.get("name"), str):
            low = a["name"].lower()
            if low in seen:
                problems.append(f"{tag}: duplicate name (case-insensitive)")
            seen.add(low)
    outs = {s["output"].lower() for s in m.get("synthetic", [])
            if isinstance(s, dict) and isinstance(s.get("output"), str)}
    for clash in sorted(outs & seen):
        problems.append(f"synthetic output {clash!r} collides with an asset name")
    total = sum(a["size"] for a in assets
                if isinstance(a, dict) and a.get("source_kind") != "private"
                and isinstance(a.get("size"), int) and not isinstance(a.get("size"), bool))
    if total > MAX_TOTAL:
        problems.append(f"total size {total} exceeds the bundle limit {MAX_TOTAL}")
    if problems:
        raise AssetError("manifest invalid:\n  " + "\n  ".join(problems))
    return assets


def synthetic_entries(m: dict) -> list[dict]:
    return list(m.get("synthetic", []))


def public_assets(assets: list[dict]) -> list[dict]:
    """The entries that may enter the ISO and the published licence list."""
    return [a for a in assets if a["source_kind"] != "private"]


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


def mirror_url(base: str, mirror_path: str) -> str:
    if not _is_https(base):
        raise AssetError(f"--mirror-base must be an https URL, got {base!r}")
    return base.rstrip("/") + "/" + urllib.parse.quote(mirror_path)


def _fetch_one(a: dict, target: Path, budget: list[int], opener, mirror_base: str | None,
               unpinned_ok: bool) -> None:
    """Download one public entry: origin first, then (public-mirror only) the mirror.

    Every source is verified against the manifest pin, so a mirror cannot change the bytes."""
    mirror = (mirror_url(mirror_base, a["mirror_path"])
              if mirror_base and a["source_kind"] == "public-mirror" and a["sha256"] is not None else None)
    print(f"fetch {a['name']} <- {a['url']}")
    try:
        sha, n = download(a["url"], target, a["sha256"], a["size"], budget, opener=opener,
                          retries=2 if mirror else 6)
    except AssetError as e:
        if mirror is None:
            hint = ("; no --mirror-base given" if a["source_kind"] == "public-mirror" else "")
            raise AssetError(f"{e}{hint}") from e
        print(f"      origin failed ({e}); trying the public mirror {mirror}")
        try:
            sha, n = download(mirror, target, a["sha256"], a["size"], budget, opener=opener)
        except AssetError as e2:
            raise AssetError(f"{a['name']}: origin and mirror both failed: origin: {e}; mirror: {e2}") from e2
    print(f"      {n} bytes sha256 {sha}")


def _check_private(assets: list[dict], private_dir: Path, dest: Path) -> list[str]:
    """Verify the private entries already present in private_dir; return the missing names."""
    pd, dd = Path(os.path.abspath(private_dir)), Path(os.path.abspath(dest))
    if pd == dd or dd in pd.parents or pd in dd.parents:
        raise AssetError("--private-dir must be a separate directory, not the bundle directory or a "
                         "parent or child of it")
    missing: list[str] = []
    for a in assets:
        if a["source_kind"] != "private":
            continue
        p = safe_join(private_dir, a["name"])
        if p.is_symlink() or not p.is_file():
            print(f"MISSING private {a['name']} (provision it into {private_dir}; it is never downloaded here)")
            missing.append(a["name"])
            continue
        if a["sha256"] is None:
            raise AssetError(f"{a['name']}: private entry is not pinned (run pin --dir {private_dir})")
        sha, n = sha256_file(p)
        if (sha, n) != (a["sha256"], a["size"]):
            raise AssetError(f"{a['name']}: private file {sha}/{n} differs from the pin "
                             f"{a['sha256']}/{a['size']}")
        print(f"ok    {a['name']} (private, present and pin verified)")
    return missing


def cmd_fetch(manifest: Path, dest: Path, allow_unpinned: bool = False, opener=None,
              mirror_base: str | None = None, include_private: bool = False,
              private_dir: Path | None = None) -> int:
    _, assets = load_manifest(manifest)
    if include_private and private_dir is None:
        raise AssetError("--include-private needs --private-dir DIR")
    if private_dir is not None and not include_private:
        raise AssetError("--private-dir is only used together with --include-private")
    public = public_assets(assets)
    unpinned = [a["name"] for a in public if a["sha256"] is None]
    if unpinned and not allow_unpinned:
        raise AssetError(f"unpinned assets (run fetch --allow-unpinned then pin): {unpinned}")
    dest.mkdir(parents=True, exist_ok=True)
    budget = [MAX_TOTAL]
    for a in public:
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
        _fetch_one(a, target, budget, opener, mirror_base, allow_unpinned)
    n_private = len(assets) - len(public)
    if include_private:
        missing = _check_private(assets, private_dir, dest)
        if missing:
            raise AssetError(f"{len(missing)} private file(s) missing from {private_dir}: {missing}")
    elif n_private:
        print(f"skip  {n_private} private entr{'y' if n_private == 1 else 'ies'} "
              "(--include-private --private-dir DIR checks files already provisioned there)")
    return 0


# --------------------------------------------------------------------------- verify

def verify_dir(manifest: Path, root: Path, require_private: bool = False,
               with_synthetic: bool = False) -> list[str]:
    """Return a list of problems (empty means the directory matches the manifest exactly).

    Public entries must be present. A private entry or a synthetic output is checked against
    its pin when the file is there, and is only required with require_private / with_synthetic
    (a synthetic file whose tool is missing on this host is never required)."""
    m, assets = load_manifest(manifest)
    problems: list[str] = []
    if not root.is_dir():
        return [f"{root} is not a directory"]
    names = {a["name"] for a in assets}
    for a in assets:
        private = a["source_kind"] == "private"
        p = safe_join(root, a["name"])
        present = not p.is_symlink() and p.is_file()
        if private and not p.is_symlink() and not p.exists():
            if require_private:
                problems.append(f"{a['name']}: private file missing")
            continue
        if a["sha256"] is None:
            problems.append(f"{a['name']}: not pinned in the manifest")
            continue
        if not present:
            problems.append(f"{a['name']}: missing (or not a regular file)")
            continue
        sha, n = sha256_file(p)
        if n != a["size"]:
            problems.append(f"{a['name']}: size {n} != manifest {a['size']}")
        elif sha != a["sha256"]:
            problems.append(f"{a['name']}: sha256 {sha} != manifest {a['sha256']}")
    for sy in synthetic_entries(m):
        names.add(sy["output"])
        p = safe_join(root, sy["output"])
        if p.is_symlink() or not p.is_file():
            if p.is_symlink() or p.exists():
                problems.append(f"{sy['output']}: not a regular file")
            elif with_synthetic and not synth_missing_tools(sy):
                problems.append(f"{sy['output']}: synthetic file missing (run synth --dest {root})")
            continue
        bad = synth_pin_problem(sy, p)
        if bad:
            problems.append(f"{sy['output']}: {bad}")
    for rel in scan_files(root):
        if rel in names or rel in GENERATED:
            continue
        problems.append(f"{rel}: not in the manifest")
    return problems


def cmd_verify(manifest: Path, root: Path, require_private: bool = False,
               with_synthetic: bool = False) -> int:
    problems = verify_dir(manifest, root, require_private, with_synthetic)
    for p in problems:
        print(f"FAIL {p}", file=sys.stderr)
    if problems:
        print(f"verify: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print(f"verify: ok ({len(load_manifest(manifest)[1])} assets)")
    return 0


# --------------------------------------------------------------------------- licences

SHARE_ALIKE_NOTICE = (
    "Files marked SHARE-ALIKE are under a licence that requires the same licence for adaptations.\n"
    "They must stay under it when redistributed or adapted, and derived outputs or evidence that\n"
    "contain renders of a share-alike asset (screenshots, frames, recordings, converted files) must\n"
    "not be published."
)


def licenses_text(assets: list[dict], synthetic: list[dict] | None = None) -> str:
    """The published licence list. Private entries never appear in it."""
    lines = [
        "LICENSES for the test asset bundle",
        "==================================",
        "",
        "Every file below keeps its own licence. This bundle only collects them.",
        "Attribution texts are reproduced as the sources require.",
        SHARE_ALIKE_NOTICE,
        "",
    ]
    for a in sorted(public_assets(assets), key=lambda x: x["name"]):
        sa = a["share_alike"]
        lines += [
            f"File:         {a['name']}" + ("   [SHARE-ALIKE]" if sa else ""),
            f"Licence:      {a['license']}",
            f"Licence URL:  {a['license_url']}",
            f"Attribution:  {a['attribution']}",
            f"Source page:  {a['source_page']}",
            "Share-alike:  " + ("YES (SHARE-ALIKE) - derivatives and redistribution must use the same "
                                "licence; derived outputs or evidence containing renders of this file "
                                "must not be published" if sa else "no"),
            f"Used for:     {a['purpose']}",
            "",
        ]
    for sy in sorted(synthetic or [], key=lambda x: x["output"]):
        lines += [
            f"File:         {sy['output']}",
            "Licence:      generated by build_test_assets.py (no third-party content)",
            f"Used for:     {sy['description']}",
            "",
        ]
    return "\n".join(lines)


def public_manifest(m: dict, with_synthetic: bool = False) -> dict:
    """The manifest copy that may be published: no private entry, synthetic only on request."""
    out = {k: v for k, v in m.items() if k not in ("assets", "synthetic")}
    out["assets"] = public_assets(m["assets"])
    if with_synthetic and m.get("synthetic"):
        out["synthetic"] = m["synthetic"]
    return out


def write_bundle_meta(manifest: Path, root: Path, with_synthetic: bool = False) -> None:
    m, assets = load_manifest(manifest)
    pub = public_manifest(m, with_synthetic)
    (root / "LICENSES.txt").write_text(licenses_text(assets, pub.get("synthetic")), encoding="utf-8")
    (root / "MANIFEST.json").write_text(json.dumps(pub, indent=2, ensure_ascii=False) + "\n",
                                        encoding="utf-8")


def cmd_licenses(manifest: Path, root: Path, with_synthetic: bool = False) -> int:
    if not root.is_dir():
        raise AssetError(f"{root} is not a directory")
    write_bundle_meta(manifest, root, with_synthetic)
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


# --------------------------------------------------------------------------- synthetic files
# Deterministic generated files for scale tests. The built-in generators are pure Python and
# use only integer arithmetic for anything that reaches the output (no libm, no zlib
# compression: PNG data is written as stored deflate blocks, whose bytes do not depend on the
# zlib build), so the bytes are the same on every machine and Python version.

OUTPUT_TOKEN = "{output}"
IDAT_SIZE = 1 << 20


def _png_chunk(tag: bytes, data: bytes) -> bytes:
    return (struct.pack(">I", len(data)) + tag + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))


class _StoredZlib:
    """A zlib stream of stored deflate blocks of fixed size; the output depends only on the input."""
    BLOCK = 65535

    def __init__(self, sink):
        self.sink = sink
        self.buf = bytearray()
        self.adler = 1
        self.started = False

    def _head(self) -> bytes:
        if self.started:
            return b""
        self.started = True
        return b"\x78\x01"

    def write(self, data: bytes) -> None:
        self.adler = zlib.adler32(data, self.adler)
        self.buf += data
        out = bytearray(self._head())
        pos = 0
        while len(self.buf) - pos >= self.BLOCK:
            out += b"\x00" + struct.pack("<HH", self.BLOCK, self.BLOCK ^ 0xFFFF)
            out += self.buf[pos:pos + self.BLOCK]
            pos += self.BLOCK
        del self.buf[:pos]
        if out:
            self.sink(bytes(out))

    def finish(self) -> None:
        n = len(self.buf)
        out = bytearray(self._head())
        out += b"\x01" + struct.pack("<HH", n, n ^ 0xFFFF) + self.buf
        out += struct.pack(">I", self.adler)
        self.buf = bytearray()
        self.sink(bytes(out))


def gen_png_gradient(path: Path, width: int = 7680, height: int = 4320) -> None:
    """8-bit RGB PNG: red = horizontal ramp, green = vertical ramp, blue = hash tiles."""
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(_png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)))
        pending = bytearray()

        def sink(b: bytes) -> None:
            pending.extend(b)
            while len(pending) >= IDAT_SIZE:
                f.write(_png_chunk(b"IDAT", bytes(pending[:IDAT_SIZE])))
                del pending[:IDAT_SIZE]
        z = _StoredZlib(sink)
        red = bytes((x * 255) // max(1, width - 1) for x in range(width))
        tile = b""
        for y in range(height):
            if y % 16 == 0:
                tile = hashlib.sha256(b"kf-png-gradient:%d" % (y // 16)).digest()
                tile = tile * (width // 32 + 3)
            off = (y % 16) * 2
            row = bytearray(1 + 3 * width)  # filter type 0
            row[1::3] = red
            row[2::3] = bytes([(y * 255) // max(1, height - 1)]) * width
            row[3::3] = tile[off:off + width]
            z.write(bytes(row))
        z.finish()
        if pending:
            f.write(_png_chunk(b"IDAT", bytes(pending)))
        f.write(_png_chunk(b"IEND", b""))


def gen_tiff_16bit(path: Path, width: int = 4096, height: int = 4096) -> None:
    """Uncompressed little-endian baseline TIFF, 16-bit RGB, one strip."""
    assert array.array("H").itemsize == 2
    strip = width * height * 6
    tags = [  # (tag, type, count, value) ; type 3 SHORT, 4 LONG
        (256, 4, 1, width), (257, 4, 1, height), (258, 3, 3, None), (259, 3, 1, 1),
        (262, 3, 1, 2), (273, 4, 1, None), (277, 3, 1, 3), (278, 4, 1, height),
        (279, 4, 1, strip), (284, 3, 1, 1),
    ]
    ifd_off = 8
    bps_off = ifd_off + 2 + 12 * len(tags) + 4
    data_off = bps_off + 6
    ifd = struct.pack("<H", len(tags))
    for tag, typ, cnt, val in tags:
        if tag == 258:
            ifd += struct.pack("<HHII", tag, typ, cnt, bps_off)
        elif tag == 273:
            ifd += struct.pack("<HHII", tag, typ, cnt, data_off)
        else:
            ifd += struct.pack("<HHII", tag, typ, cnt, val)
    ifd += struct.pack("<I", 0)
    with open(path, "wb") as f:
        f.write(b"II" + struct.pack("<HI", 42, ifd_off) + ifd + struct.pack("<HHH", 16, 16, 16))
        rx = array.array("H", [(x * 65535) // max(1, width - 1) for x in range(width)])
        tile = array.array("H")
        for y in range(height):
            if y % 16 == 0:
                d = hashlib.sha256(b"kf-tiff-16bit:%d" % (y // 16)).digest()
                tile = array.array("H", struct.unpack("<16H", d)) * (width // 16 + 3)
            row = array.array("H", bytes(6 * width))
            row[0::3] = rx
            row[1::3] = array.array("H", [(y * 65535) // max(1, height - 1)]) * width
            row[2::3] = tile[y % 16:y % 16 + width]
            if sys.byteorder == "big":
                row.byteswap()
            f.write(row.tobytes())


def _sine_table() -> list[int]:
    """1025 entries of sin(2*pi*k/1024) * 2^24, from integer Taylor series (no libm)."""
    sc = 1 << 64
    pi = 314159265358979323846264338327950288 * sc // 10 ** 35

    def div0(a: int, b: int) -> int:  # division truncating toward zero
        return a // b if a >= 0 else -((-a) // b)

    def sin_fx(x: int) -> int:
        x2 = div0(x * x, sc)
        term = total = x
        n = 1
        while term:
            term = -div0(div0(term * x2, sc), (2 * n) * (2 * n + 1))
            total += term
            n += 1
        return total
    quarter = [(sin_fx(pi * k // 512) + (1 << 39)) >> 40 for k in range(257)]
    t = []
    for k in range(1025):
        if k <= 256:
            t.append(quarter[k])
        elif k <= 512:
            t.append(quarter[512 - k])
        elif k <= 768:
            t.append(-quarter[k - 512])
        else:
            t.append(-quarter[1024 - k])
    return t


def gen_wav_sine_sweep(path: Path, seconds: int = 30, rate: int = 48000,
                       f0: int = 20, f1: int = 20000) -> None:
    """16-bit stereo PCM WAV: left sweeps f0 -> f1, right sweeps f1 -> f0 (linear, half scale)."""
    t = _sine_table()
    n = seconds * rate
    amp = 16384
    span = (f1 - f0) << 16
    pl = pr = 0
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 36 + 4 * n) + b"WAVEfmt "
                + struct.pack("<IHHIIHH", 16, 1, 2, rate, rate * 4, 4, 16)
                + b"data" + struct.pack("<I", 4 * n))
        step = rate
        for start in range(0, n, step):
            out = array.array("h", bytes(4 * min(step, n - start)))
            j = 0
            for i in range(start, min(start + step, n)):
                pl = (pl + ((((f0 << 16) + span * i // n) << 16) // rate)) & 0xFFFFFFFF
                pr = (pr + ((((f1 << 16) - span * i // n) << 16) // rate)) & 0xFFFFFFFF
                a = pl >> 22
                b = pr >> 22
                out[j] = ((t[a] + (((t[a + 1] - t[a]) * ((pl >> 6) & 0xFFFF)) >> 16)) * amp) >> 24
                out[j + 1] = ((t[b] + (((t[b + 1] - t[b]) * ((pr >> 6) & 0xFFFF)) >> 16)) * amp) >> 24
                j += 2
            if sys.byteorder == "big":
                out.byteswap()
            f.write(out.tobytes())


BUILTIN_GENERATORS = {
    "png_gradient_8k": lambda p: gen_png_gradient(p),
    "tiff_16bit_big": lambda p: gen_tiff_16bit(p),
    "wav_sine_sweep": lambda p: gen_wav_sine_sweep(p),
}


def synth_missing_tools(sy: dict) -> list[str]:
    return [t for t in sy["requires"] if shutil.which(t) is None]


def synth_mode(sy: dict) -> str:
    if sy["sha256"] is not None:
        return "full"
    if sy["sha256_prefix"] is not None:
        return "prefix"
    return "unpinned"


def prefix_hash(path: Path, nbytes: int) -> str | None:
    """sha256 of the first nbytes of the file, or None when the file is shorter."""
    h = hashlib.sha256()
    left = nbytes
    with open(path, "rb") as f:
        while left:
            b = f.read(min(left, 1 << 20))
            if not b:
                return None
            h.update(b)
            left -= len(b)
    return h.hexdigest()


def synth_pin_problem(sy: dict, path: Path) -> str | None:
    mode = synth_mode(sy)
    if mode == "unpinned":
        return "not pinned in the manifest"
    if mode == "full":
        sha, n = sha256_file(path)
        if n != sy["size"]:
            return f"size {n} != manifest {sy['size']}"
        if sha != sy["sha256"]:
            return f"sha256 {sha} != manifest {sy['sha256']}"
        return None
    got = prefix_hash(path, sy["prefix_bytes"])
    if got is None:
        return f"file is shorter than the pinned prefix of {sy['prefix_bytes']} bytes"
    if got != sy["sha256_prefix"]:
        return f"sha256 of the first {sy['prefix_bytes']} bytes {got} != manifest {sy['sha256_prefix']}"
    return None


def generate_synth(sy: dict, target: Path) -> None:
    """Generate one synthetic file at target through a private temporary directory."""
    target.parent.mkdir(parents=True, exist_ok=True)
    tmpdir = Path(tempfile.mkdtemp(prefix=".synth-", dir=target.parent))
    try:
        tmp = tmpdir / target.name
        g = sy["generator"]
        if isinstance(g, str):
            BUILTIN_GENERATORS[g](tmp)
        else:
            argv = [str(tmp) if x == OUTPUT_TOKEN else x for x in g]
            r = subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, text=True)
            if r.returncode != 0 or not tmp.is_file():
                raise AssetError(f"{sy['name']}: {g[0]} failed (exit {r.returncode}): "
                                 f"{(r.stderr or '').strip()[-400:]}")
        os.replace(tmp, target)
    except OSError as e:
        raise AssetError(f"{sy['name']}: cannot generate: {e}") from e
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)


def cmd_synth(manifest: Path, dest: Path, only: list[str] | None = None,
              allow_unpinned: bool = False) -> int:
    m, _ = load_manifest(manifest)
    entries = synthetic_entries(m)
    if only:
        unknown = sorted(set(only) - {sy["name"] for sy in entries})
        if unknown:
            raise AssetError(f"unknown synthetic name(s) {unknown}")
        entries = [sy for sy in entries if sy["name"] in only]
    if not entries:
        print("synth: the manifest has no synthetic entries")
        return 0
    dest.mkdir(parents=True, exist_ok=True)
    failed = 0
    for sy in entries:
        target = safe_join(dest, sy["output"])
        missing = synth_missing_tools(sy)
        if missing:
            print(f"skip  {sy['name']}: needs {', '.join(missing)} on PATH, which is not installed; "
                  "not generated (this is not a failure)")
            continue
        if target.is_symlink():
            raise AssetError(f"{sy['output']}: destination is a symlink")
        if target.is_file() and synth_pin_problem(sy, target) is None:
            print(f"ok    {sy['output']} (already present, pin verified)")
            continue
        print(f"synth {sy['output']}")
        generate_synth(sy, target)
        problem = synth_pin_problem(sy, target)
        if problem is None:
            print(f"      ok ({target.stat().st_size} bytes, {synth_mode(sy)} pin verified)")
        elif synth_mode(sy) == "unpinned" and allow_unpinned:
            print(f"      unpinned; generated {target.stat().st_size} bytes (run pin-synth --dir {dest})")
        else:
            print(f"FAIL  {sy['output']}: {problem}", file=sys.stderr)
            _rm(target)
            failed += 1
    return 1 if failed else 0


def cmd_pin_synth(manifest: Path, root: Path) -> int:
    m, _ = load_manifest(manifest)
    errors: list[str] = []
    updates = []
    for sy in synthetic_entries(m):
        p = safe_join(root, sy["output"])
        if p.is_symlink() or not p.is_file():
            if synth_mode(sy) == "unpinned":
                if synth_missing_tools(sy):
                    print(f"pin-synth: {sy['name']}: still unpinned (needs {sy['requires']} to generate it)")
                else:
                    errors.append(f"{sy['name']}: unpinned and not present in {root} (run synth first)")
            continue
        if synth_mode(sy) == "unpinned":
            if isinstance(sy["generator"], str):
                sha, n = sha256_file(p)
                updates.append((sy, {"sha256": sha, "size": n}))
            else:
                nb = min(PREFIX_BYTES_DEFAULT, p.stat().st_size)
                updates.append((sy, {"sha256_prefix": prefix_hash(p, nb), "prefix_bytes": nb}))
        else:
            problem = synth_pin_problem(sy, p)
            if problem:
                errors.append(f"{sy['name']}: {problem}; refusing to overwrite a pin")
    if errors:
        for e in errors:
            print(f"pin-synth: {e}", file=sys.stderr)
        return 1
    for sy, fields in updates:
        sy.update(fields)
        print(f"pinned {sy['name']} {fields}")
    validate_manifest(m)
    if updates:
        write_manifest(manifest, m)
    print(f"pin-synth: {len(updates)} newly pinned")
    return 0


# --------------------------------------------------------------------------- ISO (tool)

def find_iso_tool() -> list[str] | None:
    for tool, pre in (("genisoimage", []), ("mkisofs", []), ("xorriso", ["-as", "mkisofs"])):
        p = shutil.which(tool)
        if p:
            return [p] + pre
    return None


def bundle_entries(manifest: Path, root: Path, with_synthetic: bool = False) -> list[tuple[str, Path]]:
    """(iso path, real path) for the public assets plus the generated files, after a full verify.

    Private entries never enter the image, even when their file sits in the directory;
    synthetic files only with with_synthetic."""
    problems = verify_dir(manifest, root, with_synthetic=with_synthetic)
    if problems:
        raise AssetError("refusing to build an ISO from an unverified directory:\n  " + "\n  ".join(problems))
    write_bundle_meta(manifest, root, with_synthetic)
    m, assets = load_manifest(manifest)
    out = [(a["name"], safe_join(root, a["name"])) for a in public_assets(assets)]
    if with_synthetic:
        out += [(sy["output"], safe_join(root, sy["output"])) for sy in synthetic_entries(m)
                if safe_join(root, sy["output"]).is_file()]
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


def cmd_iso(manifest: Path, root: Path, out: Path, engine: str = "auto",
            with_synthetic: bool = False) -> int:
    _check_out(root, out)
    entries = bundle_entries(manifest, root, with_synthetic)
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
    syn = dict(action="store_true", help="also handle the synthetic generated files "
                                         "(default: not in the bundle; they are generated where used)")
    s = sub.add_parser("fetch", help="download every public asset and check it against the manifest")
    s.add_argument("--dest", type=Path, required=True)
    s.add_argument("--allow-unpinned", action="store_true",
                   help="maintainer: download entries whose sha256/size are still null (then run pin)")
    s.add_argument("--mirror-base", metavar="URL",
                   help="https base URL of the public mirror (for example https://bundles.kayfabe.dev): "
                        "a public-mirror entry whose origin fails is fetched from BASE/<mirror_path>; "
                        "the pin is checked either way")
    s.add_argument("--include-private", action="store_true",
                   help="also check the private entries, which must already be in --private-dir "
                        "(they are never downloaded and never copied into --dest)")
    s.add_argument("--private-dir", type=Path, help="directory that holds the private files")
    s = sub.add_parser("verify", help="check hashes, sizes and that no other file is present")
    s.add_argument("--dir", type=Path, required=True)
    s.add_argument("--require-private", action="store_true",
                   help="fail when a private file is absent (present ones are always checked)")
    s.add_argument("--with-synthetic", **syn)
    s = sub.add_parser("licenses", help="write LICENSES.txt (and MANIFEST.json) of the public entries")
    s.add_argument("--dir", type=Path, required=True)
    s.add_argument("--with-synthetic", **syn)
    s = sub.add_parser("iso", help="build the read-only ISO (label KFDATA) of the public entries")
    s.add_argument("--dir", type=Path, required=True)
    s.add_argument("--out", type=Path, required=True)
    s.add_argument("--engine", choices=("auto", "tool", "python"), default="auto")
    s.add_argument("--with-synthetic", **syn)
    s = sub.add_parser("all", help="fetch, (synth), verify, licenses, iso; public entries only")
    s.add_argument("--dest", type=Path, required=True)
    s.add_argument("--iso", type=Path, required=True)
    s.add_argument("--engine", choices=("auto", "tool", "python"), default="auto")
    s.add_argument("--mirror-base", metavar="URL", help="see fetch --mirror-base")
    s.add_argument("--with-synthetic", **syn)
    s = sub.add_parser("pin", help="maintainer: fill null sha256/size from downloaded files")
    s.add_argument("--dir", type=Path, required=True)
    s = sub.add_parser("synth", help="generate the synthetic files and verify their pins")
    s.add_argument("--dest", type=Path, required=True)
    s.add_argument("--only", action="append", metavar="NAME", help="generate only this entry (repeatable)")
    s.add_argument("--allow-unpinned", action="store_true",
                   help="maintainer: generate entries whose pin is still null (then run pin-synth)")
    s = sub.add_parser("pin-synth", help="maintainer: fill null synthetic pins from generated files")
    s.add_argument("--dir", type=Path, required=True)
    return p


def main(argv: list[str] | None = None) -> int:
    a = build_parser().parse_args(argv)
    try:
        if a.cmd == "fetch":
            return cmd_fetch(a.manifest, a.dest, a.allow_unpinned, mirror_base=a.mirror_base,
                             include_private=a.include_private, private_dir=a.private_dir)
        if a.cmd == "verify":
            return cmd_verify(a.manifest, a.dir, a.require_private, a.with_synthetic)
        if a.cmd == "licenses":
            return cmd_licenses(a.manifest, a.dir, a.with_synthetic)
        if a.cmd == "iso":
            return cmd_iso(a.manifest, a.dir, a.out, a.engine, a.with_synthetic)
        if a.cmd == "pin":
            return cmd_pin(a.manifest, a.dir)
        if a.cmd == "synth":
            return cmd_synth(a.manifest, a.dest, a.only, a.allow_unpinned)
        if a.cmd == "pin-synth":
            return cmd_pin_synth(a.manifest, a.dir)
        if a.cmd == "all":
            steps = [lambda: cmd_fetch(a.manifest, a.dest, mirror_base=a.mirror_base)]
            if a.with_synthetic:
                steps.append(lambda: cmd_synth(a.manifest, a.dest))
            steps += [lambda: cmd_verify(a.manifest, a.dest, with_synthetic=a.with_synthetic),
                      lambda: cmd_licenses(a.manifest, a.dest, a.with_synthetic),
                      lambda: cmd_iso(a.manifest, a.dest, a.iso, a.engine, a.with_synthetic)]
            for step in steps:
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
