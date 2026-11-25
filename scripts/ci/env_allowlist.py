#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Environment-variable gate for the product sources (STATUS: LIVE, 2026-11-20).

Rule (owner, 2026-11-20): no environment variable is required to boot a Windows or Linux guest at full performance in
production. Debug and measurement variables may exist, but they are secondary: the product path reads the environment
only inside an allowlist. This gate makes that checkable.

What it scans (`SCAN`): the C QOM device (`qemu/hw/misc/kf3`), the C display broker (`broker`), the walk kernel
(`cuda/walk`), the boot-display firmware (`firmware`) and every `crates/kf-*` crate. A read is `std::env::var`,
`var_os`, `vars`, `vars_os`, `env!`, `option_env!` (Rust) or `getenv`, `secure_getenv` (C), found on source text with
comments and string contents masked, so a name in a comment or a message is not a read.

What it leaves out, and why (`skipped`): `tests/`, `benches/` and `examples/` directories and files whose stem is a test
(`tests.rs`, `x_tests.rs`, `nb_session_test.c`, `kf_tests.cu`) and `#[cfg(test)]` items are not shipped code. The frozen
`crates/kayfabe-*` and `archive/` are not v3 (CLAUDE.md, Layout) and nothing may be added to them; `third_party/` is not ours.

The registry `scripts/ci/env_registry.json` names every variable that is read: its class, the file that reads it, why, and
whether it changes what the guest is served. The gate FAILS on
  UNREGISTERED   a read whose name is not in the registry (a read with a computed name is `<dynamic:file:fn>`);
  STALE_ENTRY    a registry entry that no scanned source reads any more (delete the entry);
  FILE_MISMATCH  the entry's `file` does not read the name (move the entry's `file`);
  BAD_ENTRY      an entry with an unknown class, a one-word `why`, no boolean `changes_guest_served`, or an offender
                 without a `proposal`;
  NEW_OFFENDER   an entry of class `product` or with `changes_guest_served: true` that is on neither
                 `product_env_allowlist` (the registry) nor the baseline;
  STALE_BASELINE a baseline name that is no longer an offender (delete the line: the ratchet only goes down);
  BAD_ALLOWLIST  an allowlisted name that the registry does not have.
`scripts/ci/env-allowlist-debt.json` is the ratchet: the offenders that existed when the gate landed. It can only shrink.
`--record-baseline` creates it when it is absent and afterwards only removes names; a finding is fixed, never recorded.

    python3 scripts/ci/env_allowlist.py            # the gate
    python3 scripts/ci/env_allowlist.py report     # markdown: counts per class and the guest-affecting table
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = "scripts/ci/env_registry.json"
BASELINE = "scripts/ci/env-allowlist-debt.json"

SCAN = ("qemu/hw/misc/kf3", "broker", "cuda/walk", "firmware", "crates")
SOURCE_SUFFIXES = (".rs", ".c", ".h", ".cu", ".cuh")
SKIP_DIRS = {"tests", "benches", "examples", "target", "node_modules", ".git"}
CLASSES = ("debug", "measurement", "inject", "stress", "system", "product")
PROPOSALS = ("device-property", "keep-as-debug", "delete")
TEST_STEM = re.compile(r"(^|[_-])tests?($|[_-])")

RUST_CALL = re.compile(r"(?<![\w:])(?:std::)?env::(vars_os|vars|var_os|var)\b\s*(\()?|(?<![\w])(option_env|env)!\s*\(")
C_CALL = re.compile(r"(?<![\w])(?:secure_getenv|getenv)\s*\(")
CFG_TEST = re.compile(r"#\s*\[\s*cfg\s*\(\s*(?:test|all\s*\(\s*test\b[^\]]*)\)\s*\]")
FN_DEF = re.compile(r"\bfn\s+(\w+)")
ENV_LITERAL = re.compile(r"[A-Z][A-Z0-9_]{3,}")


