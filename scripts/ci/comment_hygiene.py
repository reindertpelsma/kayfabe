#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Comment-hygiene ratchet (STATUS: LIVE, 2026-11-20).

Rule (owner, 2026-11-20): code comments are read by humans and agents. Working notes about who decided what, which
session found it, or when a line was added belong in `docs/design/` and the handoff files, not in a comment. A comment
says what the code does and why it has to be that way.

The gate extracts the comments of the tracked sources (Rust `//` `///` `//!` `/* */`, C and CUDA, JavaScript, Python `#`,
shell `#`, PowerShell `#` and `<# #>`), tests and scripts included, and flags a comment line that matches a PATTERN.
Outside the scan: `docs/`, `archive/`, `third_party/`, `traces/` and `site/` (documents and evidence, not code), and the frozen
`crates/kayfabe-*` (nothing may be edited there; CLAUDE.md, Layout). Python docstrings are strings, not comments, and are not read.

Each finding is identified by its file, the pattern and the normalised comment text (not the line number, which moves), and
counted: a file may repeat a note. `scripts/ci/comment-hygiene-debt.json` is the ratchet, the findings that existed when
the gate landed. A finding that is not in it FAILS; a recorded one that no longer exists FAILS too (delete it from the file:
the ratchet only goes down). A finding is fixed by rewriting the comment, never by recording it.

    python3 scripts/ci/comment_hygiene.py                 # the gate
    python3 scripts/ci/comment_hygiene.py report          # hit counts per pattern, per language, and the worst files
    python3 scripts/ci/comment_hygiene.py --record-baseline   # creates the baseline when absent; afterwards only removes
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tokenize

ROOT = Path(__file__).resolve().parents[2]
BASELINE = "scripts/ci/comment-hygiene-debt.json"

SUFFIX_LANG = {".rs": "c-like", ".c": "c-like", ".h": "c-like", ".cu": "c-like", ".cuh": "c-like", ".js": "c-like",
               ".mjs": "c-like", ".py": "python", ".sh": "shell", ".ps1": "powershell"}
SKIP_TOP = ("docs/", "archive/", "third_party/", "traces/", "site/", "crates/kayfabe-")

# (name, regex on one comment line). Every pattern was counted on the tree first (`report`); each is a phrase that
# only a note about the working process or the owner's conversation uses, so a hit is a note, not a technical word.
PATTERNS = [
    ("owner-ruling", re.compile(r"\bowner(?:'s)?\s+(?:rul(?:ed|ing)|decid(?:ed|ion)|said|asked|approved|go\b|intent|statement)", re.I)),
    ("the-owner", re.compile(
        r"\bthe owner(?:'s)?\s+(?:decid|rul|said|ask|approv|accept|statement|earlier|constraint|intent|wish|word|instruction|quote)"
        r"|\b(?:awaits?|asks?|asked|pending|until|per|unless) the owner\b(?!\s+(?:of|must|thread|lock|guard|handle|holds|that|is|has|may|can))", re.I)),
    ("owner-dated", re.compile(r"\bowner[,:]?\s*\(?\s*20\d\d-\d\d-\d\d", re.I)),
    ("owner-rulings-doc", re.compile(r"\bOWNER_RULINGS\b")),
    ("owner-todo", re.compile(r"\b(?:TODO|FIXME|NOTE|XXX)\s*\(\s*owner", re.I)),
    ("agent-or-model", re.compile(r"\b(?:Claude|Codex|Opus|Sonnet|Haiku|ChatGPT)\b|\b(?:another|other|parallel|sibling|validation|designing|investigating|reviewing|coding|AI|LLM|image-[A-Z]|Windows-timeout) agents?\b")),
    ("agent-instructions-file", re.compile(r"\b(?:AGENTS|CLAUDE)\.md\b")),
    ("session-or-handoff-note", re.compile(r"\b(?:chat|agent|Claude|AI|coding) sessions?\b|\bSTATUS_AND_HANDOFF\b", re.I)),
    ("coordinator-note", re.compile(r"\bcoordinator(?:'s)?\s+(?:ruling|decision|first step|rule\b|asked|said|wants)|\(coordinator\b|\bcoordinator:", re.I)),
    ("as-discussed", re.compile(r"\bas (?:discussed|agreed)\b|\bwe (?:agreed|discussed|decided)\b", re.I)),
    ("added-dated", re.compile(r"\b(?:Added|Fixed|Changed|Removed|Moved|Updated|Introduced|Found|Measured|Decided|Ruled|Written|Rewritten|Corrected|Retired)\s+(?:on\s+|in\s+|at\s+)?(?:\w+\s+)?20\d\d-\d\d-\d\d")),
    ("since-dated", re.compile(r"\((?:since|from|as of|until|after|before)\s+20\d\d-\d\d-\d\d")),
    # a comment that opens with a date, or with a ★ / ⊘ / ⚠ marker and a date: an annotation stamped when it was written
    ("dated-annotation", re.compile(r"^[\s*!/>\-\u2013\u2014#\[\](\u2605\u2298\u26a0]*(?:[A-Z][\w-]*\s+){0,2}20\d\d-\d\d-\d\d")),
    ("lane-label", re.compile(r"\bLane [A-Z0-9]\b")),
]


