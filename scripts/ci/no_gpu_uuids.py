#!/usr/bin/env python3
"""Keep GPU board UUIDs out of traces/ and docs/ (a ratchet: the existing hits may only go down).

A GPU UUID (`GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, as `nvidia-smi -L` prints it, or the value
of a `gpu-uuid=` / `gpu_uuid:` assignment) identifies one physical board. Traces and docs are
published with the repository, so a board's identifier does not belong in them.

How the ratchet works. `gpu-uuids-baseline.json` records, per file, a fingerprint of each UUID
found there and how many times (the fingerprint is a truncated SHA-256 of the lower-cased UUID;
the UUIDs themselves are never written to the baseline, to a log or to a message).

* a UUID, or an extra copy of one, that is not in the baseline for that same file fails: this is
  what a newly added or changed file with a UUID looks like;
* a baseline entry that is no longer found also fails, with the instruction to run `--shrink`,
  so the baseline can only get smaller (the count may only go down);
* `--shrink` rewrites the baseline to what is still found and never adds an entry;
* `--init` writes the first baseline, and refuses to overwrite one.

Exit codes: 0 clean; 1 a new hit or a stale baseline; 2 usage or a missing baseline.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).resolve().parent / "gpu-uuids-baseline.json"
SCOPE = ("traces", "docs")

_UUID = rb"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
PATTERN = re.compile(
    rb"(?:gpu-|gpu[-_]uuid[ \t]*[=:][ \t]*[\"']?)(" + _UUID + rb")(?![0-9a-f])", re.IGNORECASE
)
NOTE = "Existing hits, fingerprinted. Shrink-only: no_gpu_uuids.py --shrink."


def fingerprint(uuid: bytes) -> str:
    """A stable identity for one UUID that does not reveal it."""
    return hashlib.sha256(uuid.lower()).hexdigest()[:16]


def scan(data: bytes) -> Counter:
    """Fingerprint -> occurrences, for every GPU UUID in `data`."""
    return Counter(fingerprint(m.group(1)) for m in PATTERN.finditer(data))


def tracked(root: Path) -> list[str]:
    """The files the gate looks at: git's tracked files under traces/ and docs/, else a walk."""
    if (root / ".git").exists():
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--", *SCOPE],
            check=True,
            capture_output=True,
        ).stdout
        return sorted(p for p in out.decode().split("\0") if p)
    found = []
    for top in SCOPE:
        for base, _dirs, names in os.walk(root / top):
            found += [str((Path(base) / n).relative_to(root)) for n in names]
    return sorted(found)


def scan_tree(root: Path, rels: list[str]) -> dict[str, Counter]:
    """Relative path -> fingerprints found there (files without a hit are left out)."""
    found = {}
    for rel in rels:
        path = root / rel
        if not path.is_file():  # a tracked symlink to nowhere, a submodule entry
            continue
        hits = scan(path.read_bytes())
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
    body = {"note": NOTE, "files": {rel: dict(sorted(files[rel].items())) for rel in sorted(files)}}
    path.write_text(json.dumps(body, indent=1, sort_keys=False) + "\n")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--root", type=Path, default=ROOT)
    ap.add_argument("--baseline", type=Path, default=BASELINE)
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--init", action="store_true", help="write the first baseline")
    mode.add_argument("--shrink", action="store_true", help="drop baseline entries that are gone")
    args = ap.parse_args(argv)

    actual = scan_tree(args.root, tracked(args.root))
    if args.init:
        if args.baseline.exists():
            print(
                f"{args.baseline} exists; the baseline is never rewritten upwards", file=sys.stderr
            )
            return 2
        dump_baseline(args.baseline, actual)
        print(f"baseline written: {len(actual)} file(s)")
        return 0
    if not args.baseline.exists():
        print(f"no baseline at {args.baseline} (a gate that cannot compare must not pass)")
        return 2

    baseline = load_baseline(args.baseline)
    new, stale = compare(actual, baseline)
    if args.shrink:
        kept = {rel: have & baseline[rel] for rel, have in actual.items() if rel in baseline}
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
            "A GPU UUID identifies one board. Scrub it (for example GPU-00000000-0000-0000-"
            "0000-000000000000) instead of adding it to the baseline."
        )
    if stale:
        print("The baseline lists UUIDs that are gone: run scripts/ci/no_gpu_uuids.py --shrink")
    return 1 if new or stale else 0


if __name__ == "__main__":
    sys.exit(main())
