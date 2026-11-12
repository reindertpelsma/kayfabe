"""Tree key of a commit, a port of ci/vast-broker/src/treekey.js (kf-tree-v2).

key = SHA-256('kf-tree-v2\\nignore ' + SHA-256(effective pattern lines joined by LF) + '\\n' +
      for every unignored file sorted by UTF-8 path bytes: path NUL mode NUL object-sha LF).
Fail closed: anything the JS returns null for returns None here. Parity is checked in
test_treekey.py (JS-derived vectors); change both sides together.
"""

import hashlib
import re
import subprocess

TREE_KEY_VERSION = "kf-tree-v2"
DEFAULT_IGNORE = ("/archive/", "/docs/", "/site/")
IGNORE_FILE = ".workerignore"
PROTECTED = (("Cargo.toml", False), ("Cargo.lock", False), ("rust-toolchain.toml", False), (".cargo", True),
             ("crates", True), ("scripts", True), ("ci", True), ("ci/vast-broker", True), ("qemu", True), ("cuda", True))
LIMITS = dict(ignoreBytes=65536, patterns=512, patternLength=200, entries=200000, pathBytes=4096, depth=64,
              anyPerPattern=2, steps=20000000)
ALLOWED = re.compile(r"[A-Za-z0-9._+@/*?-]+\Z")
MODES = {"100644", "100755", "120000", "160000"}
HEX40 = re.compile(r"[0-9a-f]{40}\Z")
STAR, QUESTION = 42, 63
ANY = object()


class LimitError(Exception):
    pass


class Budget:
    def __init__(self, left=LIMITS["steps"]):
        self.left = left

    def spend(self, n):
        self.left -= n
        if self.left < 0:
            raise LimitError("matcher steps")


def glob_segment(pattern, text, budget=None):
    i = j = mark = steps = 0
    star = -1
    try:
        while j < len(text):
            steps += 1
            if i < len(pattern) and pattern[i] == STAR:
                star = i
                i += 1
                mark = j
            elif i < len(pattern) and (pattern[i] == QUESTION or pattern[i] == text[j]):
                i += 1
                j += 1
            elif star >= 0:
                i = star + 1
                mark += 1
                j = mark
            else:
                return False
        while i < len(pattern) and pattern[i] == STAR:
            i += 1
        return i == len(pattern)
    finally:
        if budget:
            budget.spend(steps + 1)


def _prefixes(segments, parts, budget):
    n = len(parts)
    cur = [0] * (n + 1)
    cur[0] = 1
    for i, seg in enumerate(segments):
        nxt = [0] * (n + 1)
        if budget:
            budget.spend(n + 1)
        if seg is ANY:
            last = i == len(segments) - 1
            seen = False
            for j in range(n + 1):
                if last:
                    if seen:
                        nxt[j] = 1
                    if cur[j]:
                        seen = True
                else:
                    if cur[j]:
                        seen = True
                    if seen:
                        nxt[j] = 1
        else:
            for j in range(n):
                if cur[j] and glob_segment(seg, parts[j], budget):
                    nxt[j + 1] = 1
        cur = nxt
    return cur


def compile_pattern(line):
    """-> dict | 'skip' | None (outside the subset)."""
    if line == "" or line.startswith("#"):
        return "skip"
    if len(line) > LIMITS["patternLength"] or not ALLOWED.fullmatch(line):
        return None
    body, dir_only, anchored = line, False, False
    if body.endswith("/"):
        dir_only, body = True, body[:-1]
    if body.startswith("/"):
        anchored, body = True, body[1:]
    if body == "" or body.startswith("/") or body.endswith("/") or "//" in body:
        return None
    parts = body.split("/")
    if len(parts) > 1:
        anchored = True
    for s in parts:
        if s in (".", ".."):
            return None
        if "**" in s and s != "**":
            return None
        if s == "**" and not anchored:
            return None
    if len(parts) == 1 and parts[0] == "**":
        return None
    if parts.count("**") > LIMITS["anyPerPattern"]:
        return None
    return {"segments": [ANY if s == "**" else s.encode() for s in parts], "anchored": anchored,
            "dirOnly": dir_only, "line": line}


