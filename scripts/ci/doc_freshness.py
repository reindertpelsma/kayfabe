#!/usr/bin/env python3
"""Inventory doc freshness, ratchet legacy findings, and fail new/edited doc debt.

Extends PR #87's doc_status_check parser. Uses only the Python standard library
and local git; external links are never fetched. Literal references are review
hints, not proof of runtime behavior or of semantic obsolescence.
"""

from __future__ import annotations

import argparse
from datetime import date
import fnmatch
import hashlib
import html
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from urllib.parse import unquote, urlsplit

import debt
import doc_status_check as status

ROOT = Path(__file__).resolve().parents[2]
ENV = r"(?:KF3_|KF_|KAYFABE_|NVKVM_)[A-Z0-9_]+"
FLAG = r"--[a-z][a-z0-9]*(?:-[a-z0-9]+)*"
CRATE = r"(?:kf|kayfabe)-[a-z][a-z0-9]*(?:-[a-z0-9]+)*"
BRANCH = r"(?<![\w/])(?:origin/)?(?:claude|codex|integration|feature|backup)/[A-Za-z0-9_./-]+"
CODE_PATH = (
    r"(?<![\w/])(?:\.{1,2}/)*(?:crates|scripts|qemu|cuda|broker|tools|firmware|\.github|"
    + CRATE
    + r")/[A-Za-z0-9_./*?-]+"
)
# git grep -E uses POSIX ERE, which has no non-capturing group syntax.
SOURCE_TOKEN = rf"\b[A-Z][A-Z0-9_]{{2,}}\b|{FLAG}".replace("(?:", "(")
CODE_ROOTS = ("crates/", "scripts/", "qemu/", "cuda/", "broker/", "tools/", "firmware/", ".github/")
EXCLUDES = (
    "archive/**",
    "docs/**",
    "traces/**",
    "third_party/**",
    "**/target/**",
    "scripts/ci/test_*.py",
    "scripts/ci/*debt.json",
    "**/Cargo.lock",
)
C1_DECISIONS = {
    "V3_BAR1_DOORBELL.md": "Scope GB203 versus Hopper before choosing LIVE or RESEARCH (C1).",
    "V3_DOORBELL_IOEVENTFD.md": "Review LIVE with existing rollout qualifications (C1).",
    "V3_HEADLESS_GRAPHICS.md": "Choose ANSWERED experiment versus LIVE coverage (C1).",
    "V3_P4_PORT_MAP.md": "Choose RESEARCH or identify a successor before archiving (C1).",
    "V3_P5_PORT_MAP.md": "Preserve porting history; identify authoritative successor (C1).",
    "V3_SECURITY_AUDIT_PLAN.md": "Choose LIVE plan versus DESIGN-ONLY stage 2 (C1).",
    "V3_SWEEP_AND_INSTALL.md": "Review whole-document scope before DESIGN-ONLY (C1).",
    "V3_USER_RING_LIFECYCLE_REVIEW1_20261016.md": (
        "Review RESEARCH; supersede findings separately (C1)."
    ),
    "V3_UVM_B3_IMPLEMENTATION.md": "Review ANSWERED host proof, excluding guest integration (C1).",
}


def git(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)


def inventory(root: Path) -> list[Path]:
    paths = {p for p in (root / "docs").rglob("*") if p.is_file()}
    paths.update(p for p in root.glob("README*") if p.is_file() and p.suffix == ".md")
    if (root / "ARCHITECTURE.md").is_file():
        paths.add(root / "ARCHITECTURE.md")
    for path in paths:
        if not path.resolve().is_relative_to(root):
            raise ValueError(f"document escapes checkout: {path.relative_to(root)}")
    return sorted(paths)


