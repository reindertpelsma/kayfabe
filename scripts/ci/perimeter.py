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


# The only variables a location pass may set (the aarch64 pass's image stub, as ci.yml does).
PASS_ENV_ALLOWED = frozenset({"KAYFABE_ISOLATE_IMAGE_STUB"})


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

    def location_passes(self) -> list[dict]:
        """G1's cargo invocations: the `[[location.pass]]` rows, then each standalone package's runs."""
        out = []
        for p in self.raw.get("location", {}).get("pass", []):
            env = dict(p.get("env", {}))
            bad = sorted(k for k in env if k not in PASS_ENV_ALLOWED)
            if bad:
                raise SystemExit(f"[[location.pass]] {p['name']}: env {bad} is not in {sorted(PASS_ENV_ALLOWED)}")
            out.append({"name": p["name"], "manifest": "Cargo.toml", "args": p["args"].split(), "env": env})
        for sa in self.standalone:
            for i, r in enumerate(sa.get("runs", [])):
                out.append({"name": f"{sa['path']}#{i}", "manifest": f"{sa['path']}/Cargo.toml",
                            "args": r.split(), "env": {}})
        return out

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
        self._text: dict[str, str] = {}
        self._toks: dict[str, list[rslex.Tok]] = {}
        self._struct: dict[str, rslex.Structure] = {}

    def text(self, f: str) -> str:
        if f not in self._text:
            try:
                self._text[f] = (self.root / f).read_text(encoding="utf-8")
            except UnicodeDecodeError as e:
                raise rslex.LexError(f"{f}: not UTF-8 ({e})") from e
        return self._text[f]

    def toks(self, f: str) -> list[rslex.Tok]:
        if f not in self._toks:
            self._toks[f] = rslex.tokenize(self.text(f), f)
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
    """L1: `unsafe`, and the names of the asm macros, in code only in class U perimeter files or
    exempt paths.

    `global_asm!`/`naked_asm!`/`asm!` need no keyword: `global_asm!` links arbitrary machine code
    from safe Rust, and the compiler's `unsafe_code` lint is silent for it when it arrives from
    another crate's exported macro (measured 2026-10-04, review finding: a `#[macro_export]`
    wrapper in kf-linux-raw expanded in kf-host emitted a symbol with no diagnostic). So the NAME
    is the keyword's equal here, raw spelling included, wherever it appears (a `use … as`
    alias still names it)."""
    out = []
    for f in tree.files:
        if may_hold_unsafe(f, cfg):
            continue
        for t in tree.toks(f):
            if t.is_ident(UNSAFE):
                out.append(Finding("L1", f, t.line,
                                   f"`{UNSAFE}` outside the perimeter (a class U crate's src/**/*_unsafe.rs)"))
            elif any(t.is_ident(m) for m in rslex.ASM_MACROS):
                out.append(Finding("L1", f, t.line,
                                   f"`{t.text}` outside the perimeter: the asm macros need no keyword, so their "
                                   "name is one"))
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
        # ★ The NAME, anywhere in code, not `include` followed by `!`: `use core::include as
        # splice; splice!("x_unsafe.rs")` and `r#include!(…)` both compile (measured 2026-10-04)
        # and splice a perimeter file into a safe module, where the compiler attributes the
        # spliced code to the perimeter file (review finding, 2026-10-04).
        for k, t in enumerate(c):
            if t.is_ident("include"):
                out.append(Finding("L4", f, t.line, "`include` (include!, or an alias of it) in a class U/P "
                                                    "crate's src/"))
            elif t.is_ident("macro_use") and s.in_attr(k):
                out.append(Finding("L4", f, t.line, "#[macro_use] (at any depth, cfg_attr included) in a class "
                                                    "U/P crate's src/"))
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


def attr_names(s: rslex.Structure, a: tuple[int, int]) -> list[rslex.Tok]:
    return s.code[a[0]:a[1] + 1]


def macro_danger(c: list[rslex.Tok], lo: int, hi: int) -> str | None:
    """What in code tokens c[lo:hi] an exported macro may not carry: what the compiler cannot
    see once it is expanded in ANOTHER crate (rustc suppresses `unsafe_code` for spans from an
    external macro), or what splices a file in relative to the CALLER's file (`include!` and
    `#[path]` resolve against the outermost call site). None if nothing."""
    for k in range(lo, hi):
        t = c[k]
        if t.is_ident(UNSAFE):
            return f"`{UNSAFE}`"
        if any(t.is_ident(m) for m in (*rslex.ASM_MACROS, "include")):
            return f"`{t.text}`"
        if t.is_ident("path") and k + 1 < hi and c[k + 1].is_punct("="):
            return "`#[path]`"
        if t.is_ident("macro_rules") and k + 1 < hi and c[k + 1].is_punct("!"):
            return "a nested `macro_rules!`"
        if t.is_ident("mod") and (
                (k + 2 < hi and c[k + 1].kind == "ident" and c[k + 2].is_punct(";"))
                or (k + 3 < hi and c[k + 1].is_punct("$") and c[k + 3].is_punct(";"))):
            return "an out-of-line `mod`"
    return None


def lex_macros(tree: Tree, cfg: Config) -> list[Finding]:
    """L5, in EVERY crate: an exported macro carries nothing the compiler cannot see once it is
    expanded elsewhere (`macro_danger`).

    Exported means any attribute naming `macro_export` at any depth (`#[cfg_attr(all(),
    macro_export)]` exports; an exact `#[macro_export]` match missed it, review 2026-10-04), or
    a body that can define an exported macro: one naming `macro_export`, or one that puts a
    `$fragment` attribute on a nested `macro_rules!` (`#[$m] macro_rules! …`, invoked with
    `macro_export`). Every crate, not class U alone: a forbid crate's safe file has no keyword
    (L1), but it can carry an exported macro wrapping `include!` or `#[path]` (review
    2026-10-04)."""
    del cfg
    out = []
    for f in tree.files:
        s = tree.struct(f)
        c = s.code
        for it in s.items:
            if it.kind != "macro_rules" or it.body_open is None:
                continue
            exported = any(x.is_ident("macro_export") for a in it.attrs for x in attr_names(s, a))
            body_exports = any(c[k].is_ident("macro_export") for k in range(it.body_open, it.end + 1))
            # A nested `macro_rules!` in a body that has ANY attribute carrying a `$fragment` can be
            # given `macro_export` by its caller (`#[$m]`, or `$(#[$m])*` before it, which no
            # adjacency test sees). Conservative on purpose: such a body fails below, because a
            # nested `macro_rules!` is itself on the list.
            nested = any(c[k].is_ident("macro_rules") and k + 1 < len(c) and c[k + 1].is_punct("!")
                         for k in range(it.body_open + 1, it.end))
            if nested and any(it.body_open < o < it.end and any(x.is_punct("$") for x in c[o:e + 1])
                              for o, e, _ in s.attrs):
                body_exports = True
            if not (exported or body_exports):
                continue
            what = macro_danger(c, it.body_open + 1, it.end)
            if what is not None:
                out.append(Finding("L5", f, c[it.kw].line,
                                   f"exported macro `{it.name}!` carries {what} in its body: expanded in another "
                                   "crate it is invisible to the compiler location gate, or it splices a file "
                                   "relative to the caller's"))
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


