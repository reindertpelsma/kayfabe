#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Validate optional offline, source-built release assets; never downloads or runs a sample."""
import hashlib
import json
from pathlib import Path, PurePosixPath
import sys

HERE = Path(__file__).resolve().parent


def assets(root, cfg=None):
    root = Path(root)
    cfg = cfg or json.loads((HERE / "release_software.json").read_text())
    receipt = json.loads((root / "receipt.json").read_text(encoding="utf-8-sig"))
    source = cfg["directx"]
    if (receipt.get("schema") != 1 or receipt.get("source_commit") != source["commit"]
            or receipt.get("source_repository") != source["repository"]
            or receipt.get("built_projects") != source["projects"]
            or receipt.get("patches") != source["patches"]):
        raise ValueError("release asset source/project pin mismatch")
    files = receipt.get("files")
    if not isinstance(files, dict) or not files or not set(source["required_files"]) <= set(files):
        raise ValueError("release assets missing required binaries, shaders, data or licence")
    if files["directx/GATE_VRS.patch"] != source["patches"]["vrs_gate"]:
        raise ValueError("VRS patch bytes do not match the source pin")
    for name in files:
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts or "\\" in name or str(path) != name or name == "receipt.json":
            raise ValueError("invalid release asset path")
    seen = set()
    for p in root.rglob("*"):
        if p.is_symlink():
            raise ValueError("release asset links are forbidden")
        if p.is_file():
            seen.add(p.relative_to(root).as_posix())
    if seen != set(files) | {"receipt.json"}:
        raise ValueError("release asset receipt does not exactly match files")
    for name, expected in files.items():
        with (root / name).open("rb") as f:
            if hashlib.file_digest(f, "sha256").hexdigest() != expected:
                raise ValueError("release asset hash mismatch: " + name)
    return sorted(seen)


def main(argv):
    try:
        names = assets(argv[0])
    except (OSError, ValueError, TypeError) as e:
        print("RELEASE_ASSETS_REFUSED", e)
        return 1
    print("RELEASE_ASSETS_CHECKED files=%d (source/bytes only; no hardware qualification)" % len(names))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
