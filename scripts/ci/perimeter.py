#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The memory-safety perimeter gates (docs/design/V3_SEC_PERIMETER.md; OWNER_RULINGS §R).

Subcommands, each exiting nonzero with `RULE path:line: message` lines on a violation:

  lex        G3, the tokenizer gate: L1-L6 and L10 over every tracked `.rs` file.
  manifest   G4, the manifest half: M1-M5, manifest coverage, pinned build scripts.
  metadata   G4, the resolved-graph half: M7-M9 (`cargo metadata --locked`).

Every rule has known-positive fixtures in `scripts/ci/test_unsafe_gates.py`, which runs in
the `stable` job's discover step: a gate here that stops firing turns that suite red.

The one list is `scripts/ci/perimeter.toml`. Nothing below enumerates crates.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rslex  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
UNSAFE = rslex.KEYWORD_UNSAFE
EXCLUDED_TOPS = ("archive/", "third_party/")


# ---------------------------------------------------------------------------------------
# configuration
# ---------------------------------------------------------------------------------------


@dataclass
class Config:
    raw: dict

    @classmethod
    def load(cls, root: Path) -> "Config":
        return cls(tomllib.loads((root / "scripts/ci/perimeter.toml").read_text()))

    def crates(self, cls_: str) -> list[str]:
        return [c["path"] for c in self.raw.get("crates", []) if c.get("class") == cls_]

    @property
    def class_u(self) -> list[str]:
        return self.crates("U")

    @property
    def class_p(self) -> list[str]:
        return self.crates("P")

    @property
    def exempt(self) -> list[str]:
        return [e["path"] for e in self.raw.get("exempt", [])]

    @property
    def standalone(self) -> list[dict]:
        return list(self.raw.get("standalone", []))

    def kf3_crates(self) -> list[str]:
        return [c["path"] for c in self.raw.get("crates", []) if c.get("kf3")]


def under(path: str, prefix: str) -> bool:
    return path == prefix or path.startswith(prefix.rstrip("/") + "/")


def crate_of(path: str, crates: list[str]) -> str | None:
    best = None
    for c in crates:
        if under(path, c) and (best is None or len(c) > len(best)):
            best = c
    return best


def in_src(path: str, crate: str) -> bool:
    return under(path, f"{crate}/src")


def is_perimeter_file(path: str, cfg: Config) -> bool:
    """A `*_unsafe.rs` under a class U or P crate's `src/` (at any depth)."""
    if not path.endswith("_unsafe.rs"):
        return False
    c = crate_of(path, cfg.class_u + cfg.class_p)
    return c is not None and in_src(path, c)


def may_hold_unsafe(path: str, cfg: Config) -> bool:
    """L1: a class U perimeter file, or anything under an exempt path."""
    if any(under(path, e) for e in cfg.exempt):
        return True
    if not path.endswith("_unsafe.rs"):
        return False
    c = crate_of(path, cfg.class_u)
    return c is not None and in_src(path, c)


def git_ls(root: Path, *patterns: str) -> list[str]:
    out = subprocess.run(["git", "ls-files", "-z", "--", *patterns], cwd=root,
                         capture_output=True, check=True).stdout.decode()
    return sorted(f for f in out.split("\0") if f and not f.startswith(EXCLUDED_TOPS))


@dataclass(frozen=True, order=True)
class Finding:
    rule: str
    path: str
    line: int
    msg: str

    def __str__(self) -> str:
        return f"{self.rule} {self.path}:{self.line}: {self.msg}"


class Tree:
    """Tokenized files, cached. `files` are repo-relative POSIX paths."""

    def __init__(self, root: Path, files: list[str]):
        self.root = root
        self.files = files
        self._toks: dict[str, list[rslex.Tok]] = {}
        self._struct: dict[str, rslex.Structure] = {}

    def toks(self, f: str) -> list[rslex.Tok]:
        if f not in self._toks:
            self._toks[f] = rslex.tokenize_file(self.root / f, f)
        return self._toks[f]

    def struct(self, f: str) -> rslex.Structure:
        if f not in self._struct:
            self._struct[f] = rslex.Structure(self.toks(f), f)
        return self._struct[f]


# ---------------------------------------------------------------------------------------
# G3: the tokenizer gate
# ---------------------------------------------------------------------------------------

CRATE_ROOT_NAMES = ("lib.rs", "main.rs", "build.rs")
CRATE_ROOT_DIRS = ("src/bin", "tests", "examples", "benches")


def is_crate_root(path: str, manifest_roots: set[str]) -> bool:
    p = PurePosixPath(path)
    if path in manifest_roots or p.name in CRATE_ROOT_NAMES:
        return True
    parent = str(p.parent)
    return any(parent.endswith(d) or f"/{d}/" in f"/{parent}/" and p.name == "main.rs"
               for d in CRATE_ROOT_DIRS)


def manifest_roots(root: Path, manifests: list[str]) -> set[str]:
    """Explicit `[lib]`/`[[bin]]`/`[[test]]`/... paths named by tracked manifests."""
    out: set[str] = set()
    for m in manifests:
        d = PurePosixPath(m).parent
        data = tomllib.loads((root / m).read_text())
        targets = [data.get("lib") or {}]
        for key in ("bin", "test", "example", "bench"):
            targets.extend(data.get(key) or [])
        for t in targets:
            if isinstance(t, dict) and "path" in t:
                out.add(str(d / t["path"]))
        b = (data.get("package") or {}).get("build")
        if isinstance(b, str):
            out.add(str(d / b))
    return out


def lex_unsafe(tree: Tree, cfg: Config) -> list[Finding]:
    """L1: `unsafe` in code only in class U perimeter files or exempt paths."""
    out = []
    for f in tree.files:
        if may_hold_unsafe(f, cfg):
            continue
        for t in tree.toks(f):
            if t.is_ident(UNSAFE):
                out.append(Finding("L1", f, t.line,
                                   f"`{UNSAFE}` outside the perimeter (a class U crate's src/**/*_unsafe.rs)"))
    return out