def indented_blocks(lines: list[tuple[int, str]]) -> list[list[tuple[int, str]]]:
    """CommonMark indented code blocks, which rustdoc runs as Rust doctests: lines indented at
    least four columns past the run's common indentation (rustdoc unindents doc fragments by
    it), opened after a blank line or at the start, outside a fence. A list item's indented
    paragraph reads the same way; that over-reads, which is the safe direction."""
    nonblank = [len(t) - len(t.lstrip(" ")) for _, t in lines if t.strip()]
    if not nonblank:
        return []
    base = min(nonblank)
    out: list[list[tuple[int, str]]] = []
    cur: list[tuple[int, str]] = []
    prev_blank, in_fence = True, None
    for ln, t in lines:
        m = re.match(r"^\s*(```+|~~~+)", t)
        if in_fence is not None:
            if re.match(rf"^\s*{re.escape(in_fence[0])}{{{len(in_fence)},}}\s*$", t):
                in_fence = None
            prev_blank = False
            continue
        if m and len(t) - len(t.lstrip(" ")) < base + 4:
            if cur:
                out.append(cur)
                cur = []
            in_fence = m.group(1)
            continue
        indent = len(t) - len(t.lstrip(" "))
        if t.strip() and indent >= base + 4 and (cur or prev_blank):
            cur.append((ln, t[base + 4:]))
        elif not t.strip() and cur:
            cur.append((ln, ""))
        elif cur:
            out.append(cur)
            cur = []
        prev_blank = not t.strip()
    if cur:
        out.append(cur)
    return out


def str_value(tok: rslex.Tok) -> str | None:
    """The value of a string literal token (escapes decoded), or None for byte/C strings."""
    text = tok.text
    if text.startswith(("b", "c")):
        return None
    if text.startswith("r"):
        hashes = len(text) - len(text[1:].lstrip("#")) - 1
        return text[2 + hashes:len(text) - 1 - hashes]
    body, out, i = text[1:-1], [], 0
    while i < len(body):
        ch = body[i]
        if ch != "\\":
            out.append(ch)
            i += 1
            continue
        nxt = body[i + 1] if i + 1 < len(body) else ""
        if nxt == "\n":  # a line continuation swallows the next line's leading whitespace
            i += 2
            while i < len(body) and body[i] in rslex.RUST_WS:
                i += 1
            continue
        simple = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", "'": "'", '"': '"'}
        if nxt in simple:
            out.append(simple[nxt])
            i += 2
        elif nxt == "x":
            out.append(chr(int(body[i + 2:i + 4], 16)))
            i += 4
        elif nxt == "u":
            e = body.index("}", i)
            out.append(chr(int(body[i + 3:e].replace("_", ""), 16)))
            i = e + 1
        else:
            out.append(nxt)
            i += 2
    return "".join(out)


def doc_attr_values(tree: Tree, f: str) -> tuple[list[tuple[int, list[tuple[int, str]]]], list[Finding]]:
    """`#[doc = …]` / `#![doc = …]` at any depth (cfg_attr included): the string literals of the
    value, concatenated, and `include_str!(<literal>)` resolved against this file (raw strings
    included). A value this cannot read is a finding, never a skip (fail closed)."""
    s = tree.struct(f)
    c = s.code
    runs, out = [], []
    for o, e, _ in s.attrs:
        k = o
        while k <= e:
            if not (c[k].is_ident("doc") and k + 1 <= e and c[k + 1].is_punct("=")):
                k += 1
                continue
            j, depth, parts = k + 2, 0, []
            while j < e:
                x = c[j]
                if x.kind == "punct" and x.text in "([{":
                    depth += 1
                elif x.kind == "punct" and x.text in ")]}":
                    if depth == 0:
                        break
                    depth -= 1
                elif x.is_punct(",") and depth == 0:
                    break
                if x.is_ident("include_str"):
                    # exactly `include_str ! ( <literal> )`; anything else cannot be resolved here
                    lit = c[j + 3] if j + 4 < len(c) and c[j + 1].is_punct("!") and c[j + 2].is_punct("(") \
                        and c[j + 4].is_punct(")") else None
                    val = str_value(lit) if lit is not None and lit.kind == "str" else None
                    md = (tree.root / f).parent / val if val is not None else None
                    if md is None or not md.is_file():
                        out.append(Finding("L10", f, x.line, "a `doc = include_str!(…)` whose file this gate "
                                                             "cannot resolve (fail closed): name it with a string "
                                                             "literal relative to this file"))
                    else:
                        parts.append(md.read_text())
                    j += 5
                    continue
                if x.kind == "str":
                    v = str_value(x)
                    if v is not None:
                        parts.append(v)
                j += 1
            if parts:
                text = "".join(parts)
                runs.append((c[k].line, [(c[k].line, ln) for ln in text.split("\n")]))
            k = j
    return runs, out


def lex_doctests(tree: Tree, cfg: Config) -> list[Finding]:
    """L10: no `unsafe` in Rust doctest code outside the perimeter: a doc-comment fence, a
    `#[doc = …]` attribute's fence (cfg_attr and include_str! included), a CommonMark indented
    code block, and any string literal that holds a fence or an indented block (a macro can turn
    a call-site literal into a doc: `#[doc = $doc]`)."""
    out = []
    for f in tree.files:
        if may_hold_unsafe(f, cfg):
            continue
        toks = tree.toks(f)
        runs = doc_lines(toks)
        attr_runs, errs = doc_attr_values(tree, f)
        out += errs
        runs += attr_runs
        for t in tree.struct(f).code:
            if t.kind == "str":
                v = str_value(t)
                if v is not None and ("```" in v or "~~~" in v or "\n    " in v):
                    runs.append((t.line, [(t.line, ln) for ln in v.split("\n")]))
        for _, lines in runs:
            for ln, info, body in fences(lines):
                if fence_is_rust(info):
                    hit = fence_has_unsafe(body)
                    if hit is not None:
                        out.append(Finding("L10", f, hit, f"`{UNSAFE}` in a doctest fence (from line {ln})"))
            for block in indented_blocks(lines):
                hit = fence_has_unsafe(block)
                if hit is not None:
                    out.append(Finding("L10", f, hit, f"`{UNSAFE}` in an indented (four-space) doctest block"))
    return sorted(set(out))


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
            if rslex.shebang_len(tree.text(f)):
                # rustc strips a first line it reads as a shebang, and its rule is subtle enough
                # (`#!/**/[…]` is Rust, `#!/x` is not) that a gate which agrees with it today is
                # one edge case from hiding a line. No tracked file has one: none may.
                findings.append(Finding("L0", f, 1, "a first line rustc would strip as a shebang: not allowed "
                                                    "in a tracked .rs file (fail closed)"))
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

SIZE_COLUMNS = ("tokens", *rslex.KINDS, "macro_unsafe", "exports")
SIZES_TSV = "scripts/ci/perimeter/sizes.tsv"
REASON_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}): (.*?)\s*— (.+)$")


def code_tokens(s: rslex.Structure) -> int:
    """Code tokens outside items under exactly `#[cfg(test)]`.

    Tokens, not lines (review 2026-10-04): a line count measures formatting, so `#[rustfmt::skip]`,
    a macro body rustfmt never touches, or two statements joined on one line add code while the
    count stays put or falls, and a fall needs no reason. A token count moves with the code."""
    tests = s.test_ranges()
    return sum(1 for k in range(len(s.code)) if not rslex.in_ranges(k, tests))


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


