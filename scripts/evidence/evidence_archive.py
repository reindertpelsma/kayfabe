#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Build, verify and look up the relocated trace-evidence archives.

STATUS: LIVE, 2026-11-07. Context: traces/EVIDENCE_ARCHIVE.md (what moved and why), the
per-directory `traces/<dir>/MANIFEST.tsv` and `MOVED.md`.

A relocated trace directory is one `<dir>.tar.zst` whose members are repository-relative paths
(`traces/<dir>/...`), so extracting at the repository root puts every file back where the
citations name it. `MANIFEST.tsv` (header `path<TAB>bytes<TAB>sha256`) lists every member, sorted
by path. The archive is deterministic: members sorted by name, mtime 0, uid/gid 0, empty
owner names, mode 0644, GNU tar format, `zstd -19 --long` single-threaded. Building the same
member bytes twice gives the same archive sha256.

Subcommands
  build  --list FILE --out DIR    FILE holds one repository-relative path per line (all under
                                  traces/<dir>/); writes DIR/<dir>.tar.zst per directory, DIR/SHA256SUMS,
                                  `traces/<dir>/MANIFEST.tsv` and the index `traces/EVIDENCE_ARCHIVE.tsv`.
  verify ARCHIVE MANIFEST         stream the archive and compare every member (bytes, sha256) with the
                                  manifest; the member set must equal the manifest set exactly.
  where  PATH                     say whether a traces path is in the tree, or only in a manifest (and
                                  then in which archive).
  check-tree                      every MANIFEST.tsv lists files that are either absent from the tree
                                  (relocated) or present with the listed bytes and sha256; every
                                  directory with a MANIFEST.tsv has a MOVED.md; the index agrees.