def source_index(root: Path) -> tuple:
    tracked = git(root, "ls-files", "-z")
    if tracked.returncode:
        raise ValueError("freshness requires a git checkout")
    files = [p for p in tracked.stdout.split("\0") if p]
    command = ["grep", "-I", "-h", "-o", "-E", SOURCE_TOKEN, "--", "."]
    command.extend(f":(exclude){path}" for path in EXCLUDES)
    found = git(root, *command)
    if found.returncode not in (0, 1):
        raise ValueError(f"source reference grep failed: {found.stderr.strip()}")
    tokens = set(found.stdout.splitlines())
    # Package names include retained crates, never archived/third-party manifests.
    crates = set()
    for name in files:
        if not name.endswith("Cargo.toml") or name.startswith(("archive/", "third_party/")):
            continue
        path = root / name
        if path.is_file():
            manifest = tomllib.loads(path.read_text(encoding="utf-8"))
            package = manifest.get("package", {}).get("name")
            if isinstance(package, str):
                crates.add(package)
    refs = git(root, "for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes")
    if refs.returncode:
        raise ValueError("cannot enumerate branch references")
    branches = set(refs.stdout.splitlines())
    branches.update(b.removeprefix("origin/") for b in list(branches))
    source_files = {
        f
        for f in files
        if not f.startswith(("archive/", "docs/", "traces/", "third_party/"))
        and (root / f).is_file()
    }
    suffixes = set()
    for name in source_files:
        parts = name.split("/")
        suffixes.update("/".join(parts[i:]) for i in range(len(parts)))
    return tokens, crates, branches, source_files, suffixes


def links(text: str) -> list[tuple[int, str]]:
    """Inline/images, reference links and HTML destinations outside fenced examples."""
    clean = status.visible(text)
    clean = re.sub(
        r"(`+)(?!`)(.*?)\1(?!`)", lambda m: "\n" * m[0].count("\n"), clean, flags=re.DOTALL
    )
    definitions = {}
    definition_lines = set()
    for match in re.finditer(r"^ {0,3}\[([^]\n]+)\]:\s*(<[^>]+>|\S+)", clean, re.MULTILINE):
        definitions[" ".join(match[1].lower().split())] = match[2].strip("<>")
        definition_lines.add(clean.count("\n", 0, match.start()) + 1)
    found = []
    # Balance parentheses so a real filename like design_(v3).md is not truncated.
    for match in re.finditer(r"!?\[([^]\n]*)\]", clean):
        line = clean.count("\n", 0, match.start()) + 1
        if line in definition_lines:
            continue
        tail = clean[match.end() :]
        target = None
        if tail.startswith("("):
            depth, escaped, end = 1, False, 1
            while end < len(tail) and depth:
                char = tail[end]
                if not escaped:
                    depth += (char == "(") - (char == ")")
                escaped = char == "\\" and not escaped
                end += 1
            if depth == 0:
                destination = tail[1 : end - 1].strip()
                if destination.startswith("<") and ">" in destination:
                    target = destination[1 : destination.index(">")]
                elif destination:
                    target = re.split(r"\s+[\"']", destination, maxsplit=1)[0].strip()
        else:
            reference = re.match(r"\[([^]\n]*)\]", tail)
            label = (reference[1] or match[1]) if reference else match[1]
            target = definitions.get(" ".join(label.lower().split()))
        if target is not None:
            found.append((line, re.sub(r"\\([ ()])", r"\1", target)))
    for match in re.finditer(r"\b(?:href|src)\s*=\s*([\"'])(.*?)\1", clean, re.IGNORECASE):
        found.append((clean.count("\n", 0, match.start()) + 1, match[2]))
    return sorted(set(found))


def local_target(root: Path, path: Path, destination: str) -> Path | None:
    parsed = urlsplit(destination)
    if parsed.scheme or parsed.netloc:
        return None
    name = unquote(parsed.path)
    if name.startswith("/"):
        target = root / name.lstrip("/")
    else:
        target = path.parent / name if name else path
    return target.resolve()


def anchors(text: str) -> set[str]:
    """Common GitHub heading IDs plus explicit HTML anchors; preserve duplicates."""
    clean = status.visible(text)
    result = {m[2] for m in re.finditer(r"\b(?:id|name)\s*=\s*([\"'])(.*?)\1", clean)}
    headings = []
    lines = clean.splitlines()
    for number, line in enumerate(lines):
        heading = re.match(r"^ {0,3}#{1,6}\s+(.*?)(?:\s+#+\s*)?$", line)
        if heading:
            headings.append(heading[1])
        elif number and re.fullmatch(r" {0,3}(?:=+|-+)\s*", line) and lines[number - 1].strip():
            headings.append(lines[number - 1])
    for heading in headings:
        heading = re.sub(r"\[([^]]+)\]\([^)]*\)", r"\1", heading)
        heading = html.unescape(re.sub(r"<[^>]+>", "", heading)).lower()
        slug = re.sub(r"[^\w\- ]", "", heading).replace(" ", "-")
        candidate, duplicate = slug, 0
        while candidate in result:
            duplicate += 1
            candidate = f"{slug}-{duplicate}"
        result.add(candidate)
    return result