def parse_ignore(text):
    """-> list of patterns (each with 'line') or None."""
    if not isinstance(text, str) or len(text.encode()) > LIMITS["ignoreBytes"] or "\r" in text:
        return None
    patterns = []
    for line in text.split("\n"):
        c = compile_pattern(line)
        if c is None:
            return None
        if c != "skip":
            patterns.append(c)
        if len(patterns) > LIMITS["patterns"]:
            return None
    return patterns


def ignored(patterns, path, directory=False, budget=None):
    parts = [s.encode() for s in path.split("/")]
    depth = len(parts)
    if depth > LIMITS["depth"]:
        raise LimitError("path depth")
    counts = lambda p, n: (not p["dirOnly"]) or n < depth or directory  # noqa: E731
    for p in patterns:
        if p["anchored"]:
            reach = _prefixes(p["segments"], parts, budget)
            for n in range(1, depth + 1):
                if reach[n] and counts(p, n):
                    return True
        else:
            for n in range(1, depth + 1):
                if counts(p, n) and glob_segment(p["segments"][0], parts[n - 1], budget):
                    return True
    return False


def guarded(patterns):
    if patterns is None:
        return None
    for path, is_dir in PROTECTED:
        if ignored(patterns, path, is_dir):
            return None
    return patterns


def patterns_for(text):
    """text None = no .workerignore in the commit (built-in default)."""
    return guarded(parse_ignore("\n".join(DEFAULT_IGNORE) if text is None else text))


def _valid_path(path):
    return (
        isinstance(path, str) and path != "" and len(path.encode()) <= LIMITS["pathBytes"]
        and not path.startswith("/") and not path.endswith("/") and "//" not in path
        and not re.search("[\0\n�]", path)
        and not any(s in (".", "..", ".git") for s in path.split("/"))
    )


def tree_key_of(entries, patterns):
    """entries: [(path, mode, sha)]. Returns the hex key or None."""
    if patterns is None:
        return None
    kept, seen, budget = [], set(), Budget()
    try:
        for path, mode, sha in entries:
            if not _valid_path(path) or mode not in MODES or not HEX40.fullmatch(sha or ""):
                return None
            if path in seen:
                return None
            seen.add(path)
            if not ignored(patterns, path, mode == "160000", budget):
                kept.append((path, mode, sha))
    except LimitError:
        return None
    if not kept:
        return None
    kept.sort(key=lambda e: e[0].encode())
    digest = hashlib.sha256("\n".join(p["line"] for p in patterns).encode()).hexdigest()
    text = TREE_KEY_VERSION + "\nignore " + digest + "\n"
    for path, mode, sha in kept:
        text += path + "\0" + mode + "\0" + sha + "\n"
    return hashlib.sha256(text.encode()).hexdigest()


def commit_tree_key(git, commit):
    """Compute the key of `commit` from a local checkout/repo through `git(args) -> bytes`. None on any failure."""
    try:
        raw = git(["ls-tree", "-r", "-z", "--full-tree", commit])
        entries = []
        for rec in raw.split(b"\0"):
            if not rec:
                continue
            meta, path = rec.split(b"\t", 1)
            mode, _type, sha = meta.decode().split(" ")
            entries.append((path.decode("utf-8"), mode, sha))
            if len(entries) > LIMITS["entries"]:
                return None
        text = None
        for path, mode, sha in entries:
            if path == IGNORE_FILE:
                if mode not in ("100644", "100755"):
                    return None
                text = git(["cat-file", "blob", sha]).decode("utf-8")
        return tree_key_of(entries, patterns_for(text))
    except (ValueError, UnicodeError, subprocess.SubprocessError, OSError):
        return None
