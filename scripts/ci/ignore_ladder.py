#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The ignore ladder: .gitignore  is a subset of  .workerignore  is a subset of  .dockerignore.

STATUS: LIVE, 2026-11-09 (the matcher, the strip step and the stripped-tree key are wired into CI and
the box payload; evidence of the first build in traces/ignore_ladder_20261105/README.md).

Why: the Vast box clones the repo and then removes everything that matches `.workerignore`, and a
test result is keyed by the hash of the sorted unignored tracked files (path, mode, blob id). A
file that is ignored for the box can therefore neither be read by a box test nor change a result
key. Three tools read these files (git, docker and this script's matcher), so the files use a
restricted syntax on which all three agree.

Restricted syntax (every non-blank, non-comment line):
  - starts with `/` (anchored at the repository root) or `**/` (any depth);
  - no negation (`!`), no trailing `/`, no `\\`, `[`, `]`, `?`, no spaces, no non-ASCII;
  - a segment is made of [A-Za-z0-9_.+@=,~-] and `*`; `**` is only a whole segment, and only
    leading or in the middle (never last);
  - a comment is a line that starts with `#`; blank lines are allowed; no trailing whitespace.
Those lines mean the same thing to git (gitignore), to docker (.dockerignore) and to `matches`.
Unanchored names (`foo`, `*.md`) are NOT allowed: git reads them at any depth, docker only at
the root.

Subcommands:
  check            ladder + syntax + (matcher == git) + (docker semantics == git) on the real tree
  hash REV         filtered-tree hash of REV under the ignore file
  chain REV N      hash of the N first-parent commits ending at REV, and which fall through
  sparse           the sparse-checkout list equivalent to the ignore file (for the box side)
  docker REV       build a real docker context from REV with the ignore file as .dockerignore and
                   compare the files docker copies with the matcher (needs a working docker)
  strip --in-place delete, in the working tree, every tracked file that the commit's OWN
                   .workerignore matches (never .git); the box and the stripped-tree CI job call it
  strip --dest DIR export the commit minus the ignored files into an empty DIR
  treekey [REV]    the kf-tree-v2 key of REV (ci/vast-broker/src/treekey.js computes the same bytes)
  keycheck [REV]   fail unless node and treekey.js print that same key

`matches` below is the ONE matcher: the strip step, the tree key and the ladder lint all call it, and
the Worker's JavaScript matcher is cross-checked against it (ci/vast-broker/test/ladder-parity.test.js).
"""

import argparse
import functools
import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

SEG_OK = re.compile(r"^[A-Za-z0-9_.+@=,~*-]+$")


class FailClosed(Exception):
    """Anything that must mean "run, do not skip, do not strip" (main prints it and exits 3)."""


class SyntaxProblem(FailClosed):
    pass


def lint_line(line):
    """Return None for blank/comment lines, the line itself for a valid pattern; else raise."""
    if line == "" or line.startswith("#"):
        return None
    if line != line.rstrip():
        raise SyntaxProblem("trailing whitespace")
    if not line.isascii():
        raise SyntaxProblem("non-ASCII")
    if line.startswith("!"):
        raise SyntaxProblem("negation is not allowed")
    if not (line.startswith("/") or line.startswith("**/")):
        raise SyntaxProblem("must start with / or **/ (git and docker disagree on bare names)")
    if line.endswith("/"):
        raise SyntaxProblem("trailing / is not allowed (git: directory only, docker: either)")
    segs = line.lstrip("/").split("/") if line.startswith("/") else line.split("/")
    if line.startswith("/") and line.startswith("//"):
        raise SyntaxProblem("empty segment")
    for i, seg in enumerate(segs):
        if seg in ("", ".", ".."):
            raise SyntaxProblem("empty, . or .. segment")
        if seg == "**":
            if i == len(segs) - 1:
                raise SyntaxProblem("`**` must not be the last segment")
            continue
        if "**" in seg:
            raise SyntaxProblem("`**` must be a whole segment")
        if not SEG_OK.match(seg):
            raise SyntaxProblem("character outside [A-Za-z0-9_.+@=,~-*] in segment %r" % seg)
    return line


def read_patterns(text, name="<ignore>"):
    """Parse an ignore file; return the list of valid pattern lines. Raises on the first bad line."""
    out = []
    for n, line in enumerate(text.split("\n"), 1):
        if n == len(text.split("\n")) and line == "":
            continue
        try:
            p = lint_line(line)
        except SyntaxProblem as e:
            raise SyntaxProblem("%s:%d: %s: %r" % (name, n, e, line)) from None
        if p is not None:
            out.append(p)
    return out


# ---- matcher 1: segment recursion (ours) ---------------------------------------------------------
@functools.lru_cache(maxsize=None)
def _seg_regex(seg):
    return re.compile("^" + "".join("[^/]*" if c == "*" else re.escape(c) for c in seg) + "$")


def _split(pattern):
    segs = pattern.split("/")
    # "/a/b" -> ["", "a", "b"]: anchored, drop the empty head. "**/a" -> ["**", "a"].
    return segs[1:] if segs[0] == "" else segs


def _match_segs(psegs, comps):
    if not psegs:
        return not comps
    head, rest = psegs[0], psegs[1:]
    if head == "**":
        return any(_match_segs(rest, comps[k:]) for k in range(len(comps) + 1))
    return bool(comps) and bool(_seg_regex(head).match(comps[0])) and _match_segs(rest, comps[1:])


def matches(patterns, path):
    """True if some pattern matches the path or one of its parent directories."""
    patterns = tuple(patterns)
    comps = path.split("/")
    return any(_dir_matches(patterns, "/".join(comps[:k])) for k in range(1, len(comps) + 1))


@functools.lru_cache(maxsize=None)
def _dir_matches(patterns, prefix):
    comps = prefix.split("/")
    return any(_match_segs(_split(pat), comps) for pat in patterns)


# ---- matcher 2: docker semantics, written from the documented algorithm (regex conversion) ------
def docker_regex(pattern):
    """moby/patternmatcher: Clean, strip the leading /, `**` + / -> (.*/)?, `*` -> [^/]*."""
    pat = pattern[1:] if pattern.startswith("/") else pattern
    out, i = "^", 0
    while i < len(pat):
        c = pat[i]
        if pat.startswith("**/", i):
            out += "(.*/)?"
            i += 3
            continue
        if c == "*":
            out += "[^/]*"
        else:
            out += re.escape(c)
        i += 1
    return re.compile(out + "$")


@functools.lru_cache(maxsize=None)
def _docker_regs(patterns):
    return [docker_regex(p) for p in patterns]


@functools.lru_cache(maxsize=None)
def _docker_dir(patterns, prefix):
    return any(r.match(prefix) for r in _docker_regs(patterns))


def docker_matches(patterns, path):
    """Docker excludes a path when the pattern matches it or any parent directory."""
    patterns = tuple(patterns)
    comps = path.split("/")
    return any(_docker_dir(patterns, "/".join(comps[:k])) for k in range(1, len(comps) + 1))


# ---- git -----------------------------------------------------------------------------------------
def git(*args, cwd=None, inp=None, check=True):
    p = subprocess.run(["git", *args], cwd=cwd, input=inp, capture_output=True, check=False)
    if check and p.returncode not in (0,):
        raise RuntimeError(
            "git %s failed rc=%d: %s" % (" ".join(args), p.returncode, p.stderr[-300:])
        )
    return p


def repo_root():
    return git("rev-parse", "--show-toplevel").stdout.decode().strip()


def tracked(rev=None, cwd=None):
    """[(mode, sha, path)] of the tree (index when rev is None). Gitlinks included."""
    if rev is None:
        raw = git("ls-files", "-s", "-z", cwd=cwd).stdout
        ents = []
        for rec in raw.split(b"\0"):
            if rec:
                meta, path = rec.split(b"\t", 1)
                mode, sha, _stage = meta.split(b" ")
                ents.append((mode.decode(), sha.decode(), os.fsdecode(path)))
        return ents
    raw = git("ls-tree", "-r", "-z", "--full-tree", rev, cwd=cwd).stdout
    ents = []
    for rec in raw.split(b"\0"):
        if rec:
            meta, path = rec.split(b"\t", 1)
            mode, _type, sha = meta.split(b" ")
            ents.append((mode.decode(), sha.decode(), os.fsdecode(path)))
    return ents


def filtered_hash(entries, patterns):
    """SHA-256 over sorted `path NUL mode NUL blob-id LF` of the unignored entries.

    Same line shape as docs/design/V3_PLATFORM_TESTING.md section 4 (boundary hash).
    Returns (hex digest, number of unignored entries)."""
    keep = [(os.fsencode(p), m, s) for m, s, p in entries if not matches(patterns, p)]
    keep.sort()
    h = hashlib.sha256()
    for p, m, s in keep:
        h.update(p + b"\0" + m.encode() + b"\0" + s.encode() + b"\n")
    return h.hexdigest(), len(keep)


def git_ignored(paths, patterns_file, scratch):
    """Paths git itself considers ignored under ONLY this file (a scratch repo, no .gitignore)."""
    git("init", "-q", scratch)
    inp = b"\0".join(os.fsencode(p) for p in paths) + b"\0"
    p = git(
        "-c",
        "core.excludesFile=" + os.path.abspath(patterns_file),
        "check-ignore",
        "--no-index",
        "--stdin",
        "-z",
        cwd=scratch,
        inp=inp,
        check=False,
    )
    if p.returncode not in (0, 1):
        raise RuntimeError("git check-ignore rc=%d: %s" % (p.returncode, p.stderr[-300:]))
    return {os.fsdecode(x) for x in p.stdout.split(b"\0") if x}


def docker_context_files(rev, ignore_text, workdir, cwd=None, docker="docker"):
    """Files a real `docker build` copies from `git archive REV` when `ignore_text` is the
    .dockerignore. Returns a set of relative paths (files only)."""
    ctx = os.path.join(workdir, "ctx")
    out = os.path.join(workdir, "out")
    os.makedirs(ctx)
    arch = subprocess.run(["git", "archive", rev], cwd=cwd, capture_output=True, check=True).stdout
    subprocess.run(["tar", "-x", "-C", ctx], input=arch, check=True)
    with open(os.path.join(ctx, ".dockerignore"), "w", encoding="utf-8") as f:
        f.write(ignore_text)
    dockerfile = os.path.join(workdir, "Dockerfile.ladder")
    with open(dockerfile, "w") as f:
        f.write("FROM scratch\nCOPY . /\n")
    subprocess.run(
        [
            docker,
            "build",
            "--no-cache",
            "-q",
            "-f",
            dockerfile,
            "-o",
            "type=local,dest=" + out,
            ctx,
        ],
        check=True,
        capture_output=True,
    )
    got = set()
    for d, _dirs, files in os.walk(out):
        for fn in files:
            got.add(os.path.relpath(os.path.join(d, fn), out))
    return got


def sparse_list(patterns):
    return ["/*"] + ["!" + p for p in patterns]


# ---- check ---------------------------------------------------------------------------------------
# Paths each file must itself ignore (semantically: some line must match them).
LADDER_SELF = {
    ".workerignore": [".git", ".git/HEAD", ".gitignore", "sub/.gitignore", ".workerignore"],
    ".dockerignore": [".dockerignore"],
}


def check(root, rev=None):
    problems = []
    texts = {}
    pats = {}
    for name in (".gitignore", ".workerignore", ".dockerignore"):
        path = os.path.join(root, name)
        if not os.path.exists(path):
            problems.append("%s is missing" % name)
            continue
        with open(path, encoding="utf-8") as f:
            texts[name] = f.read()
        try:
            pats[name] = read_patterns(texts[name], name)
        except SyntaxProblem as e:
            problems.append("syntax: %s" % e)
    if problems:
        return problems
    g, w, d = (pats[n] for n in (".gitignore", ".workerignore", ".dockerignore"))
    for lo, hi, ln, hn in (
        (g, w, ".gitignore", ".workerignore"),
        (w, d, ".workerignore", ".dockerignore"),
    ):
        for line in lo:
            if line not in hi:
                problems.append("ladder: %r is in %s but not in %s" % (line, ln, hn))
    for name, need in LADDER_SELF.items():
        for path in need:
            if not matches(pats[name], path):
                problems.append("self-ignore: %s must ignore %s" % (name, path))
    for name in pats:
        if len(set(pats[name])) != len(pats[name]):
            problems.append("duplicate lines in %s" % name)
    ents = tracked(rev, cwd=root)
    paths = [p for _m, _s, p in ents]
    with tempfile.TemporaryDirectory() as tmp:
        for name in (".gitignore", ".workerignore", ".dockerignore"):
            ours = {p for p in paths if matches(pats[name], p)}
            dock = {p for p in paths if docker_matches(pats[name], p)}
            gi = git_ignored(paths, os.path.join(root, name), os.path.join(tmp, "s" + name))
            print(
                "IGNORE_LADDER_COUNT %s tracked=%d ignored(ours)=%d ignored(git)=%d ignored(docker-semantics)=%d"
                % (name, len(paths), len(ours), len(gi), len(dock))
            )
            for label, got in (("matcher", ours), ("docker-semantics", dock)):
                if got != gi:
                    extra = sorted(got - gi)[:3]
                    miss = sorted(gi - got)[:3]
                    problems.append(
                        "%s: %s differs from git on the tree (ours-only %s, git-only %s)"
                        % (name, label, extra, miss)
                    )
        if rev is None:
            # Second, independent git entry point: tracked files matching an exclude file.
            for name in (".workerignore", ".dockerignore"):
                p = git(
                    "ls-files", "-z", "-ci", "--exclude-from=" + os.path.join(root, name), cwd=root
                )
                lsf = {os.fsdecode(x) for x in p.stdout.split(b"\0") if x}
                ours = {x for x in paths if matches(pats[name], x)}
                if lsf != ours:
                    problems.append(
                        "%s: ls-files -ci --exclude-from differs from the matcher" % name
                    )
    # The effective sets nest, whatever the lines say (no negation, so this follows from lines;
    # asserted on the real tree as a guard against a matcher bug).
    sets = [
        {p for p in paths if matches(pats[n], p)}
        for n in (".gitignore", ".workerignore", ".dockerignore")
    ]
    if not (sets[0] <= sets[1] <= sets[2]):
        problems.append("effective ignored sets do not nest on the tree")
    # What the strip step would refuse, the lint refuses on every push (floor, nothing left).
    try:
        strip_plan(ents, pats[".workerignore"])
    except FailClosed as e:
        problems.append("strip: %s" % e)
    # A tracked file ignored by .gitignore is a contradiction worth naming.
    if sets[0]:
        problems.append("tracked files match .gitignore: %s" % sorted(sets[0])[:3])
    return problems


# ---- the strip step and the tree key ---------------------------------------------------------------
# What a stripped tree must keep whatever .workerignore says (OWNER_RULINGS section AQ: every test of
# production code runs on the tree left after the strip). Deleting a test file with an ignore line
# would turn it into a test that never ran, so the strip REFUSES (fail closed) when a tracked,
# non-prose path under these roots is ignored. ci/vast-broker/src/treekey.js PROTECTED guards whole
# roots only; this guards every file below them. The two lists are compared in
# ci/vast-broker/test/ladder-parity.test.js.
KEEP_DIRS = (
    "crates",
    "qemu",
    "cuda",
    "firmware",
    "tools",
    ".cargo",
    "third_party/ogkm",
    "third_party/ogkm-580",
)
KEEP_FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "clippy.toml",
    ".gitmodules",
    ".dockerignore",
)
KEEP_NAME = re.compile(r"^(Dockerfile[A-Za-z0-9_.-]*|docker-compose[A-Za-z0-9_.-]*\.ya?ml)$")
# Roots of treekey.js PROTECTED that the floor does NOT cover file by file, and why. A root absent
# from KEEP_DIRS and from this table is a drift the parity test reports.
PROTECTED_NOT_FLOORED = {
    "scripts": "partly ignored on purpose (CI self-tests, local gates): scripts/ci/test_*.py ...",
    "ci": "the Worker trees under ci/ are ignored on purpose: they run on the runner, never on a box",
    "ci/vast-broker": "ignored by .workerignore, REFUSED by treekey.js PROTECTED: see the header of .workerignore",
}


def is_kept_code(path):
    """A path the strip step may never remove. Prose (.md) and .gitignore files are excepted: a
    test that reads one fails loudly on the stripped tree, which is how an exception is found."""
    if path.endswith(".md") or path.rsplit("/", 1)[-1] == ".gitignore":
        return False
    if path in KEEP_FILES or KEEP_NAME.match(path):
        return True
    return any(path == d or path.startswith(d + "/") for d in KEEP_DIRS)


def floor_violations(paths, patterns):
    return sorted(p for p in paths if is_kept_code(p) and matches(patterns, p))


def ignore_text_at(rev, root):
    """The commit's OWN .workerignore, byte for byte; a missing file is fail closed."""
    p = git("show", "%s:.workerignore" % rev, cwd=root, check=False)
    if p.returncode != 0:
        raise FailClosed("%s has no .workerignore in its own tree" % rev)
    try:
        return p.stdout.decode("utf-8")
    except UnicodeDecodeError:
        raise FailClosed("%s: .workerignore is not UTF-8" % rev) from None


def strip_plan(entries, patterns):
    """(kept, removed) entries; fail closed if nothing is left or the floor is violated."""
    if not entries:
        raise FailClosed("empty tree listing (truncated or unreadable)")
    kept = [e for e in entries if not matches(patterns, e[2])]
    removed = [e for e in entries if matches(patterns, e[2])]
    if not kept:
        raise FailClosed("nothing would be left after stripping")
    bad = floor_violations([e[2] for e in entries], patterns)
    if bad:
        raise FailClosed(
            "the ignore set removes code the box and the production tests need: %s%s"
            % (", ".join(bad[:5]), " (+%d more)" % (len(bad) - 5) if len(bad) > 5 else "")
        )
    return kept, removed


def tree_key(entries, pattern_lines, patterns=None):
    """The key ci/vast-broker/src/treekey.js computes (treeKeyOf): SHA-256 over
    'kf-tree-v2\\nignore <SHA-256 of the effective lines joined by LF>\\n' and, for every unignored
    entry sorted by the UTF-8 bytes of its path, 'path NUL mode NUL object-sha LF'."""
    patterns = pattern_lines if patterns is None else patterns
    keep = sorted((os.fsencode(p), m, s) for m, s, p in entries if not matches(tuple(patterns), p))
    if not keep:
        raise FailClosed("no unignored file")
    digest = hashlib.sha256("\n".join(pattern_lines).encode()).hexdigest()
    h = hashlib.sha256()
    h.update(b"kf-tree-v2\nignore " + digest.encode() + b"\n")
    for p, m, s in keep:
        h.update(p + b"\0" + m.encode() + b"\0" + s.encode() + b"\n")
    return h.hexdigest()


def _inside(root, full):
    r = os.path.realpath(root)
    return os.path.commonpath([r, os.path.realpath(os.path.dirname(full))]) == r


def strip_in_place(root, rev="HEAD"):
    """Delete from the working tree every tracked file the commit's own .workerignore matches.

    Never touches .git, never follows a symlink, deletes only paths that `git ls-tree` lists for
    REV. Returns (kept, removed, key)."""
    pats = read_patterns(ignore_text_at(rev, root), "%s:.workerignore" % rev)
    ents = tracked(rev, cwd=root)
    kept, removed = strip_plan(ents, pats)
    key = tree_key(ents, pats)
    delete_entries(root, removed)
    return len(kept), len(removed), key


def delete_entries(root, removed):
    """Remove the (mode, sha, path) entries from the working tree under ROOT, then the directories
    they leave empty. Every path is checked BEFORE the first deletion: one bad path deletes nothing.
    """
    plan = []
    for mode, _sha, path in removed:
        parts = path.split("/")
        if ".git" in parts or path.startswith("/") or ".." in parts or "" in parts:
            raise FailClosed("refusing to delete %r" % path)
        full = os.path.join(root, path)
        if not _inside(root, full):
            raise FailClosed("refusing to delete outside the tree: %r" % path)
        plan.append((mode, path, full))
    dirs = set()
    for mode, path, full in plan:
        try:
            st = os.lstat(full)
        except FileNotFoundError:
            continue  # sparse checkout: never materialised
        is_dir = stat.S_ISDIR(st.st_mode)
        if mode == "160000" and is_dir:
            shutil.rmtree(full)
        elif is_dir:
            raise FailClosed("%r is a directory on disk but a file in the tree" % path)
        else:
            os.remove(full)  # a symlink is removed itself, never followed
        dirs.add(os.path.dirname(path))
    # Prune the directories that became empty (deepest first), but never above the root.
    for d in sorted({x for d in dirs for x in _parents(d)}, key=lambda x: -x.count("/")):
        try:
            os.rmdir(os.path.join(root, d))
        except OSError:
            pass


def _parents(d):
    out = []
    while d:
        out.append(d)
        d = os.path.dirname(d)
    return out


def strip_export(rev, dest, root):
    """Export REV into the empty directory DEST without the ignored files (no .git)."""
    pats = read_patterns(ignore_text_at(rev, root), "%s:.workerignore" % rev)
    ents = tracked(rev, cwd=root)
    kept, removed = strip_plan(ents, pats)
    key = tree_key(ents, pats)
    os.makedirs(dest, exist_ok=True)
    if os.listdir(dest):
        raise FailClosed("destination %s is not empty" % dest)
    raw = git("archive", rev, cwd=root).stdout
    got = 0
    with tarfile.open(fileobj=io.BytesIO(raw)) as tar:
        for m in tar:
            name = m.name.rstrip("/")
            if m.type == tarfile.XGLTYPE or not name:
                continue
            if matches(pats, name):
                continue
            tar.extract(m, dest, filter="data")
            got += 0 if m.isdir() else 1
    # Submodule paths are directories in the archive and gitlinks in the listing.
    want = len([e for e in kept if e[0] != "160000"])
    if got != want:
        raise FailClosed("exported %d files, expected %d" % (got, want))
    return len(kept), len(removed), key


def keycheck(rev, root):
    """The Python key and the Worker's treekey.js key of REV must be the same bytes."""
    pats = read_patterns(ignore_text_at(rev, root), "%s:.workerignore" % rev)
    mine = tree_key(tracked(rev, cwd=root), pats)
    helper = os.path.join(root, "ci/vast-broker/scripts/tree-key.mjs")
    if not os.path.exists(helper):
        raise FailClosed("keycheck needs the full tree: %s is absent" % helper)
    p = subprocess.run(["node", helper, rev], cwd=root, capture_output=True, text=True, check=False)
    theirs = p.stdout.strip()
    print("IGNORE_KEY python=%s node=%s" % (mine, theirs))
    if p.returncode != 0 or mine != theirs:
        print("IGNORE_KEY_MISMATCH rc=%d %s" % (p.returncode, p.stderr[-300:]))
        return 1
    print("IGNORE_KEY OK")
    return 0


def first_parents(rev, n, cwd=None):
    out = git("rev-list", "--first-parent", "-n", str(n), rev, cwd=cwd).stdout.decode().split()
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=None)
    ap.add_argument("--ignore", default=".workerignore", help="ignore file (default .workerignore)")
    ap.add_argument("--ignore-rev", default=None, help="read the ignore file from this revision")
    ap.add_argument(
        "--own-ignore",
        action="store_true",
        help="hash/chain: each revision is filtered by the ignore file in its OWN tree "
        "(fail closed: a missing or unparsable file is an error, exit 3, meaning 'run')",
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    h = sub.add_parser("hash")
    h.add_argument("rev")
    c = sub.add_parser("chain")
    c.add_argument("rev")
    c.add_argument("n", type=int)
    sub.add_parser("sparse")
    dk = sub.add_parser("docker")
    dk.add_argument("rev")
    st = sub.add_parser("strip")
    st.add_argument("rev", nargs="?", default="HEAD")
    st.add_argument("--in-place", action="store_true")
    st.add_argument("--dest")
    tk = sub.add_parser("treekey")
    tk.add_argument("rev", nargs="?", default="HEAD")
    sub.add_parser("matchjson")
    sub.add_parser("floorjson")
    kc = sub.add_parser("keycheck")
    kc.add_argument("rev", nargs="?", default="HEAD")
    a = ap.parse_args(argv)
    root = a.root or (None if a.cmd in ("matchjson", "floorjson") else repo_root())

    def load():
        if a.ignore_rev:
            text = git("show", "%s:%s" % (a.ignore_rev, a.ignore), cwd=root).stdout.decode()
        else:
            with open(os.path.join(root, a.ignore), encoding="utf-8") as f:
                text = f.read()
        return read_patterns(text, a.ignore)

    def pats_for(rev):
        if not a.own_ignore:
            return load()
        p = git("show", "%s:%s" % (rev, a.ignore), cwd=root, check=False)
        if p.returncode != 0:
            raise SyntaxProblem("%s has no %s in its own tree" % (rev, a.ignore))
        return read_patterns(p.stdout.decode(), "%s:%s" % (rev, a.ignore))

    try:
        return run(a, root, pats_for, load)
    except FailClosed as e:
        print("IGNORE_LADDER_FAIL_CLOSED %s" % e)
        return 3


def run(a, root, pats_for, load):
    if a.cmd == "check":
        probs = check(root)
        for p in probs:
            print("IGNORE_LADDER_PROBLEM " + p)
        print("IGNORE_LADDER %s" % ("FAIL" if probs else "OK"))
        return 1 if probs else 0
    if a.cmd == "hash":
        pats = pats_for(a.rev)
        d, n = filtered_hash(tracked(a.rev, cwd=root), pats)
        print("%s %d %s" % (d, n, a.rev))
        return 0
    if a.cmd == "chain":
        revs = first_parents(a.rev, a.n + 1, cwd=root)  # one extra: the parent of the oldest
        hashes = []
        for r in revs:
            d, n = filtered_hash(tracked(r, cwd=root), pats_for(r))
            hashes.append((r, d, n))
        fell = 0
        for i in range(len(hashes) - 1):
            r, d, n = hashes[i]
            same = d == hashes[i + 1][1]
            fell += same
            print("%s %s n=%d %s" % (r[:12], d[:16], n, "FALLS_THROUGH" if same else "new"))
        print("CHAIN commits=%d fell_through=%d" % (len(hashes) - 1, fell))
        return 0
    if a.cmd == "docker":
        pats = load()
        with open(os.path.join(root, a.ignore), encoding="utf-8") as f:
            text = f.read()
        ents = tracked(a.rev, cwd=root)
        want = {p for m, _s, p in ents if m != "160000" and not docker_matches(pats, p)}
        with tempfile.TemporaryDirectory() as tmp:
            got = docker_context_files(a.rev, text, tmp, cwd=root)
        # The Dockerfile and .dockerignore are always sent to the daemon; the temp .dockerignore
        # is ours, so ignore it in the comparison unless the ignore file lists itself.
        got.discard(".dockerignore")
        want.discard(".dockerignore")
        print(
            "DOCKER_CONTEXT tracked=%d expected_copied=%d docker_copied=%d"
            % (len(ents), len(want), len(got))
        )
        bad = sorted(got ^ want)
        for x in bad[:20]:
            print("DOCKER_DIFF %s %s" % ("docker-only" if x in got else "matcher-only", x))
        print("DOCKER_CONTEXT %s" % ("OK" if not bad else "FAIL"))
        return 1 if bad else 0
    if a.cmd == "sparse":
        print("\n".join(sparse_list(load())))
        return 0
    if a.cmd == "strip":
        if a.in_place == bool(a.dest):
            raise FailClosed("strip needs exactly one of --in-place and --dest DIR")
        if a.in_place:
            kept, removed, key = strip_in_place(root, a.rev)
        else:
            kept, removed, key = strip_export(a.rev, a.dest, root)
        print("IGNORE_STRIP kept=%d removed=%d rev=%s" % (kept, removed, a.rev))
        print("IGNORE_STRIP_KEY %s" % key)
        return 0
    if a.cmd == "treekey":
        pats = read_patterns(ignore_text_at(a.rev, root), "%s:.workerignore" % a.rev)
        print(tree_key(tracked(a.rev, cwd=root), pats))
        return 0
    if a.cmd == "keycheck":
        return keycheck(a.rev, root)
    if a.cmd == "matchjson":
        # stdin {"patterns": [line...], "paths": [...], "entries": [[mode, sha, path]...]}; stdout the
        # matcher's verdicts (and the tree key when entries are given). Cross-check input for the
        # Worker's JavaScript matcher; a pattern line outside the subset is fail closed.
        req = json.load(sys.stdin)
        pats = read_patterns("\n".join(req["patterns"]) + "\n", "<stdin>")
        out = {"ignored": [matches(pats, x) for x in req.get("paths", [])]}
        if req.get("entries"):
            out["key"] = tree_key([tuple(e) for e in req["entries"]], pats)
        print(json.dumps(out))
        return 0
    if a.cmd == "floorjson":
        print(
            json.dumps(
                {
                    "keep_dirs": list(KEEP_DIRS),
                    "keep_files": list(KEEP_FILES),
                    "not_floored": sorted(PROTECTED_NOT_FLOORED),
                }
            )
        )
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