def comments_c_like(text: str, lang: str = "c") -> list[tuple[int, str]]:
    """(line, text) of the comments of Rust (`lang="rust"`), C and CUDA (`"c"`) and JavaScript (`"js"`).

    Strings and char literals are skipped, so a `//` inside one is not a comment. Rust strings may span lines and Rust block
    comments nest; JavaScript has `'` strings and template literals, where a lone `'` is not a Rust lifetime."""
    out: list[tuple[int, str]] = []
    n, i, line = len(text), 0, 1
    quotes = "\"`'" if lang == "js" else "\""
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append((line, text[i + 2:j]))
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if lang == "rust" and text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            end = max(j - 2, i + 2) if depth == 0 else n
            for k, part in enumerate(text[i + 2:end].split("\n")):
                out.append((line + k, part))
            line += text.count("\n", i, j)
            i = j
        elif c in quotes:
            j = i + 1
            while j < n and text[j] != c:
                if text[j] == "\n":
                    if c != "`" and lang != "rust":
                        break
                    line += 1
                elif text[j] == "\\" and text[j + 1:j + 2] == "\n":
                    line += 1
                j += 2 if text[j] == "\\" else 1
            i = j + 1 if j < n and text[j] == c else j
        elif c == "'":
            # a char literal ('x', '\n'); a Rust lifetime has no closing quote and is skipped
            m = re.compile(r"'(?:\\.[^']{0,10}|[^'\\\n])'").match(text, i)
            i = m.end() if m else i + 1
        else:
            i += 1
    return out


def comments_python(text: str) -> list[tuple[int, str]]:
    try:
        return [(t.start[0], t.string[1:]) for t in tokenize.generate_tokens(io.StringIO(text).readline) if t.type == tokenize.COMMENT]
    except (tokenize.TokenError, IndentationError, SyntaxError):
        return [(i + 1, l.split("#", 1)[1]) for i, l in enumerate(text.split("\n")) if "#" in l and l.lstrip().startswith("#")]


def comments_shell(text: str, powershell: bool = False) -> list[tuple[int, str]]:
    """`#` comments outside quotes. `$#`, `${#x}` and a shebang are not comments; PowerShell also has `<# ... #>`."""
    out: list[tuple[int, str]] = []
    in_block = False
    for no, raw in enumerate(text.split("\n"), 1):
        if powershell and in_block:
            if "#>" in raw:
                out.append((no, raw.split("#>")[0]))
                in_block = False
            else:
                out.append((no, raw))
            continue
        quote = ""
        k = 0
        while k < len(raw):
            c = raw[k]
            if quote:
                if c == "\\" and quote == '"' and not powershell:
                    k += 1
                elif c == "`" and powershell and quote == '"':
                    k += 1
                elif c == quote:
                    quote = ""
            elif c in "\"'":
                quote = c
            elif powershell and raw.startswith("<#", k):
                end = raw.find("#>", k + 2)
                if end < 0:
                    out.append((no, raw[k + 2:]))
                    in_block = True
                else:
                    out.append((no, raw[k + 2:end]))
                    k = end + 1
            elif c == "#" and (k == 0 or raw[k - 1] in " \t;&|(" ) and not (no == 1 and raw.startswith("#!")):
                out.append((no, raw[k + 1:]))
                break
            k += 1
    return out


def comments_of(path: str, text: str) -> list[tuple[int, str]]:
    lang = SUFFIX_LANG[Path(path).suffix]
    if lang == "c-like":
        return comments_c_like(text, {".rs": "rust", ".js": "js", ".mjs": "js"}.get(Path(path).suffix, "c"))
    if lang == "python":
        return comments_python(text)
    return comments_shell(text, powershell=path.endswith(".ps1"))


