#!/usr/bin/env python3
"""Generate and check the device-property table of the command-line reference (docs/howto/cli-reference.md).

STATUS: LIVE, 2026-11-20 (design: docs/design/V3_PACKAGING_AND_DISTRIBUTION.md section 2.3 item 5).

The property table of the manual is GENERATED from the `kf3_properties` table of qemu/hw/misc/kf3/kf3.c: the name, the
type and the default come from the C source (an expression such as `256 * MiB` or a `#define` is evaluated), the status
and the one-line meaning come from scripts/ci/doc_cli_reference_text.json. `check` FAILS when

  * a property of kf3.c has no row on the page, or the page has a row the source does not declare (missing property),
  * a row's default differs from the source's (wrong default),
  * the generated block of the page is not byte-for-byte what `generate` prints now (stale table),
  * the text file lacks a property or names one that the source does not declare.

    python3 scripts/ci/doc_cli_reference.py check          # exit 0 / 1, one line per finding
    python3 scripts/ci/doc_cli_reference.py write          # rewrite the generated block of the page in place
    python3 scripts/ci/doc_cli_reference.py print          # print the generated block

The page keeps hand-written text around the block; only the text between the two marker lines is generated.
Standard library only.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "qemu/hw/misc/kf3/kf3.c"
HEADERS = ("qemu/hw/misc/kf3/kf3.h", "qemu/hw/misc/kf3/kf3.c")
TEXT = "scripts/ci/doc_cli_reference_text.json"
PAGE = "docs/howto/cli-reference.md"
BEGIN = "<!-- kf-generated: device-properties begin (scripts/ci/doc_cli_reference.py; do not edit by hand) -->"
END = "<!-- kf-generated: device-properties end -->"
MIB = 1024 * 1024
TYPES = {"UINT32": "uint32", "UINT64": "uint64", "INT64": "int64", "BOOL": "bool", "STRING": "string", "ON_OFF_AUTO": "on/off/auto"}
ON_OFF_AUTO = {"ON_OFF_AUTO_AUTO": "auto", "ON_OFF_AUTO_ON": "on", "ON_OFF_AUTO_OFF": "off"}
MACRO = re.compile(r"DEFINE_PROP_([A-Z0-9_]+)\(")


class RefError(Exception):
    pass


def strip_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def defines(root: Path) -> dict[str, str]:
    found = {}
    for rel in HEADERS:
        path = root / rel
        if path.is_file():
            for m in re.finditer(r"^#define\s+([A-Z][A-Z0-9_]*)\s+(.+?)\s*$", strip_comments(path.read_text()), re.M):
                found.setdefault(m[1], m[2])
    return found


def evaluate(expr: str, names: dict[str, str], depth: int = 0) -> int:
    """Integers, `-N`, `A * B`, MiB and #define names; nothing else (no eval)."""
    if depth > 8:
        raise RefError("define loop in " + expr)
    value = 1
    for factor in expr.split("*"):
        factor = factor.strip()
        if re.fullmatch(r"-?(0[xX][0-9a-fA-F]+|\d+)[uUlL]*", factor):
            value *= int(re.sub(r"[uUlL]+$", "", factor), 0)
        elif factor == "MiB":
            value *= MIB
        elif factor in names:
            value *= evaluate(names[factor], names, depth + 1)
        else:
            raise RefError("cannot evaluate default %r" % expr)
    return value


def split_args(text: str) -> list[str]:
    args, depth, current, quoted = [], 0, "", False
    for ch in text:
        if ch == '"':
            quoted = not quoted
        if not quoted and ch == "(":
            depth += 1
        if not quoted and ch == ")":
            depth -= 1
        if not quoted and depth == 0 and ch == ",":
            args.append(current.strip())
            current = ""
        else:
            current += ch
    if current.strip():
        args.append(current.strip())
    return args


