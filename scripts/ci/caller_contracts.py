#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Every `CALLER-CONTRACT:` marker in the tree is listed in a reviewed ledger.

Owner rule, 2026-11-13: a function that relies on its caller instead of validating its inputs
carries a doc comment starting `CALLER-CONTRACT:` (what the caller must guarantee, why it is not
validated, what breaks otherwise). This gate makes a new such deviation impossible to add silently:
each definition must have a row in `scripts/ci/caller-contracts.tsv`, and the ledger is an exact
list (a stale row or a wrong count fails too, like the unsafe ratchet).

* Scanned: tracked files under crates/ cuda/ qemu/hw/misc/kf3/ broker/ firmware/ tools/.
* A DEFINITION is a comment line whose text starts with the marker (after `///`, `//!`, `//`, `/*`,
  `*` or `#`). A mention inside running prose ("(CALLER-CONTRACT above)") is a citation, not counted.
* A definition is keyed by (path, item): the first item after the comment block (fn, struct, enum,
  const, static, trait, type, impl, `#define`, or a C function). No item within 40 lines fails.

Usage: caller_contracts.py [--ref REV]   (--ref scans REV's tree and prints the rows it needs)
Exit codes: 0 clean; 1 a finding; 2 usage.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LEDGER = "scripts/ci/caller-contracts.tsv"
SCOPE = ("crates/", "cuda/", "qemu/hw/misc/kf3/", "broker/", "firmware/", "tools/")
TOKEN = "CALLER-CONTRACT:"
WINDOW = 40

DEF = re.compile(r"^\s*(?:///|//!|//|/\*+|\*|#)\s*CALLER-CONTRACT:")
COMMENT = re.compile(r"^\s*(?://|/\*|\*|#\s)")
RUST = re.compile(r"\b(?:fn|struct|enum|const|static|trait|type|union)\s+(?:mut\s+)?(\w+)")
IMPL = re.compile(r"^\s*(?:pub\s+)?(?:unsafe\s+)?impl\b\s*(.*?)\s*(?:\{.*|where\b.*)?$")
DEFINE = re.compile(r"^\s*#\s*define\s+(\w+)")
CFUNC = re.compile(r"^\s*(?!(?:if|for|while|switch|return|else)\b)[\w\s\*]+?\b(\w+)\s*\(")


def item_of(line: str, rust: bool = True) -> str | None:
    """The item name a source line defines, or None (`rust` picks Rust or C/C++ rules)."""
    m = DEFINE.match(line)
    if m:
        return m.group(1)
    m = IMPL.match(line) if rust else None
    if m:
        return "impl " + re.sub(r"\s+", " ", m.group(1))
    if rust and re.match(r"^\s*(?:pub|unsafe|extern|async|const|static|fn|struct|enum|trait|type|union)\b", line):
        m = RUST.search(line)
        if m:
            return m.group(1)
    # C: a function definition starts in column 0; an indented line is a statement inside a body, so a
    # marker above it is not on an item (it would otherwise name `if`).
    m = CFUNC.match(line) if not rust and not line[:1].isspace() else None
    if m:
        return m.group(1)
    return None


def definitions(path: str, text: str):
    """Yield (lineno, item or None) for each definition marker in `text`."""
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if not DEF.match(line):
            continue
        item = None
        for j in range(i + 1, min(len(lines), i + 1 + WINDOW)):
            s = lines[j]
            if not s.strip() or COMMENT.match(s) or s.lstrip().startswith("#["):
                continue
            rust = path.endswith(".rs")
            if not rust and s[:1].isspace():
                break  # C: an indented line is a statement in a body; the marker is not on an item
            item = item_of(s, rust)
            if item:
                break
        yield i + 1, item


def in_scope(path: str) -> bool:
    return path.startswith(SCOPE)  # archive/ and third_party/ are outside every prefix


def files_worktree(root: Path):
    out = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], check=True, capture_output=True).stdout
    for p in sorted(x for x in out.decode().split("\0") if x):
        if in_scope(p) and (root / p).is_file():
            data = (root / p).read_bytes()
            yield p, data


def files_ref(root: Path, ref: str):
    out = subprocess.run(
        ["git", "-C", str(root), "ls-tree", "-r", "-z", "--name-only", ref], check=True, capture_output=True
    ).stdout
    for p in sorted(x for x in out.decode().split("\0") if x):
        if in_scope(p):
            yield p, subprocess.run(
                ["git", "-C", str(root), "show", f"{ref}:{p}"], check=True, capture_output=True
            ).stdout


def scan(files) -> tuple[Counter, dict, list[str]]:
    counts: Counter = Counter()
    where: dict = {}
    errors: list[str] = []
    for path, data in files:
        if TOKEN.encode() not in data:
            continue
        try:
            text = data.decode()
        except UnicodeDecodeError:
            continue
        for lineno, item in definitions(path, text):
            if item is None:
                errors.append(
                    f"{path}:{lineno}: CALLER-CONTRACT marker has no item (fn, struct, impl, #define, ...) "
                    f"within {WINDOW} lines; put the marker on the item it constrains"
                )
                continue
            counts[(path, item)] += 1
            where.setdefault((path, item), []).append(lineno)
    return counts, where, errors


def read_ledger(text: str) -> tuple[dict, list[str]]:
    rows: dict = {}
    errors: list[str] = []
    for n, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) != 4 or not f[2].strip().isdigit():
            errors.append(f"{LEDGER}:{n}: a row is path<TAB>item<TAB>count<TAB>reason")
            continue
        key = (f[0], f[1])
        if key in rows:
            errors.append(f"{LEDGER}:{n}: duplicate row for {f[0]} {f[1]}")
        if not f[3].strip():
            errors.append(f"{LEDGER}:{n}: row for {f[0]} {f[1]} has an empty reason")
        rows[key] = (int(f[2]), n)
    return rows, errors


def check(files, ledger_text: str) -> list[str]:
    counts, where, errors = scan(files)
    rows, rerrs = read_ledger(ledger_text)
    errors += rerrs
    for key, n in sorted(counts.items()):
        path, item = key
        at = f"{path}:{where[key][0]}"
        if key not in rows:
            errors.append(f"{at}: CALLER-CONTRACT on `{item}` has no row in {LEDGER} (add: {path}\\t{item}\\t{n}\\t<reason>)")
        elif rows[key][0] != n:
            errors.append(f"{at}: `{item}` has {n} CALLER-CONTRACT marker(s), {LEDGER}:{rows[key][1]} says {rows[key][0]}")
    for key, (_, n) in sorted(rows.items()):
        if key not in counts:
            errors.append(f"{LEDGER}:{n}: stale row, no CALLER-CONTRACT on `{key[1]}` in {key[0]}; remove it")
    return errors


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ref", help="scan this revision's tree and print the rows it would need")
    a = ap.parse_args(argv)
    if a.ref:
        counts, where, errors = scan(files_ref(ROOT, a.ref))
        for k, n in sorted(counts.items()):
            print(f"{k[0]}\t{k[1]}\t{n}\t(lines {','.join(map(str, where[k]))})")
        for e in errors:
            print(e)
        return 1 if errors else 0
    errors = check(files_worktree(ROOT), (ROOT / LEDGER).read_text())
    for e in errors:
        print(e, file=sys.stderr)
    if errors:
        print(f"caller_contracts: {len(errors)} finding(s)", file=sys.stderr)
        return 1
    print("caller_contracts: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
