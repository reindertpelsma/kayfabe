#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free patch inventory and stock-path regression check, not a shell interpreter.

New patch artifacts or application sites need review. Known applicators are pinned
by source hash; stock entrypoints must not reach them through literal script names.
Dynamic dispatch and arbitrary source rewrites still require human code review.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
INVENTORY = Path("scripts/ci/no-patch-inventory.json")
EXPECTED_DEFAULT = "stock host and guest nvidia drivers are unmodified by default"
# Mutation/test programs edit their own scratch sources, not installed drivers.
SELF = {"scripts/ci/no_patch_check.py", "scripts/ci/release_checks_mutants.py"}
APPLICATION = re.compile(
    r"[\w/-]+\.patch\b|[\w/-]+\.diff\b|\bpatch\s+[-<]|\bgit\s+[^\n]*\b(?:apply|am)\b|"
    r"['\"]git['\"][^\n]*['\"](?:apply|am)['\"]|['\"]patch['\"]\s*,|"
    r"(?m:^\s*(?:(?:sudo|command|exec|if|then)\s+)*patch(?:\s|$))"
)


def script_sources(root: Path) -> dict[str, str]:
    sources = {}
    for directory in ("scripts", "tools"):
        for path in (root / directory).rglob("*"):
            rel = path.relative_to(root)
            if not path.is_file() or path.is_symlink():
                continue
            if any(
                p in {"tests", "test", "fixtures", "__pycache__"} for p in rel.parts
            ):
                continue
            if (
                rel.as_posix() in SELF
                or path.name.startswith("test_")
                or "mutant" in path.name
                or "mutation" in path.name
            ):
                continue
            if path.suffix not in {".sh", ".py", ".ps1", ".js", ".mjs", ""}:
                continue
            text = path.read_text(errors="replace")
            if path.suffix == "" and not text.startswith("#!"):
                continue
            sources[rel.as_posix()] = text
    return sources


def patch_files(root: Path) -> set[str]:
    return {
        p.relative_to(root).as_posix()
        for pattern in ("*.patch", "*.diff")
        for p in root.rglob(pattern)
        if p.is_file()
        and not any(
            part in {".git", "target", "node_modules", "fixtures"}
            for part in p.relative_to(root).parts
        )
    }


def validate(root: Path, inventory: dict) -> list[str]:
    errors = []
    if inventory.get("version") != 1:
        errors.append("unknown inventory version")
    patches = inventory["patches"]
    actual = patch_files(root)
    for rel in sorted(actual - patches.keys()):
        errors.append(f"unregistered patch: {rel}; review its default/opt-in scope")
    for rel in sorted(patches.keys() - actual):
        errors.append(f"missing patch: {rel}; update the inventory after review")
    for rel, item in patches.items():
        if item["kind"] not in {"opt-in", "recorded-only", "archive-only"}:
            errors.append(f"{rel}: unknown patch classification")
        pointer = item.get("opt_in", "")
        if not pointer or not (root / pointer).is_file():
            errors.append(f"{rel}: missing opt-in pointer {pointer!r}")
        if not item.get("reason"):
            errors.append(f"{rel}: missing scope reason")
    doc = root / inventory["policy_doc"]
    if not doc.is_file():
        errors.append("missing policy document")
    else:
        text = doc.read_text().lower()
        qualified = " ".join(re.sub(r"[`*_]", "", text).split())
        if EXPECTED_DEFAULT not in qualified:
            errors.append(
                "policy must state stock host and guest NVIDIA drivers are unmodified by default"
            )
        if "opt-in" not in text:
            errors.append("policy must disclose opt-in patches")
    sources = script_sources(root)
    applicators = inventory["applicators"]
    for rel, text in sources.items():
        if APPLICATION.search(text) and rel not in applicators:
            errors.append(f"unreviewed patch applicator/reference: {rel}")
    for rel, digest in applicators.items():
        path = root / rel
        if (
            not path.is_file()
            or hashlib.sha256(path.read_bytes()).hexdigest() != digest
        ):
            errors.append(
                f"{rel}: review hash changed; re-audit application and stock call paths"
            )
    # Conservative literal call graph: full relative paths and basenames cover
    # common $ROOT/foo.sh and $HERE/foo.sh forms, even when inside a heredoc.
    by_name = {}
    for rel in sources:
        path = Path(rel)
        by_name.setdefault(path.name, set()).add(rel)
    pending = list(inventory["entrypoints"])
    for rel in pending:
        if rel not in sources:
            errors.append(f"missing stock entrypoint: {rel}")
    visited = set()
    while pending:
        rel = pending.pop()
        if rel in visited:
            continue
        visited.add(rel)
        if rel in applicators:
            errors.append(f"stock path reaches opt-in applicator: {rel}")
        text = sources.get(rel, "")
        text = "\n".join(
            line for line in text.splitlines() if not line.lstrip().startswith("#")
        )
        names = set(re.findall(r"[\w.-]+", text))
        if rel.endswith(".py"):
            names.update(
                name.rsplit(".", 1)[-1] + ".py"
                for name in re.findall(r"\b(?:import|from)\s+([\w.]+)", text)
            )
        for name in names:
            pending.extend(by_name.get(name, ()))
    return errors


def table(inventory: dict) -> str:
    lines = [
        "| Patch | Scope | Explicit opt-in / record | Reason |",
        "|---|---|---|---|",
    ]
    for rel, item in sorted(inventory["patches"].items()):
        lines.append(
            f'| `{rel}` | {item["kind"]} | `{item["opt_in"]}` | {item["reason"]} |'
        )
    return "\n".join(lines)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--inventory", type=Path)
    args = parser.parse_args(argv)
    try:
        inventory = json.loads((args.inventory or args.root / INVENTORY).read_text())
        errors = validate(args.root, inventory)
        print(table(inventory))
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"no-patch-check: invalid inventory/tree: {error}", file=sys.stderr)
        return 2
    for error in errors:
        print(f"FAIL no-patch-check: {error}", file=sys.stderr)
    print(
        f"CODE READ: {len(inventory['patches'])} patch artifacts, {len(inventory['applicators'])} reviewed application/reference scripts, {len(errors)} errors"
    )
    print(
        "LIMIT: lexical stock-path audit; dynamic dispatch and arbitrary source rewrites require code review."
    )
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