def parse_properties(root: Path = ROOT) -> list[dict]:
    """Every DEFINE_PROP_* of the kf3_properties table, in source order: name, type, default (raw string)."""
    source = strip_comments((root / SOURCE).read_text())
    start = re.search(r"kf3_properties\s*\[\s*\]\s*=\s*\{", source)
    if not start:
        raise RefError("no kf3_properties table in " + SOURCE)
    end = source.index("};", start.end())
    body = source[start.end():end]
    names = defines(root)
    rows = []
    for m in MACRO.finditer(body):
        kind = m[1]
        if kind == "END_OF_LIST":
            continue
        depth, i = 1, m.end()
        while depth and i < len(body):
            depth += {"(": 1, ")": -1}.get(body[i], 0)
            i += 1
        args = split_args(body[m.end():i - 1])
        if kind not in TYPES or len(args) < 3 or not re.fullmatch(r'"[^"]+"', args[0]):
            raise RefError("unrecognised property declaration: " + m[0] + " ... " + ", ".join(args[:2]))
        row = {"name": args[0].strip('"'), "type": TYPES[kind], "default": None}
        if kind == "STRING":
            row["default_text"] = "unset"
        elif kind == "BOOL":
            row["default_text"] = {"true": "on", "false": "off"}[args[3]]
        elif kind == "ON_OFF_AUTO":
            if args[3] not in ON_OFF_AUTO:
                raise RefError("unrecognised on/off/auto default: " + args[3])
            row["default_text"] = ON_OFF_AUTO[args[3]]
        else:
            value = evaluate(args[3], names)
            row["default"] = value
            row["default_text"] = str(value)
            if value >= MIB and value % MIB == 0:
                row["human"] = "%d MiB" % (value // MIB)
        rows.append(row)
    if len({r["name"] for r in rows}) != len(rows):
        raise RefError("a property is declared twice")
    return rows


def load_text(root: Path = ROOT) -> dict:
    return json.loads((root / TEXT).read_text())


def render_row(row: dict, text: dict) -> str:
    entry = text[row["name"]]
    default = "`%s`" % row["default_text"]
    if row.get("human"):
        default += " (%s)" % row["human"]
    if entry.get("default_note"):
        default += " (%s)" % entry["default_note"] if not row.get("human") else "; " + entry["default_note"]
    name = "`%s`" % row["name"] + (" \\*" if entry.get("new") else "")
    return "| %s | %s | %s | %s | %s |" % (name, row["type"], default, entry["status"], entry["meaning"])


def generate(root: Path = ROOT) -> str:
    rows, text = parse_properties(root), load_text(root)
    lines = [BEGIN, "", "| Property | Type | Default | Status | What it does |", "|---|---|---|---|---|"]
    lines += [render_row(r, text) if r["name"] in text else "| `%s` | | | | MISSING TEXT |" % r["name"] for r in rows]
    lines += ["", "\\* Not described in earlier versions of this manual.", "", END]
    return "\n".join(lines)


def page_block(page_text: str) -> str | None:
    if BEGIN not in page_text or END not in page_text:
        return None
    start = page_text.index(BEGIN)
    return page_text[start:page_text.index(END) + len(END)]


def page_rows(block: str) -> dict[str, str]:
    """name -> the raw default (the first backticked token of the Default cell) of each table row of the page."""
    found = {}
    for line in block.splitlines():
        m = re.match(r"\|\s*`([a-z0-9-]+)`(?: \\\*)?\s*\|\s*[a-z0-9/]+\s*\|\s*`([^`]*)`", line)
        if m:
            found[m[1]] = m[2]
    return found


def check(root: Path = ROOT) -> list[str]:
    errors = []
    try:
        rows = parse_properties(root)
        text = load_text(root)
    except (RefError, OSError, ValueError) as exc:
        return ["cannot read the property table: %s" % exc]
    source = {r["name"]: r for r in rows}
    for name in sorted(source.keys() - text.keys()):
        errors.append("property %s of %s has no entry in %s" % (name, SOURCE, TEXT))
    for name in sorted(text.keys() - source.keys()):
        errors.append("%s names property %s, which %s does not declare" % (TEXT, name, SOURCE))
    for name, entry in text.items():
        if not entry.get("status") or not entry.get("meaning"):
            errors.append("%s: property %s needs a status and a meaning" % (TEXT, name))
    page_path = root / PAGE
    if not page_path.is_file():
        return errors + ["%s does not exist" % PAGE]
    page = page_path.read_text()
    block = page_block(page)
    if block is None:
        return errors + ["%s lacks the generated block markers" % PAGE]
    shown = page_rows(block)
    for name in sorted(source.keys() - shown.keys()):
        errors.append("%s: property %s is missing from the page" % (PAGE, name))
    for name in sorted(shown.keys() - source.keys()):
        errors.append("%s: the page lists %s, which %s does not declare" % (PAGE, name, SOURCE))
    for name in sorted(source.keys() & shown.keys()):
        if shown[name] != source[name]["default_text"]:
            errors.append("%s: property %s shows default %s, the source says %s" % (PAGE, name, shown[name], source[name]["default_text"]))
    if not (source.keys() - text.keys()) and not (text.keys() - source.keys()):
        if block != generate(root):
            errors.append("%s: the generated table is stale; run: python3 scripts/ci/doc_cli_reference.py write" % PAGE)
    return errors


def write(root: Path = ROOT) -> None:
    path = root / PAGE
    page = path.read_text()
    block = page_block(page)
    if block is None:
        raise RefError("%s lacks the generated block markers" % PAGE)
    path.write_text(page.replace(block, generate(root), 1))


def main(argv: list[str]) -> int:
    command = argv[1] if len(argv) > 1 else "check"
    if command == "print":
        print(generate())
        return 0
    if command == "write":
        write()
        return 0
    if command == "check":
        errors = check()
        for line in errors:
            print("CLI_REFERENCE_FAIL " + line)
        print("CLI_REFERENCE %s (%d properties)" % ("FAIL" if errors else "OK", len(parse_properties())))
        return 1 if errors else 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