C_TOKEN = re.compile(r'"(?:\\.|[^"\\\n])*"|\'(?:\\.|[^\'\\\n])*\'|[A-Za-z_]\w*|\d[\w.]*|\S')


def c_code_tokens(text: str) -> int:
    """C tokens after comments are stripped: identifiers, numbers, string and character
    literals each count one, and every other non-space character one (exact, and moved by
    code, not by layout)."""
    return len(C_TOKEN.findall(strip_c(text)))


def crate_files(tree: Tree, crate: str) -> list[str]:
    return [f for f in tree.files if crate_of(f, [crate]) == crate]


def measure_file(tree: Tree, f: str, crate_structs: list[rslex.Structure]) -> dict[str, int]:
    s = tree.struct(f)
    row = {c: 0 for c in SIZE_COLUMNS}
    row["tokens"] = code_tokens(s)
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
        row["tokens"] = c_code_tokens((tree.root / f).read_text())
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
            "# Every count is EXACT. `tokens` counts code tokens outside #[cfg(test)] (C: tokens after\n"
            "# comments are stripped). A decrease needs only the new number (`perimeter.py sizes --update`).\n"
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
# ⊘ Widened 2026-10-04 (review): "the caller keeps it alive", "its caller established that…",
# "validated by the caller two frames up" each hand a precondition to a caller and matched none
# of the first family.
CALLER_OBLIGATION = re.compile(
    r"\b(?:every|each|all) callers?\b"
    r"|\bcallers? (?:size|sizes|bound|bounds|must|ensure|ensures|guarantee|guarantees|pass|passes|promise"
    r"|keeps?|kept|establish|established|establishes|validated|validates|checked|checks|holds?)\b"
    r"|\b(?:validated|checked|established|guaranteed|ensured) by (?:the|its|a) caller\b"
    r"|\bcaller'?s obligation\b|\bthe caller passed\b|\bthe caller \(|\bcaller two frames up\b",
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


def l8b_sites(tree: Tree, cfg: Config) -> list[str]:
    """L8b: a SAFE, non-extern fn in a perimeter file with a raw-pointer (`*const`/`*mut`) or
    `RawFd` parameter. Its body cannot check what the pointer or descriptor names, so the
    precondition is its caller's (rule (a)) whatever its SAFETY comments say. Safe `extern "C"`
    fns are E9's. Keyed `<file>::<Type::>fn`; an exact set that only shrinks."""
    out = []
    for f in tree.files:
        if not is_perimeter_file(f, cfg):
            continue
        s = tree.struct(f)
        c = s.code
        tests = s.test_ranges()
        for it in s.items:
            if it.kind != "fn" or UNSAFE in it.qualifiers or "extern" in it.qualifiers or rslex.in_ranges(it.kw, tests):
                continue
            if it.parent is not None and it.parent.kind == "extern_block":
                continue
            k, angle = it.kw + 2, 0  # past `fn name`; skip the generic list, `<…>` is not bracket-matched
            while k < len(c) and not (angle == 0 and c[k].is_punct("(")):
                angle += 1 if c[k].is_punct("<") else -1 if c[k].is_punct(">") else 0
                k += 1
            if k >= len(c):
                continue
            params = c[k:s.match[k] + 1]
            raw = any(params[i].is_punct("*") and (params[i + 1].is_ident("const") or params[i + 1].is_ident("mut"))
                      for i in range(len(params) - 1))
            if raw or any(t.is_ident("RawFd") for t in params):
                owner = f"{it.parent.name}::" if it.parent is not None and it.parent.kind in ("impl", "trait") else ""
                out.append(f"{f}::{owner}{it.name}")
    return sorted(set(out))


def l8b_findings(sites: list[str], baseline: list[str]) -> list[Finding]:
    out = []
    for key in sorted(set(sites) - set(baseline)):
        out.append(Finding("L8b", key.split("::", 1)[0], 0, f"safe fn `{key.split('::', 1)[1]}` takes a raw pointer or "
                                                            "RawFd: make it `unsafe fn` with a `# Safety`, or take a "
                                                            "type that carries its own bound"))
    for key in sorted(set(baseline) - set(sites)):
        out.append(Finding("L8b", key.split("::", 1)[0], 0, f"[l8].raw_params lists `{key}`, which is gone: remove it"))
    return out


def mint_aliases(s: rslex.Structure, names: set[str]) -> tuple[set[str], set[str]]:
    """Names a file introduces for a mint name: `use … name as Alias` and `type Alias = …name…`.
    Returns (every alias, the `pub` ones): a `pub` alias is a name other crates can use too
    (`pub type P = Nvos21Parameters;` in kf-abi, used as `kf_abi::P` in kf-host, review
    2026-10-04). Raw spellings count: `r#Nvos21Parameters` is the same name."""
    c = s.code
    found: set[str] = set()
    public: set[str] = set()
    uses = [it for it in s.items if it.kind == "use"]
    for k in range(len(c) - 2):
        if c[k].kind == "ident" and c[k].text in names and c[k + 1].is_ident("as") and c[k + 2].kind == "ident":
            found.add(c[k + 2].text)
            if any(u.start <= k <= u.end and "pub" in u.qualifiers for u in uses):
                public.add(c[k + 2].text)
    for it in s.items:
        if it.kind == "type" and any(x.kind == "ident" and x.text in names for x in c[it.kw:it.end + 1]):
            found.add(it.name)
            if "pub" in it.qualifiers:
                public.add(it.name)
    return found, public


def mint_sites(tree: Tree, cfg: Config, kf3_crates: list[str]) -> dict[str, list[int]]:
    """L9: mint-name sites in src/ of kf3-graph crates, outside cfg(test), perimeter files and
    kf-abi. Aliases are followed to a fixpoint: a crate's own, and every crate's `pub` ones
    (kf-abi's included, though its sites are not counted)."""
    names = set(cfg.raw.get("mint", {}).get("names", []))
    files_of = {cr: [f for f in crate_files(tree, cr) if in_src(f, cr) and not is_perimeter_file(f, cfg)]
                for cr in kf3_crates}
    local: dict[str, set[str]] = {cr: set(names) for cr in kf3_crates}
    shared: set[str] = set()
    while True:
        changed = False
        for cr in kf3_crates:
            known = local[cr] | shared
            for f in files_of[cr]:
                found, public = mint_aliases(tree.struct(f), known)
                if not found <= local[cr] or not public <= shared:
                    local[cr] |= found
                    pass
                    changed = True
        if not changed:
            break
    out: dict[str, list[int]] = {}
    for cr in kf3_crates:
        if cr.endswith("/kf-abi"):
            continue
        crate_names = local[cr] | shared
        for f in files_of[cr]:
            s = tree.struct(f)
            tests = s.test_ranges()
            hits = [t.line for k, t in enumerate(s.code)
                    if t.kind == "ident" and t.text in crate_names and not rslex.in_ranges(k, tests)]
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

# `rustdoc`: an interposed rustdoc writes the JSON the export table is built from (review
# 2026-10-04: a committed `[build] rustdoc = "./rd.sh"` ran and its output was used).
FORBIDDEN_CONFIG_KEYS = ("rustflags", "rustdocflags", "rustc", "rustc-wrapper", "rustc-workspace-wrapper",
                         "rustdoc")
# Whole tables a committed cargo config may not have: `[unstable]` (turned on by the export
# generator's RUSTC_BOOTSTRAP), and every way to swap a dependency's source under its own name
# (`[source]` replace-with, `[registries]`, `[registry]`, `[patch]`, `[paths]`).
FORBIDDEN_CONFIG_TABLES = ("unstable", "source", "registries", "registry", "patch", "paths")
FORBIDDEN_ENV = re.compile(r"^(RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_TARGET_\w+_RUSTFLAGS|RUSTDOCFLAGS|"
                           r"CARGO_ENCODED_RUSTDOCFLAGS|RUSTC|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|"
                           r"RUSTC_BOOTSTRAP|CARGO_BUILD_RUSTFLAGS|CARGO_BUILD_RUSTC\w*|RUSTDOC|"
                           r"CARGO_BUILD_RUSTDOC\w*|CARGO_UNSTABLE_\w+|CARGO_REGISTRIES_\w+|"
                           r"CARGO_SOURCE_\w+|CARGO_REGISTRY_\w+)$")


def cargo_config_violations(path: str, data: dict) -> list[Finding]:
    out = []
    for t in FORBIDDEN_CONFIG_TABLES:
        if t in data:
            out.append(Finding("M4", path, 1, f"cargo config has a `[{t}]` table"))

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


DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def manifest_source_violations(path: str, data: dict) -> list[Finding]:
    """M7 (manifest half): no dependency names another registry (`registry =`, `registry-index =`),
    and no manifest patches or replaces a dependency (`[patch]`, `[replace]`): each swaps what
    a package NAME resolves to, which a name-only external list cannot see."""
    out = []
    for t in ("patch", "replace"):
        if t in data:
            out.append(Finding("M7", path, 1, f"a `[{t}]` table"))
    tables = [data, data.get("workspace") or {}, *(data.get("target") or {}).values()]
    for tbl in tables:
        for dt in DEP_TABLES:
            for name, spec in (tbl.get(dt) or {}).items():
                if isinstance(spec, dict) and ({"registry", "registry-index"} & spec.keys()):
                    out.append(Finding("M7", path, 1, f"dependency `{name}` names another registry"))
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


def kf3_overlay_findings(root: Path, cfg: Config, tracked: list[str]) -> list[Finding]:
    """K6: the kf3 C overlay is exactly `[c].files` plus its meson.build, and meson.build is the
    reviewed one (sha256). build_kf3.sh copies EVERY `*.c`/`*.h` of the directory into QEMU, and
    meson.build decides what is compiled and with which flags (`c_args`, `files(…)`, link_args):
    an unlisted C file would ship with no size row, and a `c_args` change would switch on code
    paths (review 2026-10-04)."""
    c = cfg.raw.get("c", {})
    d = c.get("overlay")
    if not d:
        return []
    meson = f"{d}/meson.build"
    want = set(c.get("files", [])) | {meson}
    got = {f for f in tracked if under(f, d)}
    out = [Finding("K6", f, 1, "a file in the kf3 overlay that perimeter.toml [c].files does not list "
                               "(build_kf3.sh copies every overlay source into QEMU)") for f in sorted(got - want)]
    out += [Finding("K6", f, 1, "[c].files lists a file the overlay no longer has") for f in sorted(want - got)]
    if meson in got and hashlib.sha256((root / meson).read_bytes()).hexdigest() != c.get("meson_sha256"):
        out.append(Finding("K6", meson, 1, "kf3's meson.build changed: re-review what it compiles and with which "
                                           "flags, then update perimeter.toml [c].meson_sha256"))
    return out


def run_manifest(root: Path, cfg: Config, members: list[str] | None = None,
                 manifests: list[str] | None = None, configs: list[str] | None = None,
                 workflows: list[str] | None = None, overlay: list[str] | None = None) -> list[Finding]:
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
    # M7's manifest half, and M10: every package is edition 2024. The tokenizer's keyword covers
    # `#[unsafe(no_mangle)]`, `export_name` and `link_section` only because edition 2024 requires
    # the `unsafe(…)` wrapper; under 2021 those attributes carry no keyword at all.
    root_ws_pkg = (tomllib.loads((root / "Cargo.toml").read_text()).get("workspace") or {}).get("package") or {}
    for m in manifests:
        data = tomllib.loads((root / m).read_text())
        out += manifest_source_violations(m, data)
        pkg = data.get("package")
        if pkg is None:
            continue
        ed = pkg.get("edition")
        if isinstance(ed, dict) and ed.get("workspace"):
            ws_pkg = root_ws_pkg if "workspace" not in data else (data["workspace"].get("package") or {})
            ed = ws_pkg.get("edition")
        if ed != "2024":
            out.append(Finding("M10", m, 1, f"edition {ed!r}: every package is edition 2024 (the unsafe "
                                            "attributes carry a keyword only there)"))
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
    # K6: the kf3 overlay's file set and meson.build
    d = cfg.raw.get("c", {}).get("overlay")
    out += kf3_overlay_findings(root, cfg, overlay if overlay is not None else (git_ls(root, f"{d}/*") if d else []))
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
    # ★ (name, SOURCE), not the name alone: a dependency resolved from another registry, or a
    # `[source]` replacement, keeps the name `libc` and passed a names-only list (review 2026-10-04).
    pinned_src = ext.get("source")
    for m in allmeta:
        for p in m["packages"]:
            if p["source"] is not None and p["source"] != pinned_src:
                out.append(Finding("M7", "Cargo.lock", 1, f"external package `{p['name']}` from {p['source']!r}, "
                                                          f"not the pinned source {pinned_src!r}"))
    # The exempt package's target list, exactly: cargo discovers `src/bin/<x>/main.rs` (any name,
    # `target` included), so a new target under the exempt path is a reviewed change here.
    for ex in cfg.raw.get("exempt", []):
        want_t = ex.get("targets")
        for m in allmeta:
            for p in m["packages"]:
                if p["source"] is None and rel_dir(root, p["manifest_path"]) == ex["path"]:
                    got_t = sorted(f"{'/'.join(t['kind'])}:{t['name']}" for t in p["targets"])
                    if want_t is None or got_t != sorted(want_t):
                        out.append(Finding("M8", f"{ex['path']}/Cargo.toml", 1,
                                           f"the exempt package's targets {got_t} != [[exempt]].targets {want_t}"))
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


def lock_violations(path: str, data: dict, pinned_source: str | None) -> list[Finding]:
    """M7 (lock half): every non-path package in a lock file comes from the pinned source and
    carries a 64-hex checksum (a registry package without one is not what it says it is)."""
    out = []
    for p in data.get("package", []):
        src = p.get("source")
        if src is None:
            continue
        if src != pinned_source:
            out.append(Finding("M7", path, 1, f"`{p.get('name')}` from {src!r}, not {pinned_source!r}"))
        if not re.fullmatch(r"[0-9a-f]{64}", str(p.get("checksum", ""))):
            out.append(Finding("M7", path, 1, f"`{p.get('name')}` has no checksum"))
    return out


def run_metadata(root: Path, cfg: Config) -> list[Finding]:
    meta = cargo_metadata(root)
    sm = {s["path"]: cargo_metadata(root, f"{s['path']}/Cargo.toml") for s in cfg.standalone}
    out = metadata_violations(root, cfg, meta, sm)
    for lock in ["Cargo.lock", *(f"{s['path']}/Cargo.lock" for s in cfg.standalone)]:
        if (root / lock).exists():
            out += lock_violations(lock, tomllib.loads((root / lock).read_text()),
                                   cfg.raw.get("external", {}).get("source"))
    return sorted(set(out))


# ---------------------------------------------------------------------------------------
# G1: the compiler location gate's verdicts (the log is written by rustc_location_wrapper.py)
# ---------------------------------------------------------------------------------------

DIAG_KINDS = [
    ("usage of an `unsafe` block", "blocks"),
    ("declaration of an `unsafe` function", "unsafe_fn"),
    ("implementation of an `unsafe` method", "unsafe_method"),
    ("declaration of an `unsafe` method", "unsafe_method"),
    ("implementation of an `unsafe` trait", "unsafe_impl"),
    ("declaration of an `unsafe` trait", "unsafe_trait"),
    ("usage of an `unsafe extern` block", "extern_blocks"),
    ("usage of the unsafe `", "unsafe_attrs"),
    ("usage of `core::arch::global_asm`", "asm"),
    ("usage of `core::arch::naked_asm`", "asm"),
]


def diag_kind(message: str) -> str:
    for prefix, kind in DIAG_KINDS:
        if message.startswith(prefix):
            return kind
    return "unknown"


def read_log(logdir: Path) -> list[dict]:
    recs = []
    for p in sorted(logdir.glob("*.json")):
        recs.append(json.loads(p.read_text()))
    return recs


def location_findings(recs: list[dict], cfg: Config, frozen_ok: bool) -> tuple[list[Finding], dict[str, int]]:
    """A diagnostic passes iff its primary file AND every macro call-site file are `*_unsafe.rs`
    under `<P>/src/`, P being the unit's OWN class U package; or it lies wholly under an exempt
    path and `debt.py frozen` passed. E0453 (an `allow` under forbid) never passes."""
    out: list[Finding] = []
    stats = {"units": len(recs), "diagnostics": 0, "outside": 0, "exempt": 0}
    per_pass: dict[str, int] = {}
    for r in recs:
        u = r["unit"]
        own = u["manifest_dir"]
        own_u = own in cfg.class_u
        for d in r["diags"]:
            stats["diagnostics"] += 1
            files = [d["file"], *(c[0] for c in d["callsites"])]
            where = f"{d['file']}:{d['line']}"
            if d["code"] == "E0453":
                out.append(Finding("G1", d["file"] or own, d["line"] or 0,
                                   f"E0453: an `allow(unsafe_code)` in a forbid unit ({u['crate_name']})"))
                stats["outside"] += 1
                continue
            if d["file"] is None:
                out.append(Finding("G1", own, 0, f"an unlocated `unsafe_code` diagnostic: {d['message']}"))
                stats["outside"] += 1
                continue
            if frozen_ok and all(f and any(under(f, e) for e in cfg.exempt) for f in files):
                stats["exempt"] += 1
                per_pass[u.get("pass", "?")] = per_pass.get(u.get("pass", "?"), 0) + 1
                continue
            ok = own_u and all(f and f.endswith("_unsafe.rs") and under(f, f"{own}/src") for f in files)
            if not ok:
                stats["outside"] += 1
                via = "" if not d["callsites"] else " via " + ", ".join(f"{c[0]}:{c[1]}" for c in d["callsites"])
                out.append(Finding("G1", d["file"], d["line"],
                                   f"{diag_kind(d['message'])} outside the perimeter of unit "
                                   f"{own} ({u['crate_name']}, class {u['class']}){via} [{where}]"))
    # ★ The exempt count is asserted, per pass and exactly, not printed (review 2026-10-04): the
    # path is hash-bound, but a count nobody checks would let a new exempt site through as a
    # changed number in a log.
    if frozen_ok:
        want: dict[str, int] = {}
        for ex in cfg.raw.get("exempt", []):
            for name, n in (ex.get("diagnostics") or {}).items():
                want[name] = want.get(name, 0) + n
        for name in sorted(set(want) | set(per_pass)):
            if per_pass.get(name, 0) != want.get(name, 0):
                out.append(Finding("G1", cfg.exempt[0] if cfg.exempt else "-", 0,
                                   f"pass {name}: {per_pass.get(name, 0)} exempt diagnostics, "
                                   f"[[exempt]].diagnostics says {want.get(name, 0)} (exact)"))
    return sorted(set(out)), stats


def compiler_counts(recs: list[dict]) -> dict[str, dict[str, set]]:
    """Per file: direct diagnostics per kind, deduplicated by (line, col); macro-expanded ones by
    (line, col, call-site chain) under the pseudo-kind `macro_unsafe`."""
    out: dict[str, dict[str, set]] = {}
    for r in recs:
        for d in r["diags"]:
            if d["code"] != "unsafe_code" or d["file"] is None:
                continue
            per = out.setdefault(d["file"], {})
            if d["callsites"]:
                key = (d["line"], d["col"], tuple(tuple(c[:3]) for c in d["callsites"]))
                per.setdefault("macro_unsafe", set()).add(key)
            else:
                per.setdefault(diag_kind(d["message"]), set()).add((d["line"], d["col"]))
    return out


def cross_check(actual: dict[str, dict], counts: dict[str, dict[str, set]]) -> list[Finding]:
    """SF4: for each perimeter file and kind, the compiler's count is at most the lexer's (the
    lexer is cfg-blind, so it can only be higher). Higher is a lexer bug, named."""
    out = []
    for f, per in sorted(counts.items()):
        if f not in actual:
            continue
        for kind, keys in sorted(per.items()):
            lex = actual[f].get(kind)
            if lex is None or len(keys) > lex:
                out.append(Finding("SF4", f, 0, f"compiler counts {len(keys)} `{kind}` sites, the tokenizer "
                                                f"{lex}: a tokenizer bug (it is cfg-blind, so it may only be higher)"))
    # Informational: where the tokenizer counts more than any pass compiled (cfg arms no pass
    # builds, or a macro defined but never expanded). Equal everywhere else.
    for f in sorted(actual):
        per = counts.get(f, {})
        for kind in (*rslex.KINDS, "macro_unsafe"):
            lex, comp = actual[f].get(kind, 0), len(per.get(kind, ()))
            if lex and comp < lex:
                print(f"  SF4 note: {f} {kind}: compiler {comp} < tokenizer {lex}")
    return out


def expected_units(meta: dict, pass_args: list[str], root: Path) -> tuple[set, list[str]]:
    """The (src, mode) units a `cargo check <pass_args>` must compile, and the targets it skips by
    `required-features`. mode: 'any', 'test', 'plain'."""
    sel = set(a for a in pass_args if a.startswith("--") and a in
              ("--all-targets", "--lib", "--bins", "--tests", "--examples", "--benches"))
    if "--all-targets" in sel:
        sel |= {"--lib", "--bins", "--tests", "--examples", "--benches"}
    feats: set[str] = set()
    all_features = "--all-features" in pass_args
    for i, a in enumerate(pass_args):
        if a == "--features" and i + 1 < len(pass_args):
            feats |= set(re.split(r"[,\s]+", pass_args[i + 1]))
        elif a.startswith("--features="):
            feats |= set(re.split(r"[,\s]+", a.split("=", 1)[1]))
    members = set(meta["workspace_members"])
    # `-p <name>` / `--package <name>` selects packages; without it, `--workspace` selects all
    picked = {pass_args[i + 1] for i, a in enumerate(pass_args) if a in ("-p", "--package") and i + 1 < len(pass_args)}
    picked |= {a.split("=", 1)[1] for a in pass_args if a.startswith("--package=")}
    want: set = set()
    skipped: list[str] = []
    for p in meta["packages"]:
        if p["id"] not in members or (picked and p["name"] not in picked):
            continue
        enabled = set(feats)
        if "--no-default-features" not in pass_args:
            stack = ["default"]
            while stack:
                f = stack.pop()
                if f in enabled and f != "default":
                    continue
                enabled.add(f)
                stack.extend(x for x in p["features"].get(f, []) if ":" not in x and "/" not in x)
        for t in p["targets"]:
            kinds = set(t["kind"])
            src = os.path.relpath(os.path.realpath(t["src_path"]), os.path.realpath(root))
            req = set(t.get("required-features") or [])
            if req and not all_features and not req <= enabled:
                skipped.append(f"{p['name']}:{t['name']} (required-features {sorted(req)})")
                continue
            if "custom-build" in kinds:
                want.add((src, "plain"))
                continue
            if kinds & {"lib", "rlib", "staticlib", "cdylib", "dylib", "proc-macro"}:
                if "--lib" in sel:
                    want.add((src, "plain"))
                if "--tests" in sel and t.get("test", True):
                    want.add((src, "test"))
            elif "bin" in kinds:
                if "--bins" in sel:
                    want.add((src, "plain"))
                if "--tests" in sel and t.get("test", True):
                    want.add((src, "test"))
            elif "test" in kinds:
                if "--tests" in sel:
                    want.add((src, "any"))
            elif "example" in kinds:
                if "--examples" in sel:
                    want.add((src, "plain"))
            elif "bench" in kinds:
                if "--benches" in sel:
                    want.add((src, "any"))
    return want, skipped


def reached_findings(recs: list[dict], passes: dict[str, tuple[dict, list[str]]], root: Path) -> tuple[list[Finding], int]:
    out = []
    total = 0
    for name, (meta, args) in sorted(passes.items()):
        units = [r["unit"] for r in recs if r["unit"]["pass"] == name]
        total += len(units)
        seen = {(u["src"], u["test"]) for u in units}
        want, skipped = expected_units(meta, args, root)
        for src, mode in sorted(want):
            ok = ((src, True) in seen if mode == "test" else (src, False) in seen if mode == "plain"
                  else (src, True) in seen or (src, False) in seen)
            if not ok:
                out.append(Finding("G1", src, 0, f"pass {name}: unit ({mode}) never reached the location wrapper "
                                                 "(a reused target dir, or a unit outside the wrapper)"))
        for sk in skipped:
            print(f"  pass {name}: skipped by required-features, covered by the tokenizer only: {sk}")
        if not units:
            out.append(Finding("G1", "-", 0, f"pass {name}: ZERO units reached the wrapper"))
    return out, total


def toolchain_roots() -> list[str]:
    """Where a unit may legitimately read `.rs` outside the checkout: the toolchain's sysroot (std)
    and cargo's registry and git checkouts."""
    roots = []
    r = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True)
    if r.returncode == 0 and r.stdout.strip():
        roots.append(os.path.realpath(r.stdout.strip()))
    home = os.environ.get("CARGO_HOME") or os.path.expanduser("~/.cargo")
    roots += [os.path.realpath(os.path.join(home, "registry")), os.path.realpath(os.path.join(home, "git"))]
    return roots


