#!/usr/bin/env python3
"""Print the set of identifier tokens (code AND comments) used in kayfabe's v3 crates (crates/kf-*).

usage: kf_idents.py <kayfabe-tree> > idents.txt

Used by analyze.py to flag which ogkm items (GSP structs, defines, class names, RPC functions) the
v3 code names: the Rust mirrors cite the C name in their doc comments, so comments count here.
A token that carries an NV_VGPU_MSG_FUNCTION_ / NV_VGPU_MSG_EVENT_ prefix is also emitted without it.
"""
import os
import re
import sys

root = sys.argv[1]
toks = set()
for d in sorted(os.listdir(os.path.join(root, "crates"))):
    if not d.startswith("kf-"):
        continue
    for dp, _dn, fn in os.walk(os.path.join(root, "crates", d)):
        for f in fn:
            if f.endswith((".rs", ".tsv", ".md")):
                t = open(os.path.join(dp, f), encoding="utf-8", errors="replace").read()
                for m in re.finditer(r"[A-Za-z_][A-Za-z0-9_]{3,}", t):
                    toks.add(m.group(0))
for t in list(toks):
    for p in ("NV_VGPU_MSG_FUNCTION_", "NV_VGPU_MSG_EVENT_"):
        if t.startswith(p):
            toks.add(t[len(p):])
print("\n".join(sorted(toks)))