def mask(text: str, lang: str) -> tuple[str, str]:
    """Return (code, skel), both the length of `text` and with the same newlines.

    `code` has comments blanked; `skel` has comments blanked and the inside of string and char literals blanked as well,
    so braces and call names are found on `skel` while the literal is read from `code` at the same offset."""
    n = len(text)
    code, skel = list(text), list(text)

    def blank(arr, a, b):
        for k in range(a, b):
            if arr[k] != "\n":
                arr[k] = " "

    i = 0
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(code, i, j)
            blank(skel, i, j)
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
            blank(code, i, j)
            blank(skel, i, j)
            i = j
        elif lang == "rust" and c in "rb" and re.match(r"b?r#*\"", text[i:i + 12]) and not (i and (text[i - 1].isalnum() or text[i - 1] == "_")):
            m = re.match(r"b?r(#*)\"", text[i:i + 12])
            close = '"' + m.group(1)
            j = text.find(close, i + m.end())
            j = n if j < 0 else j + len(close)
            blank(skel, i + m.end(), j - len(close))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            blank(skel, i + 1, min(j, n))
            i = j + 1
        elif c == "'":
            # a char literal; in Rust `'a` without a closing quote is a lifetime
            if text[i + 1:i + 2] == "\\":
                j = text.find("'", i + 3)
                j = i + 1 if j < 0 else j
                blank(skel, i + 1, j)
                i = j + 1
            elif text[i + 2:i + 3] == "'":
                blank(skel, i + 1, i + 2)
                i += 3
            else:
                i += 1
        else:
            i += 1
    return "".join(code), "".join(skel)


def match_brace(skel: str, open_at: int) -> int:
    depth = 0
    for k in range(open_at, len(skel)):
        if skel[k] == "{":
            depth += 1
        elif skel[k] == "}":
            depth -= 1
            if depth == 0:
                return k + 1
    return len(skel)


def cfg_test_spans(skel: str) -> tuple[list[tuple[int, int]], list[str]]:
    """Spans of `#[cfg(test)]` items, and the names of `#[cfg(test)] mod x;` files."""
    spans, files = [], []
    for m in CFG_TEST.finditer(skel):
        k = m.end()
        while True:  # further attributes of the same item
            a = re.compile(r"\s*#\s*\[[^\]]*\]").match(skel, k)
            if not a:
                break
            k = a.end()
        stop = re.compile(r"[{;]").search(skel, k)
        if not stop:
            continue
        mod = re.match(r"\s*(?:pub(?:\([a-z: ]+\))?\s+)?mod\s+(\w+)\s*;", skel[k:stop.end()])
        if mod:
            files.append(mod.group(1))
        end = match_brace(skel, stop.start()) if stop.group() == "{" else stop.end()
        spans.append((m.start(), end))
    return spans, files


def blank_spans(s: str, spans: list[tuple[int, int]]) -> str:
    out = list(s)
    for a, b in spans:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "
    return "".join(out)


def product_files(root: Path) -> list[Path]:
    found = []
    for top in SCAN:
        base = root / top
        for dirpath, dirnames, filenames in os.walk(base):
            rel = Path(dirpath).relative_to(root)
            if top == "crates":
                if rel == Path("crates"):
                    dirnames[:] = sorted(d for d in dirnames if d.startswith("kf-"))
                    continue
            dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
            for f in sorted(filenames):
                p = Path(dirpath) / f
                if p.suffix in SOURCE_SUFFIXES and not TEST_STEM.search(p.stem):
                    found.append(p)
    return found


def lang_of(p: Path) -> str:
    return "rust" if p.suffix == ".rs" else "c"


def literal_at(code: str, pos: int) -> tuple[str | None, str]:
    """The first argument at `pos` (just after the `(`): ('NAME', 'literal') or (None, 'ident'/'other')."""
    m = re.compile(r'\s*b?"((?:[^"\\]|\\.)*)"').match(code, pos)
    if m:
        return m.group(1), "literal"
    m = re.compile(r"\s*([A-Za-z_][\w:]*)").match(code, pos)
    if m and not code.startswith("concat", m.start(1)):
        return m.group(1), "ident"
    return None, "other"