def depinfo_findings(recs: list[dict], root: Path, cfg: Config, tracked: set[str],
                     external_roots: list[str] | None = None) -> list[Finding]:
    """L0: every in-checkout `.rs` a wrapped unit's dep-info names is a tracked file; a class U unit
    reads no `.rs` from outside the checkout's tracked set (OUT_DIR included); every class U
    `*_unsafe.rs` is read by at least one unit."""
    out = []
    rroot = os.path.realpath(root)
    external_roots = toolchain_roots() if external_roots is None else external_roots
    read: set[str] = set()
    for r in recs:
        u = r["unit"]
        if not u.get("out_dir") or not u.get("crate_name"):
            continue
        d = Path(u["out_dir"]) / f"{u['crate_name']}{u.get('extra_filename', '')}.d"
        if not d.is_absolute():
            d = Path(u["cwd"]) / d
        if not d.exists():
            if u["rc"] == 0:
                out.append(Finding("L0", u["manifest_dir"], 0, f"no dep-info {d} for unit {u['crate_name']}"))
            continue
        text = d.read_text().replace("\\ ", "\0")
        for line in text.splitlines():
            _, sep, deps = line.partition(": ")
            if not sep:
                continue
            for dep in deps.split():
                dep = dep.replace("\0", " ")
                if not dep.endswith(".rs"):
                    continue
                pth = os.path.realpath(dep if os.path.isabs(dep) else os.path.join(u["cwd"], dep))
                if under(pth, rroot):
                    relp = os.path.relpath(pth, rroot)
                    read.add(relp)
                    if relp not in tracked:
                        out.append(Finding("L0", relp, 0, f"unit {u['crate_name']} compiled an untracked file"))
                elif u["class"] == "U" and not any(under(pth, x) for x in external_roots):
                    out.append(Finding("L0", pth, 0, f"class U unit {u['crate_name']} reads a .rs outside the "
                                                     "checkout's tracked files (OUT_DIR or elsewhere)"))
    for f in sorted(tracked):
        cr = crate_of(f, cfg.class_u)
        if cr and f.endswith("_unsafe.rs") and in_src(f, cr) and f not in read:
            out.append(Finding("L0", f, 0, "a class U perimeter file no compiled unit read (unreached)"))
    return sorted(set(out))