def lex_placement(tree: Tree, cfg: Config) -> list[Finding]:
    """L2: `*_unsafe.rs` files exist only under a class U or P crate's `src/`."""
    return [Finding("L2", f, 1, "a *_unsafe.rs file outside a class U or P crate's src/")
            for f in tree.files if f.endswith("_unsafe.rs") and not is_perimeter_file(f, cfg)]


def lex_lint_name(tree: Tree, cfg: Config, roots: set[str]) -> list[Finding]:
    """L3: the identifier `unsafe_code` only as a crate-root forbid/deny, or a line-1 allow."""
    out = []
    for f in tree.files:
        s = tree.struct(f)
        c = s.code
        ok_spans: set[int] = set()
        for o, e, inner in s.attrs:
            words = [t.text for t in c[o:e + 1]]
            if inner and words in (["#", "!", "[", "forbid", "(", "unsafe_code", ")", "]"],
                                   ["#", "!", "[", "deny", "(", "unsafe_code", ")", "]"]):
                if is_crate_root(f, roots):
                    ok_spans.add(o + 5)
            if inner and o == 0 and words == ["#", "!", "[", "allow", "(", "unsafe_code", ")", "]"]:
                cr = crate_of(f, cfg.class_u)
                if f.endswith("_unsafe.rs") and cr is not None and in_src(f, cr):
                    ok_spans.add(o + 5)
        for k, t in enumerate(c):
            if t.is_ident("unsafe_code") and k not in ok_spans:
                out.append(Finding("L3", f, t.line,
                                   "`unsafe_code` may appear only as a crate-root forbid/deny, or as "
                                   "`#![allow(unsafe_code)]` as the first tokens of a class U *_unsafe.rs"))
    return out


def parent_candidates(path: str) -> list[str]:
    p = PurePosixPath(path)
    d = p.parent
    return [str(d.parent / f"{d.name}.rs"), str(d / "mod.rs"), str(d / "lib.rs"), str(d / "main.rs")]


def is_mod_decl(c: list[rslex.Tok], k: int, stem: str | None = None) -> bool:
    return (c[k].is_ident("mod") and k + 2 < len(c) and c[k + 1].kind == "ident"
            and c[k + 2].is_punct(";") and (stem is None or c[k + 1].text == stem)
            and (k == 0 or not c[k - 1].is_punct("$")))


def perimeter_reuse_attrs(f: str, s: rslex.Structure, cfg: Config, crate: str) -> set[int]:
    """`#[path = "…/x_unsafe.rs"] mod x_unsafe;` naming a perimeter file of the SAME crate under
    its own stem: one target of a package reusing another's perimeter module (kf-gop's test
    application reuses `efi_unsafe.rs` and `port_unsafe.rs`). The module is still the perimeter
    file, and its diagnostics are attributed to it, so the location gate's verdict is unchanged.
    Any other `#[path]` fails L4. Returns the attribute start indices that qualify."""
    ok: set[int] = set()
    c = s.code
    p = PurePosixPath(f)
    base = p.parent if p.name in ("lib.rs", "main.rs", "mod.rs") else p.parent / p.stem
    for it in s.items:
        if it.kind != "mod" or it.body_open is not None or not it.name.endswith("_unsafe"):
            continue
        for a in it.attrs:
            words = c[a[0]:a[1] + 1]
            if [t.text for t in words[:4]] != ["#", "[", "path", "="] or len(words) != 6 \
                    or words[4].kind != "str":
                continue
            target = os.path.normpath(str(base / words[4].text.strip('"')))
            if (PurePosixPath(target).stem == it.name and is_perimeter_file(target, cfg)
                    and crate_of(target, cfg.class_u + cfg.class_p) == crate):
                ok.add(a[0])
    return ok


def lex_modules(tree: Tree, cfg: Config) -> list[Finding]:
    """L4: no #[path], include!, or #[macro_use] mod in class U/P src/; one `mod x_unsafe;` each."""
    out = []
    crates = cfg.class_u + cfg.class_p
    files = set(tree.files)
    for f in tree.files:
        cr = crate_of(f, crates)
        if cr is None or not in_src(f, cr):
            continue
        s = tree.struct(f)
        c = s.code
        reuse_ok = perimeter_reuse_attrs(f, s, cfg, cr)
        for o, e, _ in s.attrs:
            body = c[o:e + 1]
            for j, t in enumerate(body):
                if t.is_ident("path") and j + 1 < len(body) and body[j + 1].is_punct("=") and o not in reuse_ok:
                    out.append(Finding("L4", f, t.line, "#[path] in a class U/P crate's src/"))
        for k, t in enumerate(c):
            if t.is_ident("include") and k + 1 < len(c) and c[k + 1].is_punct("!"):
                out.append(Finding("L4", f, t.line, "include!() in a class U/P crate's src/"))
        for it in s.items:
            if it.kind == "mod" and any(s.attr_is(a, "macro_use") for a in it.attrs):
                out.append(Finding("L4", f, c[it.kw].line, "#[macro_use] on a module in a class U/P crate"))
        if f.endswith("_unsafe.rs") and is_perimeter_file(f, cfg):
            for k in range(len(c)):
                if is_mod_decl(c, k):
                    out.append(Finding("L4", f, c[k].line,
                                       "an out-of-line `mod` inside a *_unsafe.rs file"))
            stem = PurePosixPath(f).stem
            decls = []
            for p in parent_candidates(f):
                if p in files and p != f:
                    pc = tree.struct(p).code
                    decls += [(p, pc[k].line) for k in range(len(pc)) if is_mod_decl(pc, k, stem)]
            if len(decls) != 1:
                out.append(Finding("L4", f, 1,
                                   f"declared by {len(decls)} `mod {stem};` lines in its parent "
                                   f"(exactly one is required): {decls}"))
    return out