Only the Python standard library and, for Python before 3.14, the `zstd` command are used.
"""

import argparse
import hashlib
import io
import os
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INDEX = "traces/EVIDENCE_ARCHIVE.tsv"
INDEX_HEADER = "dir\tarchive_bytes\tarchive_sha256\tmembers\tmember_bytes"
MANIFEST_HEADER = "path\tbytes\tsha256"
MTIME = 0


class EvidenceError(Exception):
    pass


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def top_dir(path):
    parts = path.split("/")
    if len(parts) < 3 or parts[0] != "traces":
        raise EvidenceError(f"not a file inside a traces directory: {path}")
    return parts[1]


def safe_member(name):
    parts = name.split("/")
    if name.startswith("/") or ".." in parts or "" in parts or "\t" in name or "\n" in name:
        raise EvidenceError(f"unsafe member name {name!r}")
    if parts[0] != "traces":
        raise EvidenceError(f"member outside traces/: {name!r}")


def read_manifest(path):
    lines = Path(path).read_text(encoding="utf-8").split("\n")
    if lines[-1] == "":
        lines.pop()
    if not lines or lines[0] != MANIFEST_HEADER:
        raise EvidenceError(f"{path}: bad header")
    rows = {}
    for n, line in enumerate(lines[1:], 2):
        f = line.split("\t")
        if len(f) != 3 or not f[1].isdigit() or len(f[2]) != 64:
            raise EvidenceError(f"{path}:{n}: malformed row")
        safe_member(f[0])
        if f[0] in rows:
            raise EvidenceError(f"{path}:{n}: duplicate path {f[0]}")
        rows[f[0]] = (int(f[1]), f[2])
    if list(rows) != sorted(rows):
        raise EvidenceError(f"{path}: rows are not sorted by path")
    return rows


def zstd_compress(data_path, out_path):
    """`zstd -19 --long -T1`: the exact command is part of the determinism claim."""
    subprocess.run(
        ["zstd", "-q", "-f", "-19", "--long", "-T1", "-o", str(out_path), str(data_path)],
        check=True,
    )


def zstd_stream(path):
    """A file object over the decompressed bytes of a .zst file."""
    try:
        from compression import zstd  # Python 3.14+

        return zstd.open(path, "rb")
    except ImportError:
        proc = subprocess.Popen(["zstd", "-dc", "--long=31", str(path)], stdout=subprocess.PIPE)
        return _ProcStream(proc)


class _ProcStream(io.RawIOBase):
    def __init__(self, proc):
        self.proc = proc

    def readable(self):
        return True

    def readinto(self, b):
        data = self.proc.stdout.read(len(b))
        b[: len(data)] = data
        return len(data)

    def close(self):
        if self.proc.stdout:
            self.proc.stdout.close()
        rc = self.proc.wait()
        super().close()
        if rc not in (0, None):
            raise EvidenceError(f"zstd -dc exited {rc}")


def build_tar(members, tar_path, root):
    """Deterministic tar of `members` (sorted repository-relative names)."""
    with open(tar_path, "wb") as out:
        with tarfile.open(fileobj=out, mode="w", format=tarfile.GNU_FORMAT) as tar:
            for name in members:
                src = root / name
                info = tarfile.TarInfo(name)
                info.size = src.stat().st_size
                info.mtime = MTIME
                info.mode = 0o644
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                with open(src, "rb") as fh:
                    tar.addfile(info, fh)


def cmd_build(args):
    root = Path(args.root).resolve()
    names = [x for x in Path(args.list).read_text(encoding="utf-8").split("\n") if x]
    if len(set(names)) != len(names):
        raise EvidenceError("duplicate names in the list")
    by_dir = {}
    for n in names:
        safe_member(n)
        by_dir.setdefault(top_dir(n), []).append(n)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    index_rows, sums = [], []
    for d in sorted(by_dir):
        members = sorted(by_dir[d])
        rows = []
        for m in members:
            p = root / m
            if p.is_symlink() or not p.is_file():
                raise EvidenceError(f"not a regular file: {m}")
            rows.append((m, p.stat().st_size, sha256_file(p)))
        manifest = "\n".join([MANIFEST_HEADER] + [f"{a}\t{b}\t{c}" for a, b, c in rows]) + "\n"
        (root / "traces" / d / "MANIFEST.tsv").write_text(manifest, encoding="utf-8")
        tar_path = out / f"{d}.tar"
        build_tar(members, tar_path, root)
        zst = out / f"{d}.tar.zst"
        zstd_compress(tar_path, zst)
        tar_path.unlink()
        sha = sha256_file(zst)
        verify(zst, root / "traces" / d / "MANIFEST.tsv")
        index_rows.append((d, zst.stat().st_size, sha, len(rows), sum(r[1] for r in rows)))
        sums.append(f"{sha}  {d}.tar.zst")
        print(f"{d}: {len(rows)} files, {zst.stat().st_size} bytes, {sha}", file=sys.stderr)
    (out / "SHA256SUMS").write_text("\n".join(sums) + "\n", encoding="utf-8")
    (root / INDEX).write_text(
        "\n".join([INDEX_HEADER] + ["\t".join(map(str, r)) for r in index_rows]) + "\n",
        encoding="utf-8",
    )


def verify(archive, manifest):
    """Raise EvidenceError unless the archive's members equal the manifest exactly."""
    want = read_manifest(manifest)
    seen = set()
    stream = zstd_stream(archive)
    try:
        with tarfile.open(fileobj=stream, mode="r|") as tar:
            for m in tar:
                if not m.isfile():
                    raise EvidenceError(f"{archive}: member {m.name!r} is not a regular file")
                safe_member(m.name)
                if m.name in seen:
                    raise EvidenceError(f"{archive}: duplicate member {m.name}")
                if m.name not in want:
                    raise EvidenceError(f"{archive}: member {m.name} is not in the manifest")
                h, n = hashlib.sha256(), 0
                fh = tar.extractfile(m)
                for chunk in iter(lambda: fh.read(1 << 20), b""):
                    h.update(chunk)
                    n += len(chunk)
                size, sha = want[m.name]
                if n != size or h.hexdigest() != sha:
                    raise EvidenceError(f"{archive}: {m.name} differs from the manifest")
                seen.add(m.name)
    finally:
        stream.close()
    missing = sorted(set(want) - seen)
    if missing:
        raise EvidenceError(f"{archive}: {len(missing)} manifest rows have no member, first {missing[0]}")
    return len(seen)