# ---------------------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------------------


def kf3_member_dirs(root: Path, cfg: Config) -> list[str]:
    import dependencies  # noqa: PLC0415 (scripts/ci sibling)
    meta = cargo_metadata(root)
    pk = {p["id"]: p for p in meta["packages"]}
    closure = dependencies.kf3_closure(meta, cfg.raw["kf3"]["root"])
    return sorted(rel_dir(root, pk[i]["manifest_path"]) for i in closure if pk[i]["source"] is None)


def sizes_validates(root: Path) -> list[str]:
    table = root / "docs/design/PERIMETER_EXPORTS.md"
    if not table.exists():
        return []
    import perimeter_exports as px  # noqa: PLC0415
    return px.table_counts(table.read_text())[1]


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
    validates: list[str] = []
    table = root / "docs/design/PERIMETER_EXPORTS.md"
    if exports is None and table.exists():
        import perimeter_exports as px  # noqa: PLC0415
        exports, validates = px.table_counts(table.read_text())
    actual = size_rows(tree, cfg, c_files, exports, validates)
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
    findings += l8b_findings(l8b_sites(tree, cfg), cfg.raw.get("l8", {}).get("raw_params", []))
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


def covered_crates(cfg: Config) -> list[str]:
    """G5's crates: the class U crates in kf3's graph, and every class P crate."""
    return sorted(set(cfg.kf3_crates()) | set(cfg.class_p))


