#!/usr/bin/env python3
"""Keep references to the private repository out of the public tree.

kayfabe is built, tested and reviewed from this repository alone. The private repository
(`kayfabe-private`) is a personal backup for diagnostic material; nothing public may depend on it
(docs/design/V3_PROVENANCE.md section 2, docs/OWNER_RULINGS.md section AG). A link, a path or a
"see the private repo" in a tracked file sends an outside reader to a place they cannot open.

The gate fails on the name `kayfabe-private` (case-insensitive, also with `_`) in any tracked text
file, except:

* ALLOWED: the two documents that state the policy, and this gate with its test.
* KNOWN_DEBT: files that mention it today and have not been rephrased yet. Shrink-only: a file in
  the list that no longer matches also fails, so the list is edited down and never up.
* the receipt schema id `kayfabe-private-watchdog-recovery/1`, a name that a public tool
  (scripts/bench/windows/host_tools/runners/boundary-tools/recover_pc_watchdog.py) writes into its
  JSON receipts. It names a file format, not the repository, and the receipts are recorded evidence.

Exit codes: 0 clean; 1 a new reference or a stale KNOWN_DEBT entry; 2 usage.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

PATTERN = re.compile(rb"kayfabe[-_]private", re.IGNORECASE)
SCHEMA = re.compile(rb"kayfabe-private-watchdog-recovery(?:/\d+)?", re.IGNORECASE)

ALLOWED = frozenset(
    {
        "docs/design/V3_PROVENANCE.md",
        "docs/OWNER_RULINGS.md",
        "scripts/ci/no_private_refs.py",
        "scripts/ci/test_no_private_refs.py",
    }
)

# Rephrase each of these to "the private repository" (no name, no link, no path), then delete the
# entry. Do not add to this list.
KNOWN_DEBT = frozenset(
    {
        "docs/HANDOFF_FOR_CODEX_20261011.md",
        "docs/RESUME_CODEX_WINDOWS_20261011.md",
        "docs/design/tier_checks/tier1_ogkm.md",
        # A generated inventory row that quotes tier1_ogkm.md's text; goes with it.
        "traces/doc_rot_20261018/inventory.md",
    }
)


def tracked(root: Path) -> list[str]:
    """git's tracked files, else a directory walk (for a scratch tree in the tests)."""
    if (root / ".git").exists():
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"], check=True, capture_output=True
        ).stdout
        return sorted(p for p in out.decode().split("\0") if p)
    return sorted(
        str(p.relative_to(root))
        for p in root.rglob("*")
        if p.is_file() and ".git" not in p.relative_to(root).parts
    )


def hits_in(data: bytes) -> int:
    """How many references to the private repository `data` carries (the schema id excluded)."""
    if b"\0" in data[:8192]:  # binary
        return 0
    return len(PATTERN.findall(SCHEMA.sub(b"", data)))


def scan(root: Path) -> dict[str, int]:
    found = {}
    for rel in tracked(root):
        path = root / rel
        if not path.is_file() or path.is_symlink():
            continue
        n = hits_in(path.read_bytes())
        if n:
            found[rel] = n
    return found


def judge(found: dict[str, int]) -> tuple[list[str], list[str]]:
    """(new references, stale KNOWN_DEBT entries)."""
    new = sorted(rel for rel in found if rel not in ALLOWED and rel not in KNOWN_DEBT)
    stale = sorted(rel for rel in KNOWN_DEBT if rel not in found)
    return new, stale


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--root", type=Path, default=ROOT)
    args = ap.parse_args(argv)

    new, stale = judge(scan(args.root))
    for rel in new:
        print(f"FAIL {rel}: names the private repository. The public tree must not depend on it.")
    for rel in stale:
        print(f"FAIL {rel}: listed in KNOWN_DEBT but no longer matches. Delete the entry.")
    if new or stale:
        print(
            "\nAllowed only in: " + ", ".join(sorted(ALLOWED - {Path(__file__).name})) + ".",
            file=sys.stderr,
        )
        return 1
    print("no_private_refs: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