def lex_macros(tree: Tree, cfg: Config) -> list[Finding]:
    """L5: no exported macro in a class U crate carries `unsafe` in its body."""
    out = []
    for f in tree.files:
        if crate_of(f, cfg.class_u) is None:
            continue
        s = tree.struct(f)
        c = s.code
        for it in s.items:
            if it.kind != "macro_rules" or it.body_open is None:
                continue
            body = c[it.body_open:it.end + 1]
            has_unsafe = any(t.is_ident(UNSAFE) for t in body)
            exported = any(s.attr_is(a, "macro_export") or
                           s.attr_is(a, "macro_export", "(", "local_inner_macros", ")") for a in it.attrs)
            defines_export = any(t.is_ident("macro_export") for t in body)
            if has_unsafe and (exported or defines_export):
                out.append(Finding("L5", f, c[it.kw].line,
                                   f"exported macro `{it.name}!` has `{UNSAFE}` in its body; its expansions in "
                                   "other crates are invisible to the compiler location gate"))
    return out


def lex_extern_safe(tree: Tree, cfg: Config) -> list[Finding]:
    """L6: no `safe` qualifier on an item in an extern block."""
    del cfg
    out = []
    for f in tree.files:
        s = tree.struct(f)
        c = s.code
        for it in s.items:
            if it.kind != "extern_block":
                continue
            for k in range(it.body_open + 1, it.end):
                if c[k].is_ident("safe") and k + 1 < len(c) and (c[k + 1].is_ident("fn") or c[k + 1].is_ident("static")):
                    out.append(Finding("L6", f, c[k].line, "a `safe` item inside an extern block"))
    return out


RUST_FENCE_TAGS = {"rust", "no_run", "should_panic", "compile_fail", "ignore", "test_harness",
                   "standalone_crate"}


def fence_is_rust(info: str) -> bool:
    tags = [t for t in re.split(r"[,\s]+", info.strip()) if t]
    if not tags:
        return True
    rustish = lambda t: t in RUST_FENCE_TAGS or re.fullmatch(r"edition\d{4}", t) or t.startswith("ignore-")
    return any(rustish(t) for t in tags) or all(rustish(t) for t in tags)


def doc_lines(toks: list[rslex.Tok]) -> list[tuple[int, list[tuple[int, str]]]]:
    """Group doc comments into runs; yields (first line, [(line, text)])."""
    runs: list[tuple[int, list[tuple[int, str]]]] = []
    cur: list[tuple[int, str]] = []
    last = None
    for t in toks:
        if t.kind == "doc":
            lines = []
            if t.text.startswith(("///", "//!")):
                lines = [(t.line, t.text[3:])]
            else:
                body = t.text[3:-2]
                for off, ln in enumerate(body.split("\n")):
                    lines.append((t.line + off, re.sub(r"^\s*\* ?", "", ln) if off else ln))
            if last is not None and t.kind == "doc" and t.line == last + 1:
                cur.extend(lines)
            else:
                if cur:
                    runs.append((cur[0][0], cur))
                cur = list(lines)
            last = t.end_line
        elif t.kind == "comment":
            continue
        else:
            if cur:
                runs.append((cur[0][0], cur))
            cur, last = [], None
    if cur:
        runs.append((cur[0][0], cur))
    return runs


def fences(lines: list[tuple[int, str]]) -> list[tuple[int, str, list[tuple[int, str]]]]:
    out = []
    i = 0
    while i < len(lines):
        ln, text = lines[i]
        m = re.match(r"^\s*(```+|~~~+)(.*)$", text)
        if not m:
            i += 1
            continue
        mark, info = m.group(1), m.group(2)
        body = []
        i += 1
        while i < len(lines) and not re.match(rf"^\s*{re.escape(mark[0])}{{{len(mark)},}}\s*$", lines[i][1]):
            body.append(lines[i])
            i += 1
        out.append((ln, info, body))
        i += 1
    return out


def fence_has_unsafe(body: list[tuple[int, str]]) -> int | None:
    src = "\n".join(re.sub(r"^\s*#\s", "", t) for _, t in body)
    try:
        for t in rslex.tokenize(src):
            if t.is_ident(UNSAFE):
                return body[t.line - 1][0] if t.line - 1 < len(body) else body[0][0]
        return None
    except rslex.LexError:
        for ln, t in body:
            if re.search(rf"\b{UNSAFE}\b", t):
                return ln
        return None


def lex_doctests(tree: Tree, cfg: Config) -> list[Finding]:
    """L10: no `unsafe` in a doc-comment Rust fence outside the perimeter."""
    out = []
    for f in tree.files:
        if may_hold_unsafe(f, cfg):
            continue
        toks = tree.toks(f)
        runs = doc_lines(toks)
        # `#[doc = include_str!("x.md")]`: that file's fences are doctests too.
        s = tree.struct(f)
        c = s.code
        for o, e, _ in s.attrs:
            words = [t for t in c[o:e + 1]]
            for j, t in enumerate(words):
                if t.is_ident("include_str") and j + 3 < len(words) and words[j + 2].is_punct("(") \
                        and words[j + 3].kind == "str":
                    lit = words[j + 3].text.strip('"')
                    md = (tree.root / f).parent / lit
                    if md.exists():
                        lines = [(i + 1, x) for i, x in enumerate(md.read_text().splitlines())]
                        runs.append((t.line, lines))
        for _, lines in runs:
            for ln, info, body in fences(lines):
                if fence_is_rust(info):
                    hit = fence_has_unsafe(body)
                    if hit is not None:
                        out.append(Finding("L10", f, hit, f"`{UNSAFE}` in a doctest fence (from line {ln})"))
    return out


def symlink_findings(ls_stage: str) -> list[Finding]:
    """L0 (file sets): no tracked symlink. `ls_stage` is `git ls-files -s -z` output."""
    out = []
    for entry in ls_stage.split("\0"):
        if not entry:
            continue
        meta, _, path = entry.partition("\t")
        if meta.split()[0] == "120000":
            out.append(Finding("L0", path, 0, "a tracked symlink (the lexer and the compiler would disagree "
                                              "on which file this is)"))
    return out


