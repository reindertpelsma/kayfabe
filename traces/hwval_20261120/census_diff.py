#!/usr/bin/env python3
"""Refused-ids census of a set of qemu logs, diffed against the pre-I7 (gVisor-derived) gate and the generated one.

usage: python3 -I census_diff.py <repo> <tag> <out.tsv> <log-or-gz>...

Reads every `kf3: GSP REFUSED fn<N>/0x<id>=0x<status>` line (fn76 = control, fn103 = class alloc) and every
`kf-rm: RPC-REFUSED #n <Variant> { cmd|class: <decimal> ...` line, counts in how many logs each id appears, and
for the given driver tag (e.g. 580.159.04) prints the verdict of the OLD gate (crates/kf-rm/tests/fixtures/
pre_i7_gate_verdicts.tsv, plus its two rule passes) and the NEW one (tools/api_boundaries/ingress_policy.py
compute(), the same facts the generated table is made from). NEWLY_REFUSED = old allowed, new refuses: the ids
to check against the lane verdicts. Evidence tool of traces/hwval_20261120; GPU-free.
"""
import collections
import gzip
import importlib.util
import os
import re
import sys

repo, tag, out_path, *logs = sys.argv[1:]
spec = importlib.util.spec_from_file_location("ingress_policy", os.path.join(repo, "tools/api_boundaries/ingress_policy.py"))
ip = importlib.util.module_from_spec(spec)
sys.modules["ingress_policy"] = ip
spec.loader.exec_module(ip)

tags, new, _report, _answered, _rules = ip.compute(repo, None)
ti = tags.index(tag)


def new_verdict(kind, i):
    row = new[kind].get(i)
    if row is None:
        return "refuse", "NotOnAllowlist (no row)"
    name, ok, why = row[ti]
    return ("allow" if ok else "refuse"), "%s %s" % (name, why)


old = []
for ln in open(os.path.join(repo, "crates/kf-rm/tests/fixtures/pre_i7_gate_verdicts.tsv")):
    if ln.startswith("#") or not ln.strip():
        continue
    k, i, v, a, b = ln.rstrip("\n").split("\t")
    old.append((k, int(i, 16), v, ip.vkey(a), ip.vkey(b)))


def old_verdict(kind, i):
    t = ip.vkey(tag)
    for k, j, v, a, b in old:
        if k == kind and j == i and a <= t <= b:
            return v
    if kind == "control" and (i & 0x8000 or i >> 16 == 0x2081):
        return "allow"
    return "refuse"


GSP = re.compile(rb"GSP REFUSED fn(\d+)/0x([0-9a-f]+)=0x([0-9a-f]+)")
RPC = re.compile(rb"RPC-REFUSED #\d+ (\w+) \{ (cmd|class): (\d+)")
seen = collections.defaultdict(set)  # (kind,id,source,status) -> logs
for p in logs:
    data = (gzip.open if p.endswith(".gz") else open)(p, "rb").read()
    name = os.path.basename(p)
    for m in GSP.finditer(data):
        kind = {b"76": "control", b"103": "class"}.get(m.group(1), "fn" + m.group(1).decode())
        seen[(kind, int(m.group(2), 16), "GSP", "0x" + m.group(3).decode())].add(name)
    for m in RPC.finditer(data):
        kind = "control" if m.group(2) == b"cmd" else "class"
        seen[(kind, int(m.group(3)), "RPC:" + m.group(1).decode(), "")].add(name)

rows = []
for (kind, i, src, st), names in sorted(seen.items()):
    if kind not in ("control", "class"):
        rows.append((kind, "0x%08x" % i, src, st, len(names), "-", "-", "OTHER_FN"))
        continue
    o = old_verdict(kind, i)
    n, why = new_verdict(kind, i)
    flag = "NEWLY_REFUSED" if (o == "allow" and n == "refuse") else ("NEWLY_ADMITTED" if (o == "refuse" and n == "allow") else "same")
    rows.append((kind, "0x%08x" % i, src, st, len(names), o, n, flag + " | " + why))
with open(out_path, "w") as f:
    f.write("# refused-ids census at tag %s over %d logs\n# kind\tid\tsource\tstatus\tlogs\told_gate\tnew_gate\tflag | new reason\n" % (tag, len(logs)))
    for r in rows:
        f.write("\t".join(str(x) for x in r) + "\n")
c = collections.Counter(r[7].split(" |")[0] for r in rows)
print("ids", len(rows), dict(c))
