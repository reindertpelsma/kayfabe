#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""G1, G1b, G1c — no raw or disguised address outside the memory-safety perimeter.

`v3-sec-rawaddr` (2026-10-04; audit S1-02, S1-03, S1-12; OWNER_RULINGS §R gate 2;
docs/design/V3_RAWADDR_PERIMETER.md §6).

The perimeter is every file named `*_unsafe.rs`. OUTSIDE it, no code may hold, build, pass or print a
host or device address — not as a pointer type (the old host-pointer gate's job, which sees only
`*mut`/`*const`/`NonNull`/`transmute`), and not DISGUISED: `&raw const`, `ptr::from_ref`, `.as_ptr()`,
`Box::into_raw`, `.addr()`, an `addr: usize` field, `{:p}`, a CUDA device-pointer type...

* G1  — the lexical gate over `crates/kf-*/**/*.rs` and `firmware/**/*.rs`, minus `*_unsafe.rs`,
        trybuild fixtures (`*/tests/ui/*`) and `target/`. A real lexer drops comments, char and string
        literals (P7 runs on string contents only) and `PhantomData<…>` (a ZST carries no address).
* G1b — inside the perimeter of kf-linux-raw, kf-cuda and kf-qemu: no derived `Debug` on a type
        that holds a pointer, or a `usize` named `raw`/`ptr`/`host`/`addr` (`{:?}` would print it).
* G1c — inside the perimeter: no `pub use` re-exporting a pointer function or type (G1 skips those
        files, so a re-export under a harmless name would otherwise walk an address out).
* G1  also refuses `allow(clippy::disallowed_…)` outside the perimeter: that is G1d's opt-out.

Exit status 1 on any hit, after a self-test against known positives and look-alikes
(`scripts/ci/fixtures/address_gate/`): a gate that reports zero must first report one.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "scripts/ci/fixtures/address_gate"
PERIMETER_CRATES = ("kf-linux-raw", "kf-cuda", "kf-qemu")
MIN_KF_CRATES = 18  # the floor scripts/ci/dependencies.py uses

# ── the lexer ───────────────────────────────────────────────────────────────────────────────