def crate_package_name(root: Path, crate: str) -> str:
    return tomllib.loads((root / crate / "Cargo.toml").read_text())["package"]["name"]


def run_exports(root: Path, cfg: Config, json_dir: Path | None, write: bool, date: str,
                out_dir: Path | None = None) -> tuple[list, int]:
    import perimeter_exports as px  # noqa: PLC0415 (sibling; only this subcommand needs it)
    ex = cfg.raw.get("exports", {})
    targets = ex.get("targets", ["x86_64-unknown-linux-gnu"])
    crates = covered_crates(cfg)
    files = git_ls(root, "*.rs")
    perim = {f for f in files if is_perimeter_file(f, cfg) and crate_of(f, crates) is not None}
    unsafe_lines = px.unsafe_impl_lines(root, perim)
    parts = []
    fmt = cfg.raw.get("rustdoc_format", 61)
    for cr in crates:
        pkg = crate_package_name(root, cr)
        for tgt in targets:
            if json_dir is not None:
                path = json_dir / tgt / "doc" / f"{pkg.replace('-', '_')}.json"
                if not path.exists():
                    path = json_dir / f"{pkg.replace('-', '_')}.json"
            else:
                path = px.run_rustdoc(root, pkg, tgt, out_dir or rustdoc_dir(), cfg.raw.get("toolchain"))
            doc = px.Doc(json.loads(path.read_text()), fmt)
            parts.append(px.inventory(doc, perim, unsafe_lines))
    inv, types, names, conflicts = px.union_inventories(parts)
    findings: list = [px.Finding("E2", px.TABLE, 0, c) for c in conflicts]
    tpath = root / px.TABLE
    table, vdeps, errs = px.parse_table(tpath.read_text()) if tpath.exists() else ({}, {}, [])
    findings += errs
    table_kinds = {k: r.kind for k, r in table.items()}
    carrying = px.address_carrying(types)
    fds = px.fd_carrying(types)
    inv.update(px.auto_rows(types, table_kinds, carrying, fds))
    # E11, derived by the tokenizer over each covered crate's src/
    crate_files = {cr: [f for f in files if crate_of(f, [cr]) == cr and in_src(f, cr)] for cr in crates}
    derived = px.validation_deps(root, crate_files, lambda f: f in perim)
    if write:
        lanes = ex.get("lanes", {})
        merged = {}
        for k, g in inv.items():
            old = table.get(k)
            if old is not None:
                g.checks, g.tests, g.status = old.checks, old.tests, old.status
                if g.kind == "type":
                    g.kind = old.kind
            else:
                g.status = (f"LANE:{lanes[k[0]]}: {date}: unreviewed; this file is edited by that lane"
                            if k[0] in lanes else f"OPEN: {date}: unreviewed")
                if g.kind == "type":
                    g.kind = suggest_type_kind(types.get((k[0], k[1])), carrying, fds)
            merged[k] = g
        vd = {f: (vdeps[f][0], vdeps[f][1]) if f in vdeps else ("?", "unclassified: " + ", ".join(
            sorted(derived[f])[:6])) for f in derived}
        notes = ("**Crates covered:** " + ", ".join(f"`{c}`" for c in crates) + ". The grader crates and the guest "
                 "firmware are not linked into kf3 (scripts/ci/dependencies.py); their perimeter files are counted "
                 "in `scripts/ci/perimeter/sizes.tsv` and have no rows here.")
        tpath.write_text(px.render_table(merged, vd, date, notes))
        table, vdeps, errs = px.parse_table(tpath.read_text())
        findings += errs
    debt = set(ex.get("debt", {}).get("items", []))
    findings += px.check_rows(root, inv, table, types, carrying, fds, set(cfg.raw.get("skip_guards", {}).get("macros", [])),
                              None, int(ex.get("e9_baseline", 0)), debt)
    findings += px.unsafe_fn_safety_findings(root, sorted(perim), debt)
    findings += px.reach_findings(root, sorted(perim), names)
    feats = {cr: set(tomllib.loads((root / cr / "Cargo.toml").read_text()).get("features", {})) for cr in crates}
    findings += px.cfg_findings(root, sorted(perim), set(cfg.raw.get("cfg", {}).get("allowed", [])), feats,
                                lambda f: crate_of(f, crates))
    for f in sorted(set(derived) - set(vdeps)):
        findings.append(px.Finding("E11", px.TABLE, 0, f"add `{f}` to Validation dependencies (named: "
                                                       f"{', '.join(sorted(derived[f])[:8])})"))
    for f in sorted(set(vdeps) - set(derived)):
        findings.append(px.Finding("E11", px.TABLE, vdeps[f][2], f"remove `{f}` from Validation dependencies"))
    for f, (role, _, line) in sorted(vdeps.items()):
        if role not in ("VALIDATES", "USES"):
            findings.append(px.Finding("E11", px.TABLE, line, f"`{f}`: role must be VALIDATES or USES, got {role!r}"))
    # E11b: a VALIDATES file is mutated by the E3c workflow (`perimeter.py validates`), so a pull
    # request touching it must start that workflow: its `paths:` filter names every one.
    wf = root / ".github/workflows/perimeter-mutants.yml"
    wf_text = wf.read_text() if wf.exists() else None  # a fixture workspace has no workflows
    for f, (role, _, line) in sorted(vdeps.items()):
        if role == "VALIDATES" and wf_text is not None and f"'{f}'" not in wf_text:
            findings.append(px.Finding("E11b", px.TABLE, line, f"VALIDATES `{f}` is not in perimeter-mutants.yml's "
                                                               "pull_request paths"))
    open_rows = sum(1 for r in table.values() if r.status.startswith("OPEN"))
    lane_rows = sum(1 for r in table.values() if r.status.startswith("LANE"))
    ok_rows = sum(1 for r in table.values() if r.status == "OK")
    print(f"  rows={len(table)} OK={ok_rows} OPEN={open_rows} LANE={lane_rows} validation_deps={len(vdeps)}")
    return sorted(findings), len(table)


