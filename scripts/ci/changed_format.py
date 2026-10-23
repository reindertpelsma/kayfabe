#!/usr/bin/env python3
"""Check changed C/H/CUDA lines with pinned clang-format 18; never apply a patch."""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import subprocess
import sys

from linelen import ROOT, generated, git, path_exemption

TREES = ("broker/", "cuda/", "qemu/hw/misc/kf3/")
EXTENSIONS = {".c", ".h", ".cu", ".cuh"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--base", default="origin/master")
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args()
    try:
        root = args.root.resolve()
        base = git(root, "merge-base", args.base, args.head).decode().strip()
        names = git(
            root,
            "diff",
            "--no-color",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=AM",
            base,
            args.head,
        )
        paths = []
        for name in filter(None, names.split(b"\0")):
            path = name.decode("utf-8")
            if (
                not path.startswith(TREES)
                or Path(path).suffix.lower() not in EXTENSIONS
                or path_exemption(path)
            ):
                continue
            entry = git(root, "ls-tree", "-z", args.head, "--", path)
            if not entry.startswith((b"100644 ", b"100755 ")):
                continue
            content = git(root, "show", f"{args.head}:{path}").decode("utf-8-sig")
            if not generated(content):
                paths.append(path)
        if not paths:
            print("changed-format: files=0 (no changed C/H/CUDA source)")
            return 0
        version = subprocess.run(
            ["clang-format-18", "--version"], capture_output=True, text=True, check=True
        )
        if not re.search(r"clang-format version 18\.", version.stdout):
            raise ValueError("clang-format major must be 18: " + version.stdout.strip())
        result = subprocess.run(
            [
                "git",
                "clang-format-18",
                "--binary",
                "clang-format-18",
                "--extensions=c,h,cu,cuh",
                "--style=file",
                "--diff",
                base,
                args.head,
                "--",
                *paths,
            ],
            cwd=root,
            check=False,
        )
        if result.returncode not in (0, 1):
            raise ValueError(f"git clang-format-18 failed: exit {result.returncode}")
        print(f"changed-format: files={len(paths)} format-diff={int(result.returncode == 1)}")
        return result.returncode
    except (OSError, UnicodeError, ValueError, subprocess.SubprocessError) as error:
        print(f"changed-format: INFRASTRUCTURE ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