def broken_destination(root: Path, target: Path, destination: str) -> bool:
    if not target.is_relative_to(root) or not target.exists():
        return True
    fragment = unquote(urlsplit(destination).fragment)
    if fragment and target.suffix == ".md":
        return fragment not in anchors(target.read_text(encoding="utf-8"))
    return False


def dead_references(root: Path, path: Path, text: str, index: tuple) -> list[str]:
    tokens, crates, branches, files, suffixes = index
    # Include fenced commands; hide HTML comments so retired examples can be commented out.
    text = re.sub(r"<!--.*?(?:-->|\Z)", "", text, flags=re.DOTALL)
    envs = set(re.findall(rf"\b{ENV}\b", text))
    envs.update(re.findall(r"\b([A-Z][A-Z0-9_]{2,})=", text))
    envs.update(
        re.findall(r"\b(?:env(?:ironment)?(?: variable)?|export)\s+`?([A-Z][A-Z0-9_]{2,})", text)
    )
    refs = {"env:" + t for t in envs if t not in tokens}
    refs.update("flag:" + t for t in re.findall(FLAG, text) if t not in tokens)
    chunks = re.findall(r"`([^`\n]+)`", text)
    crate_text = "\n".join(chunks)
    crate_refs = set(re.findall(rf"(?<![\w/.-]){CRATE}(?![\w/.-])", crate_text))
    crate_refs.update(re.findall(rf"\bcrate\s+({CRATE})\b", text))
    refs.update("crate:" + t for t in crate_refs if t not in crates)
    branch_refs = set(re.findall(BRANCH, text))
    branch_refs.update(re.findall(r"\bbranch(?:es)?\s+`([^`\n ]+)`", text, re.IGNORECASE))
    refs.update("branch:" + t for t in branch_refs if t.rstrip("./") not in branches)
    candidates = set(re.findall(CODE_PATH, text))
    # Shorthand source filenames inside inline code resolve against tracked suffixes.
    for chunk in chunks:
        match = re.fullmatch(r"([\w./*-]+\.(?:rs|py|sh|c|h|cu|toml|yaml|yml))(?::[\d-]+)?", chunk)
        if match:
            candidates.add(match[1])
    for name in candidates:
        # Absolute code filenames describe a host, rather than this portable checkout.
        # Markdown root-relative destinations are checked separately by local_target.
        if Path(name).is_absolute():
            continue
        name = name.rstrip(".,/")
        options = [root / name, path.parent / name, root / "crates" / name]
        exists = any(p.resolve().is_relative_to(root) and p.exists() for p in options)
        if not exists:
            anchored = name.startswith(CODE_ROOTS) or name.startswith(".")
            anchored = anchored or re.match(rf"{CRATE}/", name) is not None
            if "*" in name or "?" in name:
                exists = any(fnmatch.fnmatch(f, name) for f in files)
                if not anchored:
                    exists = exists or any(fnmatch.fnmatch(f, name) for f in suffixes)
            elif not anchored:
                exists = name in suffixes
        if not exists:
            refs.add("path:" + name)
    return sorted(refs)


def is_manual(name: str) -> bool:
    """The published user manual: versioned by the site per release tag, not a dated record."""
    return name.startswith("docs/howto/") or name == "docs/FAQ.md"