def scan_file(root: Path, p: Path, consts: dict[str, str]) -> tuple[list[tuple[str, int, str]], list[str]]:
    """Reads of one file: [(name, line, how)], plus the `#[cfg(test)] mod x;` names it declares."""
    text = p.read_text(errors="replace")
    lang = lang_of(p)
    code, skel = mask(text, lang)
    declared: list[str] = []
    if lang == "rust":
        spans, declared = cfg_test_spans(skel)
        code, skel = blank_spans(code, spans), blank_spans(skel, spans)
    rel = str(p.relative_to(root))
    own = {m.group(1): m.group(2) for m in CONST_STR.finditer(text)} if lang == "rust" else {}
    pat = RUST_CALL if lang == "rust" else C_CALL
    reads = []
    for m in pat.finditer(skel):
        line = skel.count("\n", 0, m.start()) + 1
        if lang == "rust":
            kind = m.group(1) or m.group(3)
            if kind in ("vars", "vars_os"):
                reads.append(("<env:vars>", line, "enumerate"))
                continue
            if m.group(1) and not m.group(2):
                continue  # `env::var` as a path without a call (a `use`)
            pos = m.end()
        else:
            pos = m.end()
        arg, how = literal_at(code, pos)
        if how == "literal":
            reads.append((arg, line, "literal"))
            continue
        if how == "ident" and arg in own:
            # the file's own const wins: a name defined in several files is not a global
            reads.append((own[arg], line, "const"))
            continue
        if how == "ident" and arg in consts or (how == "ident" and arg.split("::")[-1] in consts):
            reads.append((consts.get(arg, consts.get(arg.split("::")[-1])), line, "const"))
            continue
        # a computed name: the ALL-CAPS literals of the enclosing function, else a dynamic read
        names, fn = [], "?"
        if lang == "rust":
            fns = list(FN_DEF.finditer(skel, 0, m.start()))
            if fns:
                fn = fns[-1].group(1)
                body = skel.find("{", fns[-1].end())
                end = match_brace(skel, body) if body >= 0 else m.start()
                if end > m.start():
                    names = [s for s in re.findall(r'"([A-Z][A-Z0-9_]{3,})"', code[fns[-1].start():end])]
        if names:
            reads.extend((nm, line, "list") for nm in sorted(set(names)))
        elif lookups(code):
            # `parse(|k| std::env::var(k))`: the lookup is injected and the names are the `get("KF3_..")` calls of the file
            reads.extend((nm, line, "lookup") for nm in lookups(code))
        else:
            reads.append((f"<dynamic:{rel}:{fn}>", line, "dynamic"))
    return reads, declared


def lookups(code: str) -> list[str]:
    return sorted(set(re.findall(r'\bget\(\s*"((?:KF3|KF|KAYFABE)_[A-Z0-9_]+)"', code)))


CONST_STR = re.compile(r'\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:\'static\s+)?str\s*=\s*"([^"\\]*)"')


def const_table(files: list[Path]) -> dict[str, str]:
    table: dict[str, str] = {}
    for p in files:
        if p.suffix == ".rs":
            for m in CONST_STR.finditer(p.read_text(errors="replace")):
                table[m.group(1)] = m.group(2)
    return table


def scan(root: Path = ROOT) -> dict[str, list[tuple[str, int, str]]]:
    """name -> [(file, line, how)] over every product source."""
    files = product_files(root)
    consts = const_table(files)
    # files that are `#[cfg(test)] mod x;` of a scanned file are test code: their reads are not product reads
    test_files: set[Path] = set()
    parsed = {}
    for p in files:
        reads, declared = scan_file(root, p, consts)
        parsed[p] = reads
        base = p.parent if p.name in ("lib.rs", "mod.rs", "main.rs") else p.parent / p.stem
        for name in declared:
            test_files |= {base / f"{name}.rs", base / name / "mod.rs"}
            if (base / name).is_dir():
                test_files |= set((base / name).rglob("*.rs"))
    found: dict[str, list[tuple[str, int, str]]] = {}
    for p, reads in parsed.items():
        if p in test_files:
            continue
        for name, line, how in reads:
            found.setdefault(name, []).append((str(p.relative_to(root)), line, how))
    return found


def load(root: Path, rel: str, default):
    p = root / rel
    return json.loads(p.read_text()) if p.exists() else default


def is_offender(entry: dict) -> bool:
    return entry.get("class") == "product" or entry.get("changes_guest_served") is True


def check_entry(e: dict) -> str | None:
    if not isinstance(e, dict) or not isinstance(e.get("name"), str):
        return "an entry needs a `name`"
    if e.get("class") not in CLASSES:
        return "class must be one of %s" % (CLASSES,)
    if not isinstance(e.get("changes_guest_served"), bool):
        return "changes_guest_served must be true or false"
    why = e.get("why")
    if not isinstance(why, str) or "\n" in why or len(why.split()) < 3:
        return "`why` is one line of at least three words"
    if not isinstance(e.get("file"), str) or not e["file"]:
        return "`file` is the path that reads it"
    if is_offender(e) and e.get("proposal") not in PROPOSALS:
        return "an offender needs a `proposal`, one of %s" % (PROPOSALS,)
    return None