def run_lex(root: Path, cfg: Config, files: list[str] | None = None) -> tuple[list[Finding], Tree]:
    real_tree = files is None
    files = git_ls(root, "*.rs") if files is None else files
    tree = Tree(root, files)
    manifests = [m for m in (git_ls(root, "Cargo.toml", "*/Cargo.toml") if files is None else
                             [str(p.relative_to(root)) for p in root.rglob("Cargo.toml")])
                 if not m.startswith(EXCLUDED_TOPS)]
    roots = manifest_roots(root, manifests)
    findings: list[Finding] = []
    if real_tree:
        findings += symlink_findings(subprocess.run(["git", "ls-files", "-s", "-z"], cwd=root, capture_output=True,
                                                    check=True).stdout.decode())
    for f in files:
        try:
            tree.struct(f)
        except rslex.LexError as e:
            findings.append(Finding("L0", f, 0, f"cannot tokenize: {e}"))
    if findings:
        return findings, tree
    findings += lex_unsafe(tree, cfg)
    findings += lex_placement(tree, cfg)
    findings += lex_lint_name(tree, cfg, roots)
    findings += lex_modules(tree, cfg)
    findings += lex_macros(tree, cfg)
    findings += lex_extern_safe(tree, cfg)
    findings += lex_doctests(tree, cfg)
    return sorted(findings), tree


# ---------------------------------------------------------------------------------------
# G6: the size ratchet (rule c), L8 and L9
# ---------------------------------------------------------------------------------------

SIZE_COLUMNS = ("code", *rslex.KINDS, "macro_unsafe", "exports")
SIZES_TSV = "scripts/ci/perimeter/sizes.tsv"
REASON_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}): (.*?)\s*— (.+)$")


def code_lines(s: rslex.Structure) -> int:
    """Lines holding at least one code token, outside items under exactly `#[cfg(test)]`."""
    tests = s.test_ranges()
    lines: set[int] = set()
    for k, t in enumerate(s.code):
        if not rslex.in_ranges(k, tests):
            lines.update(range(t.line, t.end_line + 1))
    return len(lines)