def recommendation(row: dict) -> str:
    name = Path(row["path"]).name
    if row["status"] == "MANUAL":
        return "Published manual page; the site build versions it. Keep it matching the code."
    if row["status"] == "ARCHIVED":
        return "Keep archive read-only; no classification change."
    if row["status"] == "ASSET":
        return "Keep asset with parent record; review parent before moving."
    if name in C1_DECISIONS:
        return C1_DECISIONS[name]
    if name == "STATUS_AND_HANDOFF.md":
        return (
            "Owner: keep LIVE index; date a current source pointer, retain historical checkpoints."
        )
    if name == "V3_CONSTANT_STRATEGY.md":
        return "Owner: use CW1 PR #94 status correction; do not duplicate that change."
    if row["status"] == "SUPERSEDED":
        return "Owner: move complete historical doc to archive; leave linked SUPERSEDED stub."
    if row["status"] == "ANSWERED":
        return (
            "Owner: consider archive with pointer if the answered experiment is no longer active."
        )
    if row["findings"] or row["dead_references"]:
        return "Owner: review listed source evidence before keeping LIVE or naming a successor."
    if row["status"] == "LIVE":
        return "Keep LIVE provisionally; literal checks do not verify current behavior."
    return "Keep declared design/research scope; owner decides completion or successor."


def scan(root: Path, today: date, max_age: int) -> dict:
    paths = inventory(root)
    active = [
        p
        for p in paths
        if p.suffix == ".md"
        and "archive" not in p.relative_to(root).parts
        and not is_manual(p.relative_to(root).as_posix())
    ]
    if not active:
        raise ValueError("no active Markdown documents found")
    index = source_index(root)
    violations = status.check(root, active)["violations"]
    rows = []
    for path in paths:
        name = path.relative_to(root).as_posix()
        row = {
            "path": name,
            "status": "MISSING",
            "date": None,
            "age": None,
            "dead_references": [],
            "successor": None,
            "findings": [],
        }
        if "archive" in path.relative_to(root).parts:
            row["status"] = "ARCHIVED"
        elif path.suffix != ".md":
            row["status"] = "ASSET"
        elif is_manual(name):
            row["status"] = "MANUAL"
        else:
            text = path.read_text(encoding="utf-8")
            opening = status.opening_status(text)
            if opening:
                line, para, match, _ = opening
                value = re.match(rf"(?:{status.STATES})(?![\w-])", status.status_value(para, match))
                row["status"] = value[0] if value else "INVALID"
                dates = status.DATE.findall(para)
                if dates:
                    try:
                        stamp = date.fromisoformat(dates[0])
                        row.update(date=stamp.isoformat(), age=(today - stamp).days)
                    except ValueError:
                        pass
                if row["status"] == "SUPERSEDED":
                    for _, destination in links(para):
                        target = local_target(root, path, destination)
                        if target and target != path.resolve() and target.is_relative_to(root):
                            if target.is_file():
                                row["successor"] = target.relative_to(root).as_posix()
                                break
                    if row["successor"] is None:
                        violations.append(
                            {
                                "path": name,
                                "line": line,
                                "kind": "missing-successor",
                                "message": "link an existing successor in the STATUS paragraph",
                            }
                        )
            for line, destination in links(text):
                target = local_target(root, path, destination)
                if target is not None and broken_destination(root, target, destination):
                    violations.append(
                        {
                            "path": name,
                            "line": line,
                            "kind": "broken-link",
                            "message": f"relative link does not resolve: {destination}",
                        }
                    )
            row["dead_references"] = dead_references(root, path, text, index)
            if row["status"] == "LIVE" and row["age"] is not None and row["age"] > max_age:
                if row["dead_references"]:
                    violations.append(
                        {
                            "path": name,
                            "line": opening[0],
                            "kind": "stale-suspect",
                            "references": row["dead_references"],
                            "message": "old LIVE status with dead literal references: "
                            + ", ".join(row["dead_references"]),
                        }
                    )
        row["findings"] = sorted({v["kind"] for v in violations if v["path"] == name})
        row["recommendation"] = recommendation(row)
        rows.append(row)
    rev = git(root, "rev-parse", "HEAD")
    return {
        "today": today.isoformat(),
        "max_age_days": max_age,
        "revision": rev.stdout.strip() if rev.returncode == 0 else "uncommitted fixture",
        "working_tree_changes": bool(git(root, "status", "--porcelain").stdout),
        "documents": rows,
        "checked": len(active),
        "violations": sorted(violations, key=lambda v: (v["path"], v["line"], v["kind"])),
    }