def rustdoc_dir() -> Path:
    return Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "kf-rustdoc"


def run_k4(root: Path, cfg: Config, json_dir: Path) -> list[Finding]:
    """K4 (§8.1): kf3.h's prototypes must agree with the Rust `no_mangle` signatures. The names-only
    link closure cannot see a parameter whose type changed; a C compiler can. Its known positive
    runs every time: one parameter flipped in memory must fail to compile."""
    import perimeter_exports as px  # noqa: PLC0415
    tgt = cfg.raw.get("exports", {}).get("targets", ["x86_64-unknown-linux-gnu"])[0]
    path = json_dir / tgt / "doc" / "kf_qemu.json"
    if not path.exists():
        path = json_dir / "kf_qemu.json"
    doc = px.Doc(json.loads(path.read_text()), cfg.raw.get("rustdoc_format", 61))
    protos = px.k4_prototypes(doc)
    header = (root / "qemu/hw/misc/kf3/kf3.h").read_text()
    declared = set(re.findall(r"\b(kf3_\w+)\s*\(", rslex_strip_c(header)))
    out = []
    names = {n for n, _ in protos}
    for n in sorted(names - declared):
        out.append(Finding("K4", "qemu/hw/misc/kf3/kf3.h", 0, f"`{n}` is exported by kf-qemu and not declared"))
    for n in sorted(declared - names):
        out.append(Finding("K4", "qemu/hw/misc/kf3/kf3.h", 0, f"`{n}` is declared and kf-qemu exports no such fn"))
    rc, err = px.k4_compile(root, [p for _, p in protos])
    if rc != 0:
        out.append(Finding("K4", "qemu/hw/misc/kf3/kf3.h", 0, f"prototypes conflict with kf3.h:\n{err[:3000]}"))
    flipped, done = [], False
    for _, p in protos:
        if not done and "uint64_t" in p.split("(", 1)[1]:
            head, args_ = p.split("(", 1)
            p, done = head + "(" + args_.replace("uint64_t", "uint32_t", 1), True
        flipped.append(p)
    rc2, err2 = px.k4_compile(root, flipped)
    if not done or rc2 == 0 or "conflicting types" not in err2:
        out.append(Finding("K4", "qemu/hw/misc/kf3/kf3.h", 0, "KNOWN POSITIVE FAILED: a flipped parameter type "
                                                              "compiled; this check cannot see a mismatch"))
    print(f"  K4 prototypes={len(protos)} declared={len(declared)} known_positive={'fired' if rc2 else 'SILENT'}")
    return out