def tracked(root: Path) -> list[str]:
    p = subprocess.run(["git", "ls-files", "-z"], cwd=root, capture_output=True, check=True)
    names = [n for n in p.stdout.decode().split("\0") if n]
    return sorted(n for n in names if Path(n).suffix in SUFFIX_LANG and not n.startswith(SKIP_TOP))


def normalise(s: str) -> str:
    return " ".join(s.split())


def identity(path: str, pattern: str, text: str, nth: int) -> str:
    digest = hashlib.sha256(normalise(text).encode()).hexdigest()[:16]
    return f"{path}|{pattern}|{digest}" + (f"|#{nth}" if nth > 1 else "")


def sweep(root: Path = ROOT, files: list[str] | None = None) -> dict[str, dict]:
    """identity -> {path, line, pattern, text} for every flagged comment line."""
    found: dict[str, dict] = {}
    for rel in files if files is not None else tracked(root):
        p = root / rel
        if not p.is_file() or p.is_symlink():
            continue
        try:
            text = p.read_text(errors="replace")
        except OSError:
            continue
        seen: collections.Counter = collections.Counter()
        for line, body in comments_of(rel, text):
            for name, rx in PATTERNS:
                if rx.search(body):
                    key = (name, normalise(body))
                    seen[key] += 1
                    found[identity(rel, name, body, seen[key])] = {"path": rel, "line": line, "pattern": name, "text": normalise(body)[:200]}
    return found


def load_baseline(root: Path) -> dict[str, str]:
    p = root / BASELINE
    return json.loads(p.read_text()) if p.exists() else {}


def findings(root: Path = ROOT, files: list[str] | None = None) -> list[tuple[str, str, str]]:
    actual = sweep(root, files)
    base = load_baseline(root)
    out = []
    for key in sorted(actual.keys() - base.keys()):
        a = actual[key]
        out.append(("NEW", key, "%s:%d [%s] %s" % (a["path"], a["line"], a["pattern"], a["text"])))
    for key in sorted(base.keys() - actual.keys()):
        out.append(("STALE_BASELINE", key, "no longer found; delete it from %s (the ratchet only goes down)" % BASELINE))
    return out


def report(root: Path = ROOT, top: int = 15) -> str:
    actual = sweep(root)
    by_pattern = collections.Counter(a["pattern"] for a in actual.values())
    by_lang = collections.Counter(SUFFIX_LANG[Path(a["path"]).suffix] + ":" + Path(a["path"]).suffix for a in actual.values())
    by_file = collections.Counter(a["path"] for a in actual.values())
    lines = ["comment-hygiene: %d flagged comment lines in %d files" % (len(actual), len(by_file)), "", "per pattern:"]
    lines += ["  %-26s %d" % (n, by_pattern.get(n, 0)) for n, _ in PATTERNS]
    lines += ["", "per file type:"] + ["  %-14s %d" % kv for kv in sorted(by_lang.items())]
    lines += ["", "worst files:"] + ["  %4d  %s" % (c, f) for f, c in by_file.most_common(top)]
    return "\n".join(lines)


def record_baseline(root: Path = ROOT) -> int:
    actual = sweep(root)
    path = root / BASELINE
    if path.exists():
        base = json.loads(path.read_text())
        actual = {k: v for k, v in actual.items() if k in base}
    doc = {k: "%s [%s] %s" % (v["path"], v["pattern"], v["text"][:90]) for k, v in sorted(actual.items())}
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, indent=1, sort_keys=True, ensure_ascii=False) + "\n")
    return len(doc)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", nargs="?", default="check", choices=("check", "report"))
    ap.add_argument("--record-baseline", action="store_true", help="create the baseline, or shrink it; never adds an entry")
    ap.add_argument("--root", type=Path, default=ROOT)
    a = ap.parse_args(argv)
    if a.record_baseline:
        print("baseline: %d flagged comment lines in %s" % (record_baseline(a.root), BASELINE))
        return 0
    if a.mode == "report":
        print(report(a.root))
        return 0
    found = findings(a.root)
    for kind, key, detail in found:
        print("%s %s: %s" % (kind, key if kind != "NEW" else "", detail))
    print("comment_hygiene: baseline=%d new=%d stale=%d" % (
        len(load_baseline(a.root)), sum(1 for f in found if f[0] == "NEW"), sum(1 for f in found if f[0] == "STALE_BASELINE")))
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