def ratchet(root: Path, result: dict, baseline: dict) -> dict:
    actual = {}
    for finding in result["violations"]:
        path = finding["path"]
        digest = hashlib.sha256((root / path).read_bytes()).hexdigest()
        messages = finding.get("references", [finding["message"]])
        keys = [debt.identity(path, finding["kind"], message) for message in messages]
        for key in keys:
            actual[key] = digest
        finding["legacy"] = all(baseline.get(key) == digest for key in keys)
    result["new"] = sum(not v["legacy"] for v in result["violations"])
    result["existing_debt"] = len(result["violations"]) - result["new"]
    return actual


def markdown(result: dict) -> str:
    def cell(value):
        return str(value if value is not None else "—").replace("|", "\\|").replace("\n", " ")

    lines = [
        "# Documentation freshness and owner decision inventory",
        "",
        f"**STATUS: RESEARCH, {result['today']}.** Generated review inventory.",
        "",
        f"CODE READ: checkout HEAD `{result['revision']}`; working tree changes="
        f"{result['working_tree_changes']}; local git branch refs and tracked "
        f"source grep; age threshold {result['max_age_days']} days. Active Markdown documents: "
        f"{result['checked']}. Findings: {len(result['violations'])}; new={result['new']}; "
        f"legacy={result['existing_debt']}.",
        "",
        "CODE READ: metadata and literal links/references below are evidence about source text. "
        "Paths, crate manifests, option spellings and local branch refs are checked. "
        "Common heading IDs and explicit HTML anchors are checked for Markdown targets. "
        "Runtime behavior, external URLs and non-Markdown fragment semantics are not verified. "
        "Archive rows are excluded from checks; ASSET rows are non-Markdown inventory only.",
        "",
        "INFERRED: each proposed action needs owner review; a dead reference may describe "
        "intentional history. No status, archive move or deletion follows automatically. "
        "C1's nine decisions are retained from PR #87; the constant strategy belongs to PR #94. "
        "README proposals belong in the DR1 PR body, with no README edits.",
        "",
        "| Document | Status | Date | Age (days) | Dead references (CODE READ) | Successor | "
        "Findings | Proposed action (INFERRED) |",
        "|---|---|---|---:|---|---|---|---|",
    ]
    for row in result["documents"]:
        values = [
            row["path"],
            row["status"],
            row["date"],
            row["age"],
            "; ".join(row["dead_references"]) or "—",
            row["successor"],
            ", ".join(row["findings"]) or "—",
            row["recommendation"],
        ]
        lines.append("| " + " | ".join(cell(v) for v in values) + " |")
    lines.extend(["", "## Actionable diagnostics", ""])
    for item in result["violations"]:
        label = "LEGACY" if item.get("legacy") else "NEW"
        lines.append(
            f"- CODE READ: {item['path']}:{item['line']}: {label} {item['kind']}: "
            + cell(item["message"])
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--today", type=date.fromisoformat, default=date.today())
    parser.add_argument("--max-age-days", type=int, default=60)
    parser.add_argument("--format", choices=("text", "json", "markdown"), default="text")
    parser.add_argument("--baseline", type=Path)
    parser.add_argument(
        "--record", action="store_true", help="explicitly record reviewed legacy debt"
    )
    args = parser.parse_args()
    if args.max_age_days < 0:
        parser.error("--max-age-days must be nonnegative")
    try:
        root = args.root.resolve()
        result = scan(root, args.today, args.max_age_days)
        path = args.baseline or root / "scripts/ci/doc-freshness-debt.json"
        baseline = json.loads(path.read_text()) if path.exists() else {}
        if not isinstance(baseline, dict):
            raise ValueError("baseline must be an identity-to-document-hash mapping")
        actual = ratchet(root, result, baseline)
        if args.record:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(actual, sort_keys=True, indent=2) + "\n")
            ratchet(root, result, actual)
        if args.format == "json":
            print(json.dumps(result, indent=2))
        elif args.format == "markdown":
            print(markdown(result), end="")
        else:
            for item in result["violations"]:
                label = "LEGACY" if item["legacy"] else "NEW"
                print(f"{item['path']}:{item['line']}: {label} {item['kind']}: {item['message']}")
            print(
                f"doc freshness: checked={result['checked']} existing-debt="
                f"{result['existing_debt']} new={result['new']}"
            )
        return int(bool(result["new"]))
    except (OSError, UnicodeError, ValueError) as error:
        print(f"doc freshness scan failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