def findings(root: Path = ROOT) -> list[tuple[str, str, str]]:
    reads = scan(root)
    reg = load(root, REGISTRY, None)
    if reg is None:
        return [("NO_REGISTRY", REGISTRY, "the registry is missing")]
    out: list[tuple[str, str, str]] = []
    entries: dict[str, dict] = {}
    invalid: set[str] = set()  # a bad entry is reported once, not again as unregistered
    for e in reg.get("entries", []):
        bad = check_entry(e)
        if bad:
            out.append(("BAD_ENTRY", e.get("name", "?") if isinstance(e, dict) else "?", bad))
            if isinstance(e, dict) and isinstance(e.get("name"), str):
                invalid.add(e["name"])
            continue
        if e["name"] in entries:
            out.append(("BAD_ENTRY", e["name"], "listed twice"))
        entries[e["name"]] = e
    for name, sites in sorted(reads.items()):
        if name not in entries and name not in invalid:
            f, l, _ = sites[0]
            out.append(("UNREGISTERED", name, "read at %s:%d; add it to %s with a class, a why and changes_guest_served" % (f, l, REGISTRY)))
    for name, e in sorted(entries.items()):
        if name not in reads:
            out.append(("STALE_ENTRY", name, "no product source reads it any more; delete the entry from %s" % REGISTRY))
        elif e["file"] not in {f for f, _, _ in reads[name]}:
            out.append(("FILE_MISMATCH", name, "%s does not read it (read in %s)" % (e["file"], sorted({f for f, _, _ in reads[name]})[0])))
    allow = reg.get("product_env_allowlist", [])
    for name in allow:
        if name not in entries and name not in invalid:
            out.append(("BAD_ALLOWLIST", name, "the allowlist names a variable the registry does not have"))
    base = set(load(root, BASELINE, {}).get("offenders", []))
    offenders = {n for n, e in entries.items() if is_offender(e) and n not in allow}
    base_known = base - invalid
    for name in sorted(offenders - base):
        e = entries[name]
        why = "class product" if e["class"] == "product" else "changes what the guest is served"
        out.append(("NEW_OFFENDER", name, "%s and is not on product_env_allowlist; make it a device property or take it out of the product path" % why))
    for name in sorted(base_known - offenders):
        out.append(("STALE_BASELINE", name, "no longer an offender; delete it from %s (the ratchet only goes down)" % BASELINE))
    return out


def report(root: Path = ROOT) -> str:
    reads = scan(root)
    reg = load(root, REGISTRY, {"entries": []})
    entries = reg["entries"]
    lines = ["| class | names | change what the guest is served |", "|---|---|---|"]
    for cls in CLASSES:
        rows = [e for e in entries if e["class"] == cls]
        lines.append("| %s | %d | %d |" % (cls, len(rows), sum(1 for e in rows if e["changes_guest_served"])))
    lines.append("| **total** | %d | %d |" % (len(entries), sum(1 for e in entries if e["changes_guest_served"])))
    lines += ["", "| name | class | first read | proposal | why |", "|---|---|---|---|---|"]
    for e in sorted((e for e in entries if is_offender(e)), key=lambda e: (e["file"], e["name"])):
        sites = reads.get(e["name"], [("?", 0, "")])
        f, l, _ = sorted(s for s in sites if s[0] == e["file"])[0] if any(s[0] == e["file"] for s in sites) else sites[0]
        lines.append("| `%s` | %s | `%s:%d` | %s | %s |" % (e["name"], e["class"], f, l, e.get("proposal", "?"), e["why"]))
    return "\n".join(lines)


def record_baseline(root: Path = ROOT) -> int:
    """Create the baseline when it is absent; afterwards only remove names (it never adds one)."""
    reg = load(root, REGISTRY, {"entries": []})
    allow = set(reg.get("product_env_allowlist", []))
    offenders = {e["name"] for e in reg["entries"] if is_offender(e) and e["name"] not in allow}
    path = root / BASELINE
    if path.exists():
        offenders &= set(json.loads(path.read_text()).get("offenders", []))
    path.write_text(json.dumps({"offenders": sorted(offenders)}, indent=2) + "\n")
    return len(offenders)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", nargs="?", default="check", choices=("check", "report"))
    ap.add_argument("--record-baseline", action="store_true", help="create the baseline, or shrink it; never adds a name")
    ap.add_argument("--root", type=Path, default=ROOT)
    a = ap.parse_args(argv)
    if a.record_baseline:
        print("baseline: %d offenders in %s" % (record_baseline(a.root), BASELINE))
        return 0
    if a.mode == "report":
        print(report(a.root))
        return 0
    found = findings(a.root)
    for kind, name, detail in found:
        print("%s %s: %s" % (kind, name, detail))
    reads = scan(a.root)
    reg = load(a.root, REGISTRY, {"entries": []})
    print("env_allowlist: reads=%d names registered=%d offenders-on-baseline=%d findings=%d" % (
        len(reads), len(reg["entries"]), len(load(a.root, BASELINE, {}).get("offenders", [])), len(found)))
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
