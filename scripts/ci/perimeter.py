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


def report(name: str, findings: list[Finding], extra: str = "") -> int:
    for f in findings:
        print(f)
    print(f"PERIMETER_{name.upper()} findings={len(findings)}{extra}")
    return 1 if findings else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=("lex", "manifest", "metadata"))
    args = ap.parse_args(argv)
    cfg = Config.load(ROOT)
    if args.cmd == "lex":
        findings, tree = run_lex(ROOT, cfg)
        return report("lex", findings, f" files={len(tree.files)}")
    if args.cmd == "manifest":
        return report("manifest", run_manifest(ROOT, cfg))
    if args.cmd == "metadata":
        return report("metadata", run_metadata(ROOT, cfg))
    return 2


if __name__ == "__main__":
    sys.exit(main())
