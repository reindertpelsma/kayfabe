#!/usr/bin/env python3
"""Reference tree key from git itself, for parity with src/treekey.js (test only).

The CI Worker computes the key from the GitHub trees API with its own matcher. This script
computes it from `git ls-files -s` and `git check-ignore --no-index`, so a test can assert that
both agree on this repository's real paths (test/treekey-parity.test.js).

check-ignore reads every per-directory .gitignore and the user's excludes, which must not take
part. So the listed paths are recreated as empty files in a fresh temporary repository (a
submodule as a directory), and git is run there with core.excludesFile set to the pattern file
and the global and system configuration disabled.

Usage: treekey_ref.py --patterns FILE [--repo DIR] [--list]
Prints the hex key, or with --list one 'ignored|kept<TAB>path' line per entry first.
"""

import argparse
import hashlib
import os
import subprocess
import sys
import tempfile

VERSION = b"kf-tree-v2\n"


def ls_files(repo):
    out = subprocess.run(
        ["git", "-C", repo, "ls-files", "-s", "-z"], check=True, capture_output=True
    ).stdout
    entries = []
    for record in out.split(b"\0"):
        if not record:
            continue
        meta, path = record.split(b"\t", 1)
        mode, sha, stage = meta.split(b" ")
        if stage != b"0":
            raise SystemExit("unmerged index entry")
        entries.append((path, mode, sha))
    return entries


def ignored_paths(entries, patterns):
    env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull)
    with tempfile.TemporaryDirectory(prefix="kf-treekey-") as shadow:
        subprocess.run(["git", "init", "-q", shadow], check=True, env=env)
        for path, mode, _ in entries:
            target = os.path.join(shadow.encode(), path)
            if mode == b"160000":
                os.makedirs(target, exist_ok=True)
            else:
                os.makedirs(os.path.dirname(target), exist_ok=True)
                open(target, "wb").close()
        result = subprocess.run(
            [
                "git",
                "-C",
                shadow,
                "-c",
                "core.excludesFile=" + os.path.abspath(patterns),
                "check-ignore",
                "--no-index",
                "--stdin",
                "-z",
            ],
            input=b"".join(path + b"\0" for path, _, _ in entries),
            capture_output=True,
            env=env,
        )
        # Exit 1 means "nothing ignored"; anything else above 1 is an error.
        if result.returncode not in (0, 1):
            raise SystemExit("git check-ignore failed")
        return {p for p in result.stdout.split(b"\0") if p}


def ignore_digest(patterns):
    """SHA-256 of the effective pattern lines (non-blank, not starting with '#')."""
    with open(patterns, "rb") as source:
        lines = [line for line in source.read().split(b"\n") if line and not line.startswith(b"#")]
    return hashlib.sha256(b"\n".join(lines)).hexdigest().encode()


def tree_key(entries, ignored, patterns):
    kept = sorted((e for e in entries if e[0] not in ignored), key=lambda e: e[0])
    digest = hashlib.sha256(VERSION + b"ignore " + ignore_digest(patterns) + b"\n")
    for path, mode, sha in kept:
        digest.update(path + b"\0" + mode + b"\0" + sha + b"\n")
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--patterns", required=True)
    parser.add_argument("--repo", default=".")
    parser.add_argument("--list", action="store_true")
    args = parser.parse_args()
    entries = ls_files(args.repo)
    ignored = ignored_paths(entries, args.patterns)
    if args.list:
        for path, _, _ in entries:
            state = b"ignored" if path in ignored else b"kept"
            sys.stdout.buffer.write(state + b"\t" + path + b"\n")
    print(tree_key(entries, ignored, args.patterns))


if __name__ == "__main__":
    main()