def cmd_verify(args):
    n = verify(args.archive, args.manifest)
    print(f"ok: {n} members match {args.manifest}")


def load_index(root):
    p = root / INDEX
    lines = p.read_text(encoding="utf-8").split("\n")
    if lines[-1] == "":
        lines.pop()
    if not lines or lines[0] != INDEX_HEADER:
        raise EvidenceError(f"{p}: bad header")
    rows = {}
    for line in lines[1:]:
        f = line.split("\t")
        if len(f) != 5:
            raise EvidenceError(f"{p}: malformed row {line!r}")
        rows[f[0]] = {"bytes": int(f[1]), "sha256": f[2], "members": int(f[3]), "member_bytes": int(f[4])}
    return rows


def manifests(root):
    return sorted((root / "traces").glob("*/MANIFEST.tsv"))


def cmd_where(args):
    root = Path(args.root).resolve()
    path = args.path.strip().rstrip("/")
    if (root / path).exists():
        print(f"in tree: {path}")
        return
    try:
        d = path.split("/")[1]
        rows = read_manifest(root / "traces" / d / "MANIFEST.tsv")
    except (IndexError, FileNotFoundError):
        print(f"not in tree and in no manifest: {path}")
        sys.exit(1)
    if path in rows:
        size, sha = rows[path]
        print(f"relocated: {path} {size} bytes sha256 {sha} in {d}.tar.zst (scripts/evidence/fetch_evidence_archive.py {d})")
        return
    under = [p for p in rows if p.startswith(path + "/")]
    if under:
        print(f"directory with {len(under)} relocated files in {d}.tar.zst")
        return
    print(f"not in tree and not in the manifest of {d}: {path}")
    sys.exit(1)


def cmd_check_tree(args):
    root = Path(args.root).resolve()
    index = load_index(root)
    problems = []
    seen_dirs = set()
    for mf in manifests(root):
        d = mf.parent.name
        seen_dirs.add(d)
        moved_md = mf.parent / "MOVED.md"
        if not moved_md.is_file():
            problems.append(f"{d}: MANIFEST.tsv without MOVED.md")
        if d not in index:
            problems.append(f"{d}: not in {INDEX}")
            continue
        rows = read_manifest(mf)
        if len(rows) != index[d]["members"] or sum(r[0] for r in rows.values()) != index[d]["member_bytes"]:
            problems.append(f"{d}: index counts differ from the manifest")
        moved = moved_md.read_text(encoding="utf-8") if moved_md.is_file() else ""
        if moved_md.is_file() and index[d]["sha256"] not in moved:
            problems.append(f"{d}: MOVED.md does not name the archive sha256")
        for path, (size, sha) in rows.items():
            p = root / path
            if p.exists() and (p.stat().st_size != size or sha256_file(p) != sha):
                problems.append(f"{path}: in the tree but differs from the manifest")
    for d in sorted(set(index) - seen_dirs):
        problems.append(f"{d}: in {INDEX} but has no MANIFEST.tsv")
    if problems:
        print("\n".join(problems))
        sys.exit(1)
    print(f"ok: {len(seen_dirs)} manifests agree with the tree and {INDEX}")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--root", default=str(ROOT))
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--list", required=True)
    b.add_argument("--out", required=True)
    v = sub.add_parser("verify")
    v.add_argument("archive")
    v.add_argument("manifest")
    w = sub.add_parser("where")
    w.add_argument("path")
    sub.add_parser("check-tree")
    args = ap.parse_args(argv)
    try:
        {"build": cmd_build, "verify": cmd_verify, "where": cmd_where, "check-tree": cmd_check_tree}[
            args.cmd
        ](args)
    except EvidenceError as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
