#!/usr/bin/env python3
"""Derive the set of ogkm API items kayfabe serves or probes, from kayfabe's own source.

usage: derive_served.py <kayfabe-tree> [extra-ids-file ...] > served.json

Reads (comment-stripped, so a name that only appears in a comment is NOT counted):
  every crates/kf-* crate (generated/classes.rs is read for the class table; other generated/ files
  are skipped for control names)

Output JSON:
  ctrl_names   : sorted RM control symbol names referenced in code (NV####_CTRL_CMD_*)
  ctrl_ids     : sorted 0x######## control ids referenced as literals in code (top half is a
                 known control class) plus the ids in the extra-ids file (the matrix F-4 /
                 group P/K/I/A lists, which are written in the doc, not in code)
  ctrl_id_sources: {id: [code | extra_ids.txt | windows_ids.txt]}
  classes      : {NAME: id} from kf-abi generated/classes.rs
  alloc_structs: allocation / NVOS parameter struct names referenced in code
Nothing here reads a blob or a closed binary.
"""
import json
import os
import re
import sys

# every v3 crate (crates/kf-*); the frozen pre-v3 tree crates/kayfabe-* is not v3 and is not read
CODE_DIRS = None
# Top 16 bits of a control id = the class the control belongs to.
CTRL_CLASS_TOPS = {"2080", "0080", "0073", "2081", "5070", "0041", "00f1", "30f1", "9096",
                   "00de", "0090", "208f", "2082", "00fd", "0071", "0000", "00da", "9067",
                   "a06f", "c36f", "c56f", "906f", "b06f", "50a0", "003e"}


def strip_rust_comments(s):
    s = re.sub(r"/\*.*?\*/", " ", s, flags=re.S)
    return re.sub(r"//[^\n]*", " ", s)


def main():
    root = sys.argv[1]
    extras = sys.argv[2:]
    names, ids, allocs = set(), set(), set()
    src = {}
    classes = {}
    dirs = CODE_DIRS or sorted("crates/" + d for d in os.listdir(os.path.join(root, "crates"))
                               if d.startswith("kf-"))
    for d in dirs:
        for dp, _dn, fn in os.walk(os.path.join(root, d)):
            if "/tests" in dp or "/target" in dp:
                continue
            for f in fn:
                if not f.endswith(".rs"):
                    continue
                p = os.path.join(dp, f)
                raw = open(p, encoding="utf-8", errors="replace").read()
                if p.endswith("generated/classes.rs"):
                    for m in re.finditer(r"pub const (\w+): u32 = (0x[0-9a-fA-F]+);", raw):
                        classes[m.group(1)] = int(m.group(2), 16)
                    continue
                if "/generated/" in p:
                    continue
                code = strip_rust_comments(raw)
                for m in re.finditer(r"\bNV[0-9A-Fa-f]{4}_CTRL_CMD_[A-Z0-9_]+", code):
                    names.add(m.group(0))
                for m in re.finditer(r"\b0x([0-9a-fA-F]{4})_?([0-9a-fA-F]{4})\b", code):
                    top = m.group(1).lower()
                    if top in CTRL_CLASS_TOPS and top != "0000":
                        ids.add(int(m.group(1) + m.group(2), 16))
                        src.setdefault(int(m.group(1) + m.group(2), 16), set()).add("code")
                for m in re.finditer(
                        r"\bNV[0-9A-Za-z_]*(?:ALLOC|ALLOCATION)_PARAM(?:ETER)?S\b|\bNVOS[0-9A-Z]+_PARAMETERS\b",
                        code):
                    allocs.add(m.group(0))
    for extra in extras:
        for ln in open(extra):
            ln = ln.split("#")[0].strip()
            if ln:
                ids.add(int(ln, 16))
                src.setdefault(int(ln, 16), set()).add(os.path.basename(extra))
    out = {
        "ctrl_names": sorted(names),
        "ctrl_ids": sorted("0x%08x" % i for i in ids),
        "ctrl_id_sources": {"0x%08x" % i: sorted(v) for i, v in sorted(src.items())},
        "classes": classes,
        "alloc_structs": sorted(allocs),
    }
    json.dump(out, sys.stdout, indent=1, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
