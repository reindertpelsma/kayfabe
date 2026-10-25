"""Capture streamed guest metadata and host logs as private data, without guest disk reads."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile

ALLOWED = {".log", ".txt", ".json", ".jsonl", ".out", ".rc", ".tsv", ".csv", ".sha256"}


def verify_pulled(receipts, root):
    if not receipts:
        raise ValueError("no inherited pull receipts")
    for row in receipts:
        if row["failed"] or row["not_pulled"] or row["verified"] != len(row["files"]):
            raise ValueError("inherited private capture incomplete")
        for name, item in row["files"].items():
            path = Path(root) / row["run"] / name
            with path.open("rb") as stream:
                actual = hashlib.file_digest(stream, "sha256").hexdigest()
            if actual != item["sha256"]:
                raise ValueError("private capture changed before destroy")
    return True


def pack(roots, target, *, max_bytes=500_000_000, max_files=2000):
    files = []
    for root, prefix in roots:
        root = Path(root)
        if not root.is_dir():
            continue
        for path in sorted(root.rglob("*")):
            if path.is_file() and not path.is_symlink() and path.suffix in ALLOWED:
                files.append((path, prefix + "/" + str(path.relative_to(root))))
    if len(files) > max_files or sum(p.stat().st_size for p, _ in files) > max_bytes:
        raise ValueError("private capture size/file bound exceeded")
    with tarfile.open(target, "w:gz") as archive:
        for path, name in files:
            archive.add(path, arcname=name, recursive=False)
    path = Path(target)
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return dict(sha256=digest, bytes=path.stat().st_size, files=len(files))


def verify_archive(path, expected):
    with Path(path).open("rb") as stream:
        actual = hashlib.file_digest(stream, "sha256").hexdigest()
    if actual != expected:
        raise ValueError("capture SHA256 mismatch")
    with tarfile.open(path, "r:gz") as archive:
        members = archive.getmembers()
        if len(members) > 2000 or sum(m.size for m in members) > 500_000_000:
            raise ValueError("private capture size/file bound exceeded")
        for member in members:
            name = PurePosixPath(member.name)
            if not member.isfile() or name.is_absolute() or ".." in name.parts:
                raise ValueError("unsafe archive member")
            if not name.parts or name.parts[0] not in ("evidence", "vm_logs", "bench"):
                raise ValueError("unexpected capture root")
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    print(json.dumps(pack(config["roots"], config["target"])))


if __name__ == "__main__":
    main()
