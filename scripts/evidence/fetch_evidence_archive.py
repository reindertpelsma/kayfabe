#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Download relocated trace evidence, verify it, and put the files back where citations name them.

STATUS: LIVE, 2026-11-07. See traces/EVIDENCE_ARCHIVE.md.

    fetch_evidence_archive.py windows_matrix_20261011        # one directory
    fetch_evidence_archive.py --all                          # every relocated directory
    fetch_evidence_archive.py --list                         # what is relocated (no network)
    fetch_evidence_archive.py --base-url URL --cache DIR ... # other mirror, other download dir

Each archive is fetched from `<base-url>/<dir>.tar.zst` (default https://logs.kayfabe.dev/evidence),
its size and sha256 are compared with `traces/EVIDENCE_ARCHIVE.tsv`, every member is compared with
`traces/<dir>/MANIFEST.tsv` (the member set must be equal), and only then are the files written
below `--dest` (default: the repository root), never over a file that differs (use --force).
Equivalent by hand: `curl -O <url>` then `echo "<sha256>  <dir>.tar.zst" | sha256sum -c`.
Standard library only; Python before 3.14 also needs the `zstd` command.
"""

import argparse
import hashlib
import importlib.util
import os
import sys
import tarfile
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_BASE = "https://logs.kayfabe.dev/evidence"


def _lib():
    spec = importlib.util.spec_from_file_location("evidence_archive", HERE / "evidence_archive.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ea = _lib()


def download(url, dest, expect_bytes):
    tmp = dest.with_name(dest.name + ".part")
    req = urllib.request.Request(url, headers={"User-Agent": "kayfabe-fetch-evidence-archive/1"})
    with urllib.request.urlopen(req, timeout=120) as resp, open(tmp, "wb") as out:
        while True:
            chunk = resp.read(1 << 20)
            if not chunk:
                break
            out.write(chunk)
            if out.tell() > expect_bytes:
                raise ea.EvidenceError(f"{url}: longer than the {expect_bytes} bytes the index records")
    os.replace(tmp, dest)


def fetch_one(root, d, info, args):
    cache = Path(args.cache)
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / f"{d}.tar.zst"
    if not (archive.is_file() and archive.stat().st_size == info["bytes"] and ea.sha256_file(archive) == info["sha256"]):
        download(f"{args.base_url.rstrip('/')}/{d}.tar.zst", archive, info["bytes"])
    if archive.stat().st_size != info["bytes"] or ea.sha256_file(archive) != info["sha256"]:
        archive.unlink()
        raise ea.EvidenceError(f"{d}: downloaded archive does not match traces/EVIDENCE_ARCHIVE.tsv (deleted)")
    manifest = root / "traces" / d / "MANIFEST.tsv"
    ea.verify(archive, manifest)
    rows = ea.read_manifest(manifest)
    dest = Path(args.dest).resolve()
    written = skipped = 0
    stream = ea.zstd_stream(archive)
    try:
        with tarfile.open(fileobj=stream, mode="r|") as tar:
            for m in tar:
                ea.safe_member(m.name)
                target = dest / m.name
                size, sha = rows[m.name]
                data = tar.extractfile(m).read()
                if target.exists():
                    if hashlib.sha256(data).hexdigest() == sha and target.stat().st_size == size:
                        skipped += 1
                        continue
                    if not args.force:
                        raise ea.EvidenceError(f"{m.name} exists and differs; use --force")
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
                os.chmod(target, 0o644)
                written += 1
    finally:
        stream.close()
    print(f"{d}: verified, {written} files written, {skipped} already present")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("dirs", nargs="*", help="traces directory names (as in traces/<dir>)")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--base-url", default=DEFAULT_BASE)
    ap.add_argument("--cache", default=str(Path(os.environ.get("TMPDIR", "/tmp")) / "kf-evidence-archive"))
    ap.add_argument("--dest", default=str(ROOT))
    ap.add_argument("--root", default=str(ROOT), help="where traces/EVIDENCE_ARCHIVE.tsv and the manifests are")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args(argv)
    root = Path(args.root).resolve()
    try:
        index = ea.load_index(root)
        if args.list:
            for d, i in sorted(index.items()):
                print(f"{d}\t{i['members']} files\t{i['member_bytes']} bytes\tarchive {i['bytes']} bytes\t{i['sha256']}")
            return 0
        names = sorted(index) if args.all else args.dirs
        if not names:
            ap.error("name a directory or pass --all (--list shows them)")
        for d in names:
            if d not in index:
                raise ea.EvidenceError(f"{d}: not a relocated directory (see --list)")
            fetch_one(root, d, index[d], args)
    except (ea.EvidenceError, OSError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