def strip_c(src: str) -> str:
    """C source with comments removed (strings and char literals kept, newlines kept)."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            if j < 0:
                raise ValueError("unterminated C comment")
            out.append("\n" * src.count("\n", i, j + 2))
            i = j + 2
        elif src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif c in "\"'":
            j = i + 1
            while j < n and src[j] != c:
                j += 2 if src[j] == "\\" else 1
            out.append(src[i:j + 1])
            i = j + 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def c_code_lines(text: str) -> int:
    return sum(1 for ln in strip_c(text).splitlines() if ln.strip())


def crate_files(tree: Tree, crate: str) -> list[str]:
    return [f for f in tree.files if crate_of(f, [crate]) == crate]


def measure_file(tree: Tree, f: str, crate_structs: list[rslex.Structure]) -> dict[str, int]:
    s = tree.struct(f)
    row = {c: 0 for c in SIZE_COLUMNS}
    row["code"] = code_lines(s)
    for site in rslex.unsafe_sites(s):
        if site.kind in rslex.KINDS:
            row[site.kind] += 1
        elif site.kind == "unknown":
            raise rslex.LexError(f"{f}:{site.line}: an `{UNSAFE}` this tokenizer cannot classify")
    for it in rslex.macro_bodies(s):
        inner = [x for x in rslex.unsafe_sites(s) if it.body_open < x.k < it.end
                 and x.kind in rslex.KINDS and x.kind != "asm"]
        if inner:
            calls = sum(rslex.macro_invocations(cs, it.name) for cs in crate_structs)
            row["macro_unsafe"] += len(inner) * calls
    return row


def size_rows(tree: Tree, cfg: Config, c_files: list[str], exports: dict[str, int] | None = None,
              validates: list[str] = ()) -> dict[str, dict[str, int]]:
    rows: dict[str, dict[str, int]] = {}
    structs: dict[str, list[rslex.Structure]] = {}
    for f in sorted(set(f for f in tree.files if is_perimeter_file(f, cfg)) | set(validates)):
        cr = crate_of(f, cfg.class_u + cfg.class_p) or ""
        if cr not in structs:
            structs[cr] = [tree.struct(x) for x in crate_files(tree, cr) if in_src(x, cr)]
        rows[f] = measure_file(tree, f, structs[cr])
        rows[f]["exports"] = (exports or {}).get(f, 0)
    for f in c_files:
        row = {c: 0 for c in SIZE_COLUMNS}
        row["code"] = c_code_lines((tree.root / f).read_text())
        rows[f] = row
    return rows


def read_tsv(text: str) -> dict[str, dict]:
    rows: dict[str, dict] = {}
    lines = [ln for ln in text.splitlines() if ln.strip() and not ln.startswith("#")]
    if not lines:
        return rows
    header = lines[0].split("\t")
    if header != ["path", *SIZE_COLUMNS, "reason"]:
        raise ValueError(f"{SIZES_TSV}: header {header} != {['path', *SIZE_COLUMNS, 'reason']}")
    for ln in lines[1:]:
        cells = ln.split("\t")
        if len(cells) != len(header):
            raise ValueError(f"{SIZES_TSV}: malformed row: {ln!r}")
        row = {c: int(v) for c, v in zip(SIZE_COLUMNS, cells[1:-1])}
        row["reason"] = cells[-1]
        rows[cells[0]] = row
    return rows


def write_tsv(rows: dict[str, dict]) -> str:
    head = ("# The perimeter size ratchet (docs/design/V3_SEC_PERIMETER.md §2; OWNER_RULINGS §R rule c).\n"
            "# Every count is EXACT. A decrease needs only the new number (`perimeter.py sizes --update`).\n"
            "# A rise, or a new row, needs a new reason: `YYYY-MM-DD: <column>+<delta> ... — <why>`.\n")
    out = [head + "\t".join(["path", *SIZE_COLUMNS, "reason"])]
    for f in sorted(rows):
        r = rows[f]
        out.append("\t".join([f, *(str(r[c]) for c in SIZE_COLUMNS), r.get("reason", "")]))
    return "\n".join(out) + "\n"


def size_findings(actual: dict[str, dict], stored: dict[str, dict], base: dict[str, dict] | None,
                  judge: bool = True) -> list[Finding]:
    """SF2/SF3 (exact, every row present, no stale row) and SF1 (a rise carries its reason).

    `base` is the TSV at the comparison base, or None when the base predates the ratchet: then
    every row must carry a dated `baseline` reason."""
    out: list[Finding] = []
    for f in sorted(actual.keys() - stored.keys()):
        out.append(Finding("SF3", SIZES_TSV, 0, f"no row for perimeter file {f} ({fmt_row(actual[f])})"))
    for f in sorted(stored.keys() - actual.keys()):
        out.append(Finding("SF3", SIZES_TSV, 0, f"stale row {f}: no such perimeter file"))
    for f in sorted(actual.keys() & stored.keys()):
        diff = [f"{c} {stored[f][c]}->{actual[f][c]}" for c in SIZE_COLUMNS if stored[f][c] != actual[f][c]]
        if diff:
            out.append(Finding("SF2", SIZES_TSV, 0, f"{f}: counts changed ({', '.join(diff)}); update the row"))
    for f in sorted(stored) if judge else []:
        reason = stored[f]["reason"]
        if base is None:
            m = REASON_RE.match(reason)
            if not m or "baseline" not in m.group(2):
                out.append(Finding("SF1", SIZES_TSV, 0, f"{f}: the first landing needs "
                                                       f"`YYYY-MM-DD: baseline — <why>`, got {reason!r}"))
            continue
        old = base.get(f)
        risen = {c: stored[f][c] - (old[c] if old else 0) for c in SIZE_COLUMNS}
        risen = {c: d for c, d in risen.items() if d > 0}
        if not risen and old is not None:
            continue
        m = REASON_RE.match(reason)
        tokens = set(m.group(2).split()) if m else set()
        want = {f"{c}+{d}" for c, d in risen.items()}
        if old is not None and reason == old["reason"]:
            out.append(Finding("SF1", SIZES_TSV, 0, f"{f}: {sorted(want)} rose but the reason is unchanged"))
        elif not m or not want <= tokens:
            out.append(Finding("SF1", SIZES_TSV, 0, f"{f}: a {'new row' if old is None else 'rise'} needs "
                                                   f"`YYYY-MM-DD: {' '.join(sorted(want))} — <why>`, got {reason!r}"))
    return out


def fmt_row(r: dict) -> str:
    return " ".join(f"{c}={r[c]}" for c in SIZE_COLUMNS if r[c])


# The phrasing of a precondition handed to callers. "the caller's buffer is borrowed in this
# expression" or "the CALLING thread" describe the call, not an obligation, and do not match.
CALLER_OBLIGATION = re.compile(
    r"\b(?:every|each|all) callers?\b"
    r"|\bcallers? (?:size|sizes|bound|bounds|must|ensure|ensures|guarantee|guarantees|pass|passes|promise)\b"
    r"|\bcaller'?s obligation\b|\bthe caller passed\b|\bthe caller \(",
    re.IGNORECASE)


def l8_sites(tree: Tree, cfg: Config) -> list[tuple[str, str, int]]:
    """L8: `// SAFETY:` comments that state a CALLER obligation inside a safe fn of a perimeter
    file. Each is a precondition left to safe callers, which rule (a) forbids."""
    out = []
    for f in tree.files:
        if not is_perimeter_file(f, cfg):
            continue
        s = tree.struct(f)
        code_ix = {id(t): k for k, t in enumerate(s.code)}
        fns = [it for it in s.items if it.kind == "fn" and it.body_open is not None]
        block: list[rslex.Tok] = []
        nxt = 0

        def flush():
            if not block or not re.match(r"^//\s*SAFETY:", block[0].text):
                return
            text = " ".join(t.text[2:].strip() for t in block)
            if not CALLER_OBLIGATION.search(text):
                return
            inner = [it for it in fns if it.body_open < nxt <= it.end]
            if not inner:
                return
            fn = max(inner, key=lambda it: it.body_open)
            if UNSAFE in fn.qualifiers:
                return
            owner = f"{fn.parent.name}::" if fn.parent is not None and fn.parent.kind in ("impl", "trait") else ""
            out.append((f, owner + fn.name, block[0].line))

        for t in s.all:
            if t.kind == "comment" and t.text.startswith("//"):
                if block and t.line != block[-1].line + 1:
                    flush()
                    block = []
                block.append(t)
                continue
            if t.is_code:
                if block:
                    nxt = code_ix[id(t)]
                    flush()
                    block = []
        flush()
    return sorted(out)


def mint_aliases(s: rslex.Structure, names: set[str]) -> set[str]:
    """Names a file introduces for a mint name: `use … name as Alias` and `type Alias = …name…`."""
    c = s.code
    found = set()
    for k in range(len(c) - 2):
        if c[k].kind == "ident" and c[k].text in names and c[k + 1].is_ident("as") and c[k + 2].kind == "ident":
            found.add(c[k + 2].text)
    for it in s.items:
        if it.kind == "type" and any(x.kind == "ident" and x.text in names for x in c[it.kw:it.end + 1]):
            found.add(it.name)
    return found


def mint_sites(tree: Tree, cfg: Config, kf3_crates: list[str]) -> dict[str, list[int]]:
    """L9: mint-name sites in src/ of kf3-graph crates, outside cfg(test), perimeter files and kf-abi."""
    names = set(cfg.raw.get("mint", {}).get("names", []))
    out: dict[str, list[int]] = {}
    for cr in kf3_crates:
        if cr.endswith("/kf-abi"):
            continue
        files = [f for f in crate_files(tree, cr) if in_src(f, cr) and not is_perimeter_file(f, cfg)]
        crate_names = set(names)
        while True:
            more = set().union(*(mint_aliases(tree.struct(f), crate_names) for f in files)) - crate_names
            if not more:
                break
            crate_names |= more
        for f in files:
            s = tree.struct(f)
            tests = s.test_ranges()
            hits = [t.line for k, t in enumerate(s.code)
                    if t.kind == "ident" and not t.raw and t.text in crate_names and not rslex.in_ranges(k, tests)]
            if hits:
                out[f] = hits
    return out


def mint_findings(sites: dict[str, list[int]], baseline: dict[str, int]) -> list[Finding]:
    out = []
    for f in sorted(set(sites) | set(baseline)):
        got, want = len(sites.get(f, [])), baseline.get(f, 0)
        if got > want:
            out.append(Finding("L9", f, sites[f][0], f"{got} mint-name sites > baseline {want}: build the value in a "
                                                     "perimeter file, or raise [mint.baseline] with a reason"))
        elif got < want:
            out.append(Finding("L9", f, 0, f"{got} mint-name sites < baseline {want}: lower [mint.baseline] "
                                           "(the count only goes down, and exactly)"))
    return out


def l8_findings(sites: list[tuple[str, str, int]], baseline: list[str]) -> list[Finding]:
    got = {f"{f}::{fn}": line for f, fn, line in sites}
    out = []
    for key in sorted(set(got) - set(baseline)):
        f = key.split("::", 1)[0]
        out.append(Finding("L8", f, got[key], f"a caller-obligation SAFETY comment in safe fn `{key.split('::', 1)[1]}`:"
                                              " check the precondition inside, or make the fn `unsafe fn`"))
    for key in sorted(set(baseline) - set(got)):
        out.append(Finding("L8", key.split("::", 1)[0], 0, f"[l8].sites lists `{key}`, which is gone: remove it"))
    return out


# ---------------------------------------------------------------------------------------
# G4: manifests
# ---------------------------------------------------------------------------------------

FORBIDDEN_CONFIG_KEYS = ("rustflags", "rustdocflags", "rustc", "rustc-wrapper", "rustc-workspace-wrapper")
FORBIDDEN_ENV = re.compile(r"^(RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_TARGET_\w+_RUSTFLAGS|RUSTDOCFLAGS|"
                           r"CARGO_ENCODED_RUSTDOCFLAGS|RUSTC|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|"
                           r"RUSTC_BOOTSTRAP|CARGO_BUILD_RUSTFLAGS|CARGO_BUILD_RUSTC\w*)$")


def cargo_config_violations(path: str, data: dict) -> list[Finding]:
    out = []

    def walk(d: dict, trail: str) -> None:
        for k, v in d.items():
            key = f"{trail}.{k}" if trail else k
            if k in FORBIDDEN_CONFIG_KEYS and (trail in ("build", "host") or trail.startswith("target")):
                out.append(Finding("M4", path, 1, f"cargo config sets `{key}`"))
            if trail == "env" and FORBIDDEN_ENV.match(k):
                out.append(Finding("M4", path, 1, f"cargo config [env] sets `{k}`"))
            if isinstance(v, dict):
                walk(v, key)
    walk(data, "")
    return out


def workflow_env_violations(path: str, text: str) -> list[Finding]:
    """M4: a workflow `env:` mapping (any level) setting a compiler-flag variable."""
    out = []
    in_env, env_indent = False, -1
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.split("#", 1)[0].rstrip()
        if not stripped.strip():
            continue
        indent = len(stripped) - len(stripped.lstrip())
        m = re.match(r"^(\s*)(?:-\s+)?env:\s*$", stripped)
        if m:
            in_env, env_indent = True, indent
            continue
        if in_env and indent <= env_indent:
            in_env = False
        if in_env:
            km = re.match(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*:", stripped)
            if km and FORBIDDEN_ENV.match(km.group(1)):
                out.append(Finding("M4", path, n, f"workflow env sets `{km.group(1)}`"))
        inline = re.search(r"\benv:\s*\{([^}]*)\}", stripped)
        if inline:
            for k in re.findall(r"([A-Za-z_][A-Za-z0-9_]*)\s*:", inline.group(1)):
                if FORBIDDEN_ENV.match(k):
                    out.append(Finding("M4", path, n, f"workflow env sets `{k}`"))
    return out


def run_manifest(root: Path, cfg: Config, members: list[str] | None = None,
                 manifests: list[str] | None = None, configs: list[str] | None = None,
                 workflows: list[str] | None = None) -> list[Finding]:
    """M1-M5 plus manifest coverage and the pinned build scripts.

    `members` are package directories relative to root (from `cargo metadata --no-deps`).
    """
    out: list[Finding] = []
    if members is None:
        members = workspace_members(root)
    if manifests is None:
        manifests = git_ls(root, "Cargo.toml", "*/Cargo.toml")
    if configs is None:
        configs = [f for f in git_ls(root, ".cargo/*", "*/.cargo/*")
                   if PurePosixPath(f).name in ("config", "config.toml")]
    if workflows is None:
        workflows = git_ls(root, ".github/workflows/*")
    standalone = [s["path"] for s in cfg.standalone]
    class_u = set(cfg.class_u)
    exempt = set(cfg.exempt)
    # M1
    for m in members:
        if m in class_u or m in exempt:
            continue
        data = tomllib.loads((root / m / "Cargo.toml").read_text())
        if data.get("lints") != {"workspace": True}:
            out.append(Finding("M1", f"{m}/Cargo.toml", 1,
                               f"[lints] must be exactly `workspace = true` (found {data.get('lints')!r})"))
    # M2
    ws = tomllib.loads((root / "Cargo.toml").read_text()).get("workspace", {})
    if ((ws.get("lints") or {}).get("rust") or {}).get("unsafe_code") != "forbid":
        out.append(Finding("M2", "Cargo.toml", 1, "[workspace.lints.rust].unsafe_code must be \"forbid\""))
    # M3
    pinned = cfg.raw.get("manifest_lints", {})
    for c in sorted(class_u | set(standalone)):
        data = tomllib.loads((root / c / "Cargo.toml").read_text())
        want = pinned.get(c)
        if want is None:
            out.append(Finding("M3", f"{c}/Cargo.toml", 1, "no pinned [lints] table in perimeter.toml"))
        elif data.get("lints") != want:
            out.append(Finding("M3", f"{c}/Cargo.toml", 1,
                               f"[lints] differs from the pinned table: {data.get('lints')!r} != {want!r}"))
    # M4
    for f in configs:
        out += cargo_config_violations(f, tomllib.loads((root / f).read_text()))
    for f in workflows:
        out += workflow_env_violations(f, (root / f).read_text())
    # The toolchain pin: perimeter.toml, rust-toolchain.toml and every workflow `toolchain:` agree,
    # because the location wrapper execs only the pinned rustc and rustdoc's JSON format is pinned.
    pin = cfg.raw.get("toolchain")
    rt = root / "rust-toolchain.toml"
    if rt.exists() and tomllib.loads(rt.read_text()).get("toolchain", {}).get("channel") != pin:
        out.append(Finding("M0", "rust-toolchain.toml", 1, f"channel differs from perimeter.toml toolchain {pin!r}"))
    for f in workflows:
        for n, line in enumerate((root / f).read_text().splitlines(), 1):
            m = re.match(r"^\s*toolchain:\s*([^\s#]+)", line)
            if m and m.group(1).strip("'\"") != pin:
                out.append(Finding("M0", f, n, f"workflow toolchain {m.group(1)} differs from the pin {pin!r}"))
    # M5 and coverage
    known = set(members) | set(standalone)
    for m in manifests:
        d = str(PurePosixPath(m).parent)
        d = "" if d == "." else d
        data = tomllib.loads((root / m).read_text())
        if "package" in data and d not in known:
            out.append(Finding("M1", m, 1, "a tracked package that is neither a workspace member nor "
                                           "listed under `standalone` in perimeter.toml"))
        lib = data.get("lib") or {}
        if lib.get("proc-macro") or lib.get("proc_macro"):
            out.append(Finding("M5", m, 1, "a repository package may not be a proc-macro"))
    # pinned build scripts: the set of build-script sources equals the table, with equal hashes
    pins = cfg.raw.get("build_scripts", {})
    found: set[str] = set()
    for m in manifests:
        d = PurePosixPath(m).parent
        data = tomllib.loads((root / m).read_text())
        b = (data.get("package") or {}).get("build")
        cand = [str(d / b)] if isinstance(b, str) else ([str(d / "build.rs")] if b is not False else [])
        for x in cand:
            if (root / x).exists():
                found.add(x)
        bdir = root / d / "build"
        if bdir.is_dir():
            found |= {str(p.relative_to(root)) for p in bdir.rglob("*.rs")}
    for x in sorted(found | set(pins)):
        if x not in pins:
            out.append(Finding("M4", x, 1, "an unpinned build script (add its sha256 to [build_scripts])"))
        elif x not in found:
            out.append(Finding("M4", x, 1, "a pinned build script that no longer exists"))
        elif hashlib.sha256((root / x).read_bytes()).hexdigest() != pins[x]:
            out.append(Finding("M4", x, 1, "build script changed: re-review it and update its sha256"))
    return sorted(out)


def cargo_metadata(root: Path, manifest: str | None = None, no_deps: bool = False) -> dict:
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked"]
    if no_deps:
        cmd.append("--no-deps")
    if manifest:
        cmd += ["--manifest-path", str(root / manifest)]
    r = subprocess.run(cmd, cwd=root, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"cargo metadata failed ({' '.join(cmd)}):\n{r.stderr}")
    return json.loads(r.stdout)


def rel_dir(root: Path, manifest_path: str) -> str:
    return str(Path(os.path.realpath(manifest_path)).parent.relative_to(os.path.realpath(root)))


def workspace_members(root: Path) -> list[str]:
    m = cargo_metadata(root, no_deps=True)
    ids = set(m["workspace_members"])
    return sorted(rel_dir(root, p["manifest_path"]) for p in m["packages"] if p["id"] in ids)


# ---------------------------------------------------------------------------------------
# G4: the resolved graph
# ---------------------------------------------------------------------------------------


def external_edges(meta: dict) -> set[str]:
    pk = {p["id"]: p for p in meta["packages"]}
    members = set(meta["workspace_members"])
    edges = set()
    for n in meta["resolve"]["nodes"]:
        if n["id"] not in members:
            continue
        for d in n["deps"]:
            if pk[d["pkg"]]["source"] is None:
                continue
            for k in d["dep_kinds"]:
                kind = k["kind"] or "normal"
                tgt = f" [{k['target']}]" if k["target"] else ""
                edges.add(f"{pk[n['id']]['name']} -> {pk[d['pkg']]['name']} ({kind}){tgt}")
    return edges


def reachable(meta: dict, roots: set[str], kinds: tuple | None = None) -> set[str]:
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen: set[str] = set()
    stack = list(roots)
    while stack:
        x = stack.pop()
        if x in seen:
            continue
        seen.add(x)
        for d in nodes[x]["deps"]:
            if kinds is None or any((k["kind"] in kinds) for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    return seen


def metadata_violations(root: Path, cfg: Config, meta: dict, standalone_metas: dict[str, dict]) -> list[Finding]:
    """M7 (exact external set, edges, reachable proc-macros, no git), M8 (path packages), M9."""
    out: list[Finding] = []
    ext = cfg.raw.get("external", {})
    pk = {p["id"]: p for p in meta["packages"]}
    allmeta = [meta, *standalone_metas.values()]
    externals = {p["name"] for m in allmeta for p in m["packages"] if p["source"] is not None}
    want = set(ext.get("packages", []))
    for n in sorted(externals - want):
        out.append(Finding("M7", "Cargo.lock", 1, f"external package `{n}` is not in [external].packages"))
    for n in sorted(want - externals):
        out.append(Finding("M7", "Cargo.lock", 1, f"[external].packages lists `{n}`, which the lock no longer has"))
    edges = set().union(*(external_edges(m) for m in allmeta))
    wedges = set(ext.get("edges", []))
    for e in sorted(edges - wedges):
        out.append(Finding("M7", "Cargo.lock", 1, f"new member->external edge `{e}`"))
    for e in sorted(wedges - edges):
        out.append(Finding("M7", "Cargo.lock", 1, f"stale [external].edges row `{e}`"))
    pms = set()
    for m in allmeta:
        mpk = {p["id"]: p for p in m["packages"]}
        for x in reachable(m, set(m["workspace_members"])):
            if any("proc-macro" in t["kind"] for t in mpk[x]["targets"]):
                pms.add(mpk[x]["name"])
    wpm = set(ext.get("proc_macros_reachable", []))
    if pms != wpm:
        out.append(Finding("M7", "Cargo.lock", 1,
                           f"reachable proc-macros {sorted(pms)} != [external].proc_macros_reachable {sorted(wpm)}"))
    for m in allmeta:
        for p in m["packages"]:
            if p["source"] and p["source"].startswith("git+"):
                out.append(Finding("M7", "Cargo.lock", 1, f"git source `{p['name']}` ({p['source']})"))
    # M8: every path package is a root member or a standalone package (or a member of one)
    standalone = {s["path"] for s in cfg.standalone}
    root_members = {rel_dir(root, pk[i]["manifest_path"]) for i in meta["workspace_members"]}
    for m in allmeta:
        for p in m["packages"]:
            if p["source"] is not None:
                continue
            d = rel_dir(root, p["manifest_path"])
            if d not in root_members and d not in standalone:
                out.append(Finding("M8", f"{d}/Cargo.toml", 1,
                                   "a path package that is neither a root member nor a standalone package"))
    # M9: every target's source lies under its own package directory
    for m in allmeta:
        for p in m["packages"]:
            if p["source"] is not None:
                continue
            pdir = os.path.realpath(Path(p["manifest_path"]).parent)
            for t in p["targets"]:
                src = os.path.realpath(t["src_path"])
                if not (src + "/").startswith(pdir + "/"):
                    out.append(Finding("M9", rel_dir(root, p["manifest_path"]) + "/Cargo.toml", 1,
                                       f"target `{t['name']}` borrows a source outside its package: {t['src_path']}"))
    return sorted(set(out))


def run_metadata(root: Path, cfg: Config) -> list[Finding]:
    meta = cargo_metadata(root)
    sm = {s["path"]: cargo_metadata(root, f"{s['path']}/Cargo.toml") for s in cfg.standalone}
    return metadata_violations(root, cfg, meta, sm)


# ---------------------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------------------


def kf3_member_dirs(root: Path, cfg: Config) -> list[str]:
    import dependencies  # noqa: PLC0415 (scripts/ci sibling)
    meta = cargo_metadata(root)
    pk = {p["id"]: p for p in meta["packages"]}
    closure = dependencies.kf3_closure(meta, cfg.raw["kf3"]["root"])
    return sorted(rel_dir(root, pk[i]["manifest_path"]) for i in closure if pk[i]["source"] is None)


def tsv_at(root: Path, ref: str) -> dict[str, dict] | None:
    r = subprocess.run(["git", "show", f"{ref}:{SIZES_TSV}"], cwd=root, capture_output=True, text=True)
    if r.returncode != 0:
        if "does not exist" in r.stderr or "exists on disk, but not in" in r.stderr:
            return None
        raise SystemExit(f"git show {ref}:{SIZES_TSV} failed: {r.stderr.strip()}")
    return read_tsv(r.stdout)


def run_sizes(root: Path, cfg: Config, base: str | None, update: bool,
              kf3_crates: list[str] | None = None,
              exports: dict[str, int] | None = None) -> tuple[list[Finding], dict]:
    tree = Tree(root, git_ls(root, "*.rs"))
    c_files = cfg.raw.get("c", {}).get("files", [])
    actual = size_rows(tree, cfg, c_files, exports)
    path = root / SIZES_TSV
    stored = read_tsv(path.read_text()) if path.exists() else {}
    base_rows = tsv_at(root, base) if base else None
    findings: list[Finding] = []
    if update:
        new = {}
        for f, r in actual.items():
            old = stored.get(f)
            if old is not None and all(r[c] <= old[c] for c in SIZE_COLUMNS):
                new[f] = {**r, "reason": old["reason"]}
            elif old is not None:
                new[f] = old
                print(f"RISE {f}: {fmt_row(r)} — edit the row by hand with a dated reason")
            else:
                print(f"NEW  {f}\t" + "\t".join(str(r[c]) for c in SIZE_COLUMNS) + "\tYYYY-MM-DD: … — <why>")
        path.write_text(write_tsv(new))
        stored = new
    findings += size_findings(actual, stored, base_rows, judge=base is not None)
    findings += l8_findings(l8_sites(tree, cfg), cfg.raw.get("l8", {}).get("sites", []))
    if kf3_crates is None:
        kf3_crates = kf3_member_dirs(root, cfg)
    findings += mint_findings(mint_sites(tree, cfg, kf3_crates), cfg.raw.get("mint", {}).get("baseline", {}))
    totals: dict[str, dict[str, int]] = {}
    for f, r in actual.items():
        cr = crate_of(f, cfg.class_u + cfg.class_p) or "qemu/hw/misc/kf3"
        t = totals.setdefault(cr, {c: 0 for c in SIZE_COLUMNS})
        for c in SIZE_COLUMNS:
            t[c] += r[c]
    return sorted(findings), totals


def report(name: str, findings: list[Finding], extra: str = "") -> int:
    for f in findings:
        print(f)
    print(f"PERIMETER_{name.upper()} findings={len(findings)}{extra}")
    return 1 if findings else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=("lex", "manifest", "metadata", "sizes"))
    ap.add_argument("--base", help="sizes: the git ref whose sizes.tsv a rise is judged against")
    ap.add_argument("--update", action="store_true", help="sizes: write decreases and drop stale rows")
    args = ap.parse_args(argv)
    cfg = Config.load(ROOT)
    if args.cmd == "lex":
        findings, tree = run_lex(ROOT, cfg)
        return report("lex", findings, f" files={len(tree.files)}")
    if args.cmd == "manifest":
        return report("manifest", run_manifest(ROOT, cfg))
    if args.cmd == "metadata":
        return report("metadata", run_metadata(ROOT, cfg))
    if args.cmd == "sizes":
        findings, totals = run_sizes(ROOT, cfg, args.base, args.update)
        for cr, t in sorted(totals.items()):
            print(f"  {cr}: {fmt_row(t)}")
        return report("sizes", findings, f" base={args.base or '-'}")
    return 2


if __name__ == "__main__":
    sys.exit(main())
