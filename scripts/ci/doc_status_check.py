#!/usr/bin/env python3
"""Check dated opening STATUS metadata and explicit design-doc supersessions.

Run from any directory; --root selects a checkout (default: this script's repo).
Exit 0 means these mechanical checks passed, 1 lists violations, and 2 means the
scan could not run. This does not determine whether a status is true, whether a
correction is above the claim it corrects, or whether prose implies supersession.
Those semantic questions still need review. Only docs/design/**/*.md is scanned;
explicit supersession targets can also resolve to preserved docs/archive/ files.
"""

from __future__ import annotations

import argparse
from datetime import date
import json
from pathlib import Path
import re
import sys
from urllib.parse import unquote

STATES = r"LIVE|SUPERSEDED|ANSWERED|DESIGN-ONLY|RESEARCH"
STATUS = re.compile(r"^(?:[>#*★⊘✔!]+\s*)*STATUS\s*(?=[:(—–-])", re.MULTILINE)
DATE = re.compile(r"\b\d{4}-\d{2}-\d{2}\b")
SUPERSEDES = re.compile(
    r"\bsupersedes\s+(?:`([^`\n]+\.md)`|\[[^\]\n]+\]\(([^)\n]+\.md(?:#[^)\n]*)?)\))",
    re.IGNORECASE,
)


def visible(text: str) -> str:
    """Hide comments and fenced examples without changing source line numbers."""
    # YAML front matter is document metadata, not opening prose. Keep line numbers.
    text = re.sub(r"\A---\n.*?\n(?:---|\.\.\.)[ \t]*(?:\n|$)",
                  lambda m: "\n" * m.group().count("\n"), text, count=1, flags=re.DOTALL)
    text = re.sub(r"<!--.*?(?:-->|\Z)", lambda m: "\n" * m.group().count("\n"), text,
                  flags=re.DOTALL)
    lines = []
    fence = None
    for line in text.splitlines():
        stripped = re.sub(r"^\s*(?:>\s*)*", "", line)
        marker = re.match(r"(`{3,}|~{3,})", stripped)
        if fence is not None:
            if marker and marker[1][0] == fence[0] and len(marker[1]) >= len(fence):
                fence = None
            lines.append("")
        elif marker:
            fence = marker[1]
            lines.append("")
        else:
            lines.append(line)
    return "\n".join(lines)


def paragraphs(text: str) -> list[tuple[int, str]]:
    return [(text.count("\n", 0, m.start()) + 1, m.group())
            for m in re.finditer(r"[^\n]+(?:\n(?!\s*\n)[^\n]+)*", text)
            if m.group().strip()]


def status_value(paragraph: str, match: re.Match) -> str:
    tail = paragraph[match.end():].lstrip()
    if tail.startswith(":"):
        return tail[1:].lstrip(" *")
    if tail.startswith("("):
        end = tail.find(")")
        return tail[end + 1:].lstrip(" :*") if end >= 0 else ""
    # Existing heading form: STATUS — 2026-09-26 / **LIVE**.
    return re.sub(r"^[—–-]\s*\d{4}-\d{2}-\d{2}\s*/\s*", "", tail).lstrip(" *")


def check(root: Path) -> dict:
    design = root / "docs/design"
    paths = sorted(design.rglob("*.md")) if design.is_dir() else []
    if not paths:
        raise ValueError("no design documents found under docs/design")
    violations = []

    def add(path: Path, line: int, kind: str, message: str):
        violations.append({"path": path.relative_to(root).as_posix(), "line": line,
                           "kind": kind, "message": message})

    texts = {path: visible(path.read_text(encoding="utf-8")) for path in paths}
    for path, text in texts.items():
        parts = paragraphs(text)
        # A leading H1 title is optional. STATUS may also precede that title.
        body = parts[1:] if parts and parts[0][1].startswith("# ") else parts
        status_parts = [(line, para, STATUS.search(para)) for line, para in body
                        if STATUS.search(para)]
        if not status_parts:
            add(path, 1, "missing-status", "open with a dated STATUS using an allowed state")
        else:
            line, para, match = status_parts[0]
            line += para.count("\n", 0, match.start())
            if not body or para != body[0][1] or match.start() != 0:
                add(path, line, "misplaced-status", "move current STATUS before opening prose/corrections")
            if not re.match(rf"(?:{STATES})(?![\w-])", status_value(para, match)):
                add(path, line, "invalid-status", f"use one of {STATES.replace('|', ', ')}")
            dates = DATE.findall(para)
            if not dates:
                add(path, line, "missing-date", "date the STATUS paragraph with YYYY-MM-DD")
            else:
                for raw in dates:
                    try:
                        date.fromisoformat(raw)
                    except ValueError:
                        add(path, line, "invalid-date", f"invalid calendar date {raw}")

        for match in SUPERSEDES.finditer(text):
            target_name = unquote((match[1] or match[2]).split("#", 1)[0])
            candidates = [path.parent / target_name, root / target_name]
            if "/" not in target_name:
                candidates.append(root / "docs/archive" / target_name)
            target = next((p.resolve() for p in candidates
                           if p.is_file() and p.resolve().is_relative_to(root)), None)
            line = text.count("\n", 0, match.start()) + 1
            if target is None:
                add(path, line, "missing-supersession-target", f"resolve {target_name} or review its historical location")
                continue
            old_text = texts.get(target)
            if old_text is None:
                old_text = visible(target.read_text(encoding="utf-8"))
            notices = [para for _, para in paragraphs(old_text)
                       if re.search(r"\bsuperseded\b", para, re.IGNORECASE)
                       and path.name in para]
            if not notices:
                add(path, line, "missing-supersession-notice",
                    f"record SUPERSEDED and {path.name} in the same paragraph of {target.relative_to(root)}")
    return {"checked": len(paths), "violations": sorted(violations, key=lambda v: (v["path"], v["line"], v["kind"]))}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--format", choices=("text", "json"), default="text")
    args = parser.parse_args()
    try:
        result = check(args.root.resolve())
    except (OSError, UnicodeError, ValueError) as error:
        print(f"doc status scan failed: {error}", file=sys.stderr)
        return 2
    if args.format == "json":
        print(json.dumps(result, indent=2))
    else:
        for item in result["violations"]:
            print(f"{item['path']}:{item['line']}: {item['kind']}: {item['message']}")
        print(f"design docs: checked={result['checked']} violations={len(result['violations'])}")
    return int(bool(result["violations"]))


if __name__ == "__main__":
    sys.exit(main())