def rslex_strip_c(text: str) -> str:
    return strip_c(text)


def suggest_type_kind(t, carrying: set[str], fds: set[str]) -> str:
    if t is None:
        return "plain data"
    if t.repr_c:
        return "FFI struct"
    if "Drop" in t.traits or t.name in fds:
        return "owning handle"
    if t.has_lifetime:
        return "borrowed view"
    if t.name in carrying:
        return "owning handle"
    return "plain data"


def report(name: str, findings: list[Finding], extra: str = "") -> int:
    for f in findings:
        print(f)
    print(f"PERIMETER_{name.upper()} findings={len(findings)}{extra}")
    return 1 if findings else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=("lex", "manifest", "metadata", "sizes", "location", "reached", "depinfo",
                                    "passes", "toolchain", "exports", "k4", "mutants", "validates"))
    ap.add_argument("--crate", help="mutants/validates: the crate directory (crates/<name>) of a full run")
    ap.add_argument("--require-caught", action="store_true",
                    help="mutants: a full run over --crate: every OK fn row there needs a caught mutant, and the "
                         "baseline must have succeeded")
    ap.add_argument("--outcomes", type=Path, action="append", default=[],
                    help="mutants: cargo-mutants' mutants.out/outcomes.json (repeatable)")
    ap.add_argument("--json-dir", type=Path, help="exports: pre-generated rustdoc JSON (default: run rustdoc)")
    ap.add_argument("--write", action="store_true", help="exports: merge the inventory into the table")
    ap.add_argument("--date", default=None, help="exports --write: the date new rows carry (default: today)")
    ap.add_argument("--root", type=Path, default=ROOT, help="the checkout to judge (default: this script's)")
    ap.add_argument("--config", type=Path, help="perimeter.toml to use (default: <root>/scripts/ci/perimeter.toml)")
    ap.add_argument("--log", type=Path, help="location/reached/depinfo/sizes: the G1 wrapper log directory")
    ap.add_argument("--frozen-ok", action="store_true", help="location: `debt.py frozen` passed in this job")
    ap.add_argument("--base", help="sizes: the git ref whose sizes.tsv a rise is judged against")
    ap.add_argument("--update", action="store_true", help="sizes: write decreases and drop stale rows")
    args = ap.parse_args(argv)
    root = args.root.resolve()
    cfg = Config(tomllib.loads(args.config.read_text())) if args.config else Config.load(root)
    if args.cmd == "toolchain":
        print(cfg.raw["toolchain"])
        return 0
    if args.cmd == "passes":
        for p in cfg.location_passes():
            env = ",".join(f"{k}={v}" for k, v in sorted(p["env"].items()))
            print("\t".join([p["name"], p["manifest"], env or "-", " ".join(p["args"])]))
        return 0
    if args.cmd == "lex":
        findings, tree = run_lex(root, cfg)
        return report("lex", findings, f" files={len(tree.files)}")
    if args.cmd == "manifest":
        return report("manifest", run_manifest(root, cfg))
    if args.cmd == "metadata":
        return report("metadata", run_metadata(root, cfg))
    if args.cmd == "sizes":
        findings, totals = run_sizes(root, cfg, args.base, args.update)
        if args.log is not None:
            tree = Tree(root, git_ls(root, "*.rs"))
            actual = size_rows(tree, cfg, cfg.raw.get("c", {}).get("files", []), {}, sizes_validates(root))
            findings = sorted(findings + cross_check(actual, compiler_counts(read_log(args.log))))
        for cr, t in sorted(totals.items()):
            print(f"  {cr}: {fmt_row(t)}")
        return report("sizes", findings, f" base={args.base or '-'} cross_check={'yes' if args.log else 'no'}")
    if args.cmd == "exports":
        import datetime  # noqa: PLC0415
        date = args.date or datetime.date.today().isoformat()
        findings, n = run_exports(root, cfg, args.json_dir, args.write, date)
        return report("exports", findings, f" rows={n}")
    if args.cmd == "validates":
        import perimeter_exports as px  # noqa: PLC0415
        _, vfiles = px.table_counts((root / px.TABLE).read_text())
        for f in vfiles:
            if args.crate is None or f.startswith(args.crate.rstrip("/") + "/"):
                print(f)
        return 0
    if args.cmd == "mutants":
        import perimeter_exports as px  # noqa: PLC0415
        text = (root / px.TABLE).read_text()
        table, _, _ = px.parse_table(text)
        outcomes = [o for p in args.outcomes for o in json.loads(p.read_text())["outcomes"]]
        findings, missed = px.mutant_findings(root, outcomes, table, px.table_counts(text)[1],
                                              args.crate if args.require_caught else None)
        for (f, item), descs in sorted(missed.items()):
            print(f"  missed {len(descs):3d}  {f}  {item}")
        total = sum(1 for o in outcomes if o.get("summary") == "MissedMutant")
        return report("mutants", findings, f" outcomes={len(outcomes)} missed={total}")
    if args.cmd == "k4":
        return report("k4", run_k4(root, cfg, args.json_dir or rustdoc_dir()))
    if args.log is None:
        ap.error(f"{args.cmd} requires --log")
    recs = read_log(args.log)
    if args.cmd == "location":
        findings, st = location_findings(recs, cfg, args.frozen_ok)
        return report("location", findings, f" units={st['units']} diagnostics={st['diagnostics']} "
                                            f"outside={st['outside']} exempt={st['exempt']}")
    if args.cmd == "reached":
        passes = {}
        for p in cfg.location_passes():
            meta = cargo_metadata(root, p["manifest"] if p["manifest"] != "Cargo.toml" else None)
            passes[p["name"]] = (meta, p["args"])
        findings, total = reached_findings(recs, passes, root)
        return report("reached", findings, f" units={total} passes={len(passes)}")
    if args.cmd == "depinfo":
        tracked = {f for f in subprocess.run(["git", "ls-files", "-z", "--", "*.rs"], cwd=root, capture_output=True,
                                             check=True).stdout.decode().split("\0") if f}
        return report("depinfo", depinfo_findings(recs, root, cfg, tracked))
    return 2


if __name__ == "__main__":
    sys.exit(main())
