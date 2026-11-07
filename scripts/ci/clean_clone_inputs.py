#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Preflight documented Rust builds; cargo compilation/tests remain the runtime oracle.

Only literal include paths and exact relocated trace literals are checked statically.
Constructed paths are exercised by cargo test, not claimed to be statically proved.
"""

from __future__ import annotations

import argparse
import csv
from pathlib import Path
import re
import sys

REQUIRED = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "firmware/kf-gop/Cargo.toml",
    "firmware/kf-gop/Cargo.lock",
)
DOCS = ("README.md", "docs/howto/building.md")
INCLUDE = re.compile(r'include_(?:str|bytes)!\s*\(\s*"([^"\n]+)"\s*\)')
TRACE = re.compile(r'"(?:\.\./)*(traces/[^"\n]+)"')


def validate(root: Path) -> list[str]:
    errors = []
    for rel in REQUIRED:
        if not (root / rel).is_file():
            errors.append(f"required build input absent from clean tree: {rel}")
    for rel in DOCS:
        path = root / rel
        if not path.is_file():
            errors.append(f"documented steps missing: {rel}")
            continue
        text = " ".join(path.read_text().split())
        if "cargo build --workspace --release" not in text:
            errors.append(
                f"{rel}: documented build changed/missing; review clean_clone_check.sh commands"
            )
        if "cargo test" not in text:
            errors.append(
                f"{rel}: documented test changed/missing; review clean_clone_check.sh commands"
            )
    moved = set()
    for manifest in (root / "traces").glob("*/MANIFEST.tsv"):
        with manifest.open() as stream:
            for row in csv.DictReader(stream, delimiter="\t"):
                # Archive manifests use 'path'; accept only actual rows, never fetch.
                if row.get("path"):
                    moved.add(row["path"])
    for directory in ("crates", "firmware"):
        for source in (root / directory).rglob("*.rs"):
            if "target" in source.relative_to(root).parts:
                continue
            # Ignore line comments; this is deliberately lexical, not a Rust parser.
            text = "\n".join(
                line.split("//", 1)[0] for line in source.read_text().splitlines()
            )
            for match in INCLUDE.finditer(text):
                path = source.parent / match[1]
                if not path.is_file():
                    errors.append(
                        f"{source.relative_to(root)}: literal build/test input absent: {match[1]}"
                    )
            for match in TRACE.finditer(text):
                rel = match[1]
                if rel in moved and not (root / rel).is_file():
                    errors.append(
                        f"{source.relative_to(root)}: archived input {rel} must remain in the tree for build/test; see traces/EVIDENCE_ARCHIVE.md"
                    )
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    errors = validate(args.root)
    for error in errors:
        print(f"clean-clone preflight: {error}", file=sys.stderr)
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