def lex(src: str) -> tuple[list[str], list[str]]:
    """Code with comments, char literals and string literals blanked (newlines kept, so line
    numbers survive), and per line the concatenated CONTENTS of the string literals on it."""
    out: list[str] = []
    strings: dict[int, list[str]] = {}
    i, n, line = 0, len(src), 0
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c == "\n":
            out.append(c)
            line += 1
            i += 1
        elif c == "/" and nxt == "/":
            while i < n and src[i] != "\n":
                i += 1
        elif c == "/" and nxt == "*":
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    if src[i] == "\n":
                        out.append("\n")
                        line += 1
                    i += 1
        elif (c == "r" or (c == "b" and nxt == "r")) and re.match(r'b?r(#*)"', src[i:]) and (
            i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")
        ):
            m = re.match(r'b?r(#*)"', src[i:])
            hashes = m.group(1)
            i += m.end()
            end = src.find('"' + hashes, i)
            end = n if end < 0 else end
            body = src[i:end]
            strings.setdefault(line, []).append(body)
            for ch in body:
                if ch == "\n":
                    out.append("\n")
                    line += 1
            out.append('""')
            i = end + 1 + len(hashes)
        elif c == '"' or (c == "b" and nxt == '"' and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_"))):
            i += 1 if c == '"' else 2
            start_line, body = line, []
            while i < n and src[i] != '"':
                if src[i] == "\\" and i + 1 < n:
                    body.append(src[i : i + 2])
                    i += 2
                    continue
                if src[i] == "\n":
                    out.append("\n")
                    line += 1
                body.append(src[i])
                i += 1
            strings.setdefault(start_line, []).append("".join(body))
            out.append('""')
            i += 1
        elif c == "'":
            # A char literal ('x', '\n', '\u{..}', '\'') or a lifetime ('a, 'static): a literal
            # closes within a few characters, a lifetime never closes.
            m = re.match(r"'(\\u\{[0-9a-fA-F]+\}|\\.|[^\\'\n])'", src[i:])
            if m:
                out.append("' '")
                i += m.end()
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    code = "".join(out).split("\n")
    lines_strings = ["\n".join(strings.get(k, [])) for k in range(len(code))]
    return code, lines_strings


def drop_phantom(code: str) -> str:
    """Erase `PhantomData<…>` (balanced): a zero-sized marker cannot carry an address."""
    out, i = [], 0
    while True:
        j = code.find("PhantomData<", i)
        if j < 0:
            out.append(code[i:])
            return "".join(out)
        out.append(code[i:j])
        k, depth = j + len("PhantomData<"), 1
        while k < len(code) and depth:
            depth += {"<": 1, ">": -1}.get(code[k], 0)
            k += 1
        out.append("PhantomData")
        i = k


# ── G1: disguised addresses outside the perimeter ───────────────────────────────────────────

CODE_PATTERNS = [
    ("P0", r"\*\s*(const|mut)\b"),
    ("P1", r"&raw\s+(const|mut)\b"),
    ("P2", r"\bptr::(from_ref|from_mut|null|null_mut|dangling|dangling_mut|without_provenance(_mut)?"
           r"|with_exposed_provenance(_mut)?|addr_of(_mut)?|slice_from_raw_parts(_mut)?|hash)\b"),
    ("P2", r"\bptr\s+as\s+\w+"),
    # a renamed `ptr` module (`use core::{ptr as q}; q::from_ref(..)`): the call site too. The
    # slice/array/Cell functions of the same names make no pointer.
    ("P2", r"(?<!slice::)(?<!array::)(?<!Cell::)\b(from_ref|from_mut)\s*\("),
    ("P2", r"use\s+(core|std)::ptr::\*"),
    ("P3", r"\baddr_of(_mut)?!"),
    ("P4", r"\b(as_ptr|as_mut_ptr|as_ptr_range|as_mut_ptr_range|into_raw|into_raw_parts"
           r"|expose_provenance|with_addr|map_addr)\b"),
    ("P5", r"\.addr\s*\(\s*\)"),
    ("P5", r"::addr\b(?!\s*(::|\{))"),
    ("P6", r"\b(AtomicPtr|UnsafeCell|SyncUnsafeCell|CUdeviceptr|DevAddr)\b"),
    ("P6", r"\bfmt::Pointer\b|\bPointer::fmt\b|use[^;]*\bPointer\b"),
    ("P9", r"\b(addr|addrs|ptr|hva|dptr|devptr|dev_ptr|host_addr|host_ptr|dev_addr|base_addr)\s*:\s*\[?\s*"
           r"(usize|AtomicUsize|NonZeroUsize)\b"),
    ("P9", r"\b(ptr|dptr|devptr|dev_ptr|dev_addr)\s*:\s*\[?\s*(u64|AtomicU64|NonZeroU64)\b"),
    ("P9", r"\bfn\s+\w*(addr|ptr)\s*(<[^>]*>)?\s*\([^)]*\)\s*->\s*(Option<\s*)?usize\b"),
    ("G1d-optout", r"\b(allow|expect)\s*\(\s*clippy::disallowed_"),
]
STRING_PATTERNS = [("P7", r"\{[^{}]*:[^{}]*p\}")]
CODE_RES = [(k, re.compile(p)) for k, p in CODE_PATTERNS]
STRING_RES = [(k, re.compile(p)) for k, p in STRING_PATTERNS]


def g1_hits(src: str) -> list[tuple[int, str, str]]:
    """(line, pattern, text) per hit; one per (line, pattern)."""
    code, strings = lex(src)
    hits = []
    for no, (c, s) in enumerate(zip(code, strings), start=1):
        c = drop_phantom(c)
        for k, r in CODE_RES:
            if r.search(c):
                hits.append((no, k, c.strip()))
        for k, r in STRING_RES:
            if s and r.search(s):
                hits.append((no, k, s.strip()))
    return hits


# ── G1b: no pointer-printing Debug inside the perimeter ─────────────────────────────────────

POINTER_FIELD = re.compile(r"NonNull\s*<|\*\s*(mut|const)\b|\b(raw|ptr|host|addr)\s*:\s*usize\b")


def _item_body(code: list[str], start: int) -> tuple[int, str]:
    """From the line after an attribute, the item header line and its body text (balanced
    braces, or a tuple struct's parentheses up to `;`)."""
    i = start
    while i < len(code) and (not code[i].strip() or code[i].strip().startswith("#[")):
        i += 1
    if i >= len(code):
        return i, ""
    text = "\n".join(code[i:])
    head = re.match(r"\s*(pub(\([^)]*\))?\s+)?(struct|enum)\s+\w+", text)
    if not head:
        return i, ""
    # the body opens at the first `{` or `(` after the item's name (generics have neither)
    m = re.compile(r"[{(;]").search(text, head.end())
    if not m or m.group(0) == ";":
        return i, ""
    pairs = {"{": "}", "(": ")"}
    o, cl, depth, k = m.group(0), pairs[m.group(0)], 0, m.start()
    while k < len(text):
        depth += 1 if text[k] == o else -1 if text[k] == cl else 0
        k += 1
        if depth == 0:
            break
    return i, text[m.start() : k]


def g1b_hits(src: str) -> list[tuple[int, str, str]]:
    code, _ = lex(src)
    hits = []
    for no, line in enumerate(code):
        if re.match(r"\s*#\[derive\([^)]*\bDebug\b", line):
            at, body = _item_body(code, no + 1)
            if body and POINTER_FIELD.search(body):
                hits.append((at + 1, "G1b", code[at].strip() if at < len(code) else ""))
    return hits


# ── G1c: no pointer re-exports from the perimeter ───────────────────────────────────────────

REEXPORT = re.compile(r"\bpub(\([^)]*\))?\s+use\s+[^;]*\b(ptr|NonNull|addr_of|AtomicPtr|UnsafeCell)\b")


def g1c_hits(src: str) -> list[tuple[int, str, str]]:
    code, _ = lex(src)
    joined = "\n".join(code)
    hits = []
    for m in REEXPORT.finditer(joined):
        no = joined.count("\n", 0, m.start()) + 1
        hits.append((no, "G1c", m.group(0).strip()))
    return hits


# ── the scan ────────────────────────────────────────────────────────────────────────────────

def g1_files() -> list[Path]:
    files = []
    for base in [*sorted((ROOT / "crates").glob("kf-*")), ROOT / "firmware"]:
        for p in base.rglob("*.rs"):
            rel = p.relative_to(ROOT).as_posix()
            if p.name.endswith("_unsafe.rs") or "/tests/ui/" in rel or "/target/" in f"/{rel}":
                continue
            files.append(p)
    return files


def perimeter_files() -> list[Path]:
    out = []
    for c in PERIMETER_CRATES:
        out += sorted((ROOT / "crates" / c / "src").rglob("*_unsafe.rs"))
    return out


def scan() -> list[str]:
    report = []
    crates = {p.relative_to(ROOT).parts[1] for p in g1_files() if p.relative_to(ROOT).parts[0] == "crates"}
    if len(crates) < MIN_KF_CRATES:
        report.append(f"G1 scanned only {len(crates)} kf-* crates (floor {MIN_KF_CRATES}): the scan is blind")
    for p in g1_files():
        for no, k, t in g1_hits(p.read_text()):
            report.append(f"{p.relative_to(ROOT)}:{no}: {k}: {t}")
    perim = perimeter_files()
    if len(perim) < 3:
        report.append(f"G1b/G1c found only {len(perim)} perimeter files: the scan is blind")
    for p in perim:
        src = p.read_text()
        for no, k, t in g1b_hits(src) + g1c_hits(src):
            report.append(f"{p.relative_to(ROOT)}:{no}: {k}: {t}")
    return report


def code_lines(src: str) -> list[int]:
    """1-based numbers of lines that hold code (not blank, not a `//` comment)."""
    return [i for i, l in enumerate(src.splitlines(), start=1) if l.strip() and not l.strip().startswith("//")]


def self_test() -> list[str]:
    """Every code line of each POSITIVE fixture is a hit; the look-alikes are none."""
    fails = []
    for fixture, finder in [
        ("positive.rs.txt", g1_hits),
        ("perimeter_debug.rs.txt", g1b_hits),
        ("perimeter_reexport.rs.txt", g1c_hits),
    ]:
        src = (FIXTURES / fixture).read_text()
        hit = {no for no, _, _ in finder(src)}
        if fixture != "positive.rs.txt":
            # the item header (or the `use`) a hit is reported at carries a `// HIT` marker
            want = {i for i, l in enumerate(src.splitlines(), start=1) if "// HIT" in l}
        else:
            want = set(code_lines(src))
        if not want:
            fails.append(f"{fixture}: no known positive to find")
        for no in sorted(want - hit):
            fails.append(f"{fixture}:{no}: a known positive the gate MISSED: {src.splitlines()[no - 1].strip()}")
    for fixture, finder in [
        ("negative.rs.txt", g1_hits),
        ("perimeter_debug_negative.rs.txt", g1b_hits),
        ("perimeter_debug_negative.rs.txt", g1c_hits),
    ]:
        for no, k, t in finder((FIXTURES / fixture).read_text()):
            fails.append(f"{fixture}:{no}: a look-alike the gate FLAGGED ({k}): {t}")
    return fails


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--self-test", action="store_true", help="run only the fixture self-test")
    args = ap.parse_args()
    fails = self_test()
    for f in fails:
        print(f"SELF-TEST: {f}")
    if fails:
        print("★ ADDRESS GATE SELF-TEST FAILED — the gate cannot be trusted to report zero.")
        return 1
    print("address gate self-test: every known positive found, no look-alike flagged")
    if args.self_test:
        return 0
    report = scan()
    for r in report:
        print(r)
    print(f"ADDRESS_GATE files={len(g1_files())} perimeter={len(perimeter_files())} offenders={len(report)}")
    if report:
        print(
            "\n★ RAW-ADDRESS BREACH — OWNER_RULINGS §R gate 2, THE_CONSTRAINTS §13.\n"
            "A host or device address (or a disguise for one) appears outside the perimeter\n"
            "(`*_unsafe.rs`), or the perimeter would print or re-export one. Fixes, no other:\n"
            "  - hold an opaque handle (HostSpan / StaticSpan / a kf-cuda handle) instead of the\n"
            "    address, and move the code that needs the address into the perimeter, behind a\n"
            "    method that validates its own inputs;\n"
            "  - a derived Debug over a pointer field: write it by hand (length only);\n"
            "  - a census key or identity built from an address: key it by content (kf-util textkey).\n"
            "Never add an allowlist: the only exceptions are lexical (comments, strings, PhantomData)."
        )
    return bool(report)


if __name__ == "__main__":
    raise SystemExit(main())
