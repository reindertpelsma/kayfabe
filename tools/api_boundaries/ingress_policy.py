#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# STATUS: LIVE, 2026-11-20 (task I7; design: docs/design/V3_GUEST_INGRESS_TABLE.md).
"""The guest-ingress allowlist, generated from NVIDIA's open-gpu-kernel-modules (ogkm) and kayfabe's own data.

Inputs, and nothing else:
  - ogkm release trees (one per tag): the exported-method tables (src/nvidia/generated/g_*_nvoc.c), the
    RMCTRL_FLAGS_* values (src/nvidia/inc/kernel/rmapi/control.h), the class privilege table
    (src/nvidia/src/kernel/rmapi/resource_list.h) and the control / class names (src/common/sdk/nvidia/inc);
  - tools/api_boundaries/served.json (derive_served.py: what kayfabe's code names, the gate excluded) and
    measured_points.tsv (the measured host tags, for the report);
  - tools/api_boundaries/ingress_rules.tsv: kayfabe's own rows (Mode2Rpc, Empirical) and judgements;
  - tools/api_boundaries/answered_never_forwarded.txt (ids kayfabe answers itself);
  - the tag list: the "# tags" line of traces/driver_matrix/ranges.tsv (the guest versions kayfabe has a
    driver-ABI table for).

The policy (ours, docs/design/V3_GUEST_INGRESS_TABLE.md section 2):
  control  ALLOWED at tag T  iff  served  and  T exports it  and  T's flags have NON_PRIVILEGED
                                   and neither PRIVILEGED, INTERNAL nor RM_TEST_ONLY_CODE;
  class    ALLOWED at tag T  iff  served  and  resource_list.h at T gives it RS_FLAGS_ALLOC_NON_PRIVILEGED
                                   and neither RS_FLAGS_ALLOC_PRIVILEGED nor _KERNEL_PRIVILEGED,
                                   or it is a client root (internal class RmClientResource: serverAllocClient
                                   has no resource privilege check);
  a rule row of ingress_rules.tsv overrides the derived verdict for its tags (allow: Mode2Rpc / Empirical;
  refuse: Judgement), and every override is listed in the report;
  every other served id is REFUSED BY NAME with its reason; an id no tag of ogkm knows goes to the report.
  Unserved ids have no row: the gate's default is to refuse.

usage:
  ingress_policy.py extract --clone CLONE --out FACTS_DIR [TAG ...]     check out each tag, write FACTS_DIR/<tag>.json
  ingress_policy.py extract --tree TAG=PATH ... --out FACTS_DIR         the same from existing trees
  ingress_policy.py emit --facts FACTS_DIR [--repo ROOT]                 write the Rust table and the report
  ingress_policy.py check --facts FACTS_DIR [--repo ROOT]                exit 1 if the committed outputs are stale
Nothing is compiled; no blob, no closed binary and no third-party allowlist is read.
"""
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = "tools/api_boundaries/ingress_policy.py"
OUT_RS = "crates/kf-abi/src/ingress_generated.rs"
OUT_REPORT = "tools/api_boundaries/ingress_report.tsv"
RULES = "tools/api_boundaries/ingress_rules.tsv"
SERVED = "tools/api_boundaries/served.json"
ANSWERED = "tools/api_boundaries/answered_never_forwarded.txt"
RANGES = "traces/driver_matrix/ranges.tsv"

# --------------------------------------------------------------------------------------- C text helpers
_CMT = re.compile(r"/\*.*?\*/|//[^\n]*", re.S)


def _read(p):
    with open(p, "rb") as f:
        return f.read().decode("utf-8", "replace")


def _strip(t):
    return _CMT.sub(" ", t)


def vkey(tag):
    return tuple(int(x) for x in tag.split("."))


# --------------------------------------------------------------------------------------- per-tag extraction
# One exported-method entry of an NVOC class: flags, then methodId, then the owning class.
_EXPORT = re.compile(
    r"/\*flags=\*/\s*(0x[0-9a-fA-F]+)[uU]?\s*,\s*/\*accessRight=\*/[^,]*,\s*/\*methodId=\*/\s*"
    r"(0x[0-9a-fA-F]+)[uU]?\s*,[^{}]*?/\*pClassInfo=\*/\s*&\(__nvoc_class_def_(\w+)\.classInfo\)")
_DEFINE_HEX = re.compile(r"^[ \t]*#[ \t]*define[ \t]+(\w+)[ \t]+\(?[ \t]*(0x[0-9a-fA-F]+)[uUlL]*[ \t]*\)?", re.M)
_FINN_CTRL = re.compile(r"^[ \t]*#[ \t]*define[ \t]+(NV\w+)[ \t]+\((0x[0-9a-fA-F]{5,8})U?\)[ \t]*/\*[ \t]*finn: Evaluated from",
                        re.M)
_RS_ENTRY = re.compile(r"RS_ENTRY\s*\((.*?)\)\s*(?=RS_ENTRY|#|$)", re.S)


def _flag_values(control_h):
    """RMCTRL_FLAGS_<NAME> -> value, read from THIS tag's control.h (the bit values moved in R555)."""
    vals = {}
    for m in re.finditer(r"#\s*define\s+RMCTRL_FLAGS_(\w+)\s+(0x[0-9a-fA-F]+)\b", _strip(_read(control_h))):
        if int(m.group(2), 16):
            vals[m.group(1)] = int(m.group(2), 16)
    return vals


def _flag_names(value, vals):
    return sorted(n for n, v in vals.items() if value & v == v)


def _rs_entries(resource_list):
    """[(external class name, [RS_FLAGS_* names], guarded)] in file order."""
    text = _strip(_read(resource_list))
    out = []
    # Track #if/#endif nesting by position so an entry inside a conditional block is marked.
    conds = []
    depth = 0
    for m in re.finditer(r"^[ \t]*#[ \t]*(if|ifdef|ifndef|endif)\b[^\n]*", text, re.M):
        conds.append((m.start(), m.group(1)))
    i = 0
    for m in re.finditer(r"RS_ENTRY\s*\(", text):
        while i < len(conds) and conds[i][0] < m.start():
            depth += -1 if conds[i][1] == "endif" else 1
            i += 1
        start = m.end()
        lvl, j = 1, start
        while lvl and j < len(text):
            c = text[j]
            lvl += 1 if c == "(" else -1 if c == ")" else 0
            j += 1
        body = text[start:j - 1]
        fields = _split_top(body)
        if len(fields) < 7:
            continue
        ext = fields[0].strip()
        flags = re.findall(r"RS_FLAGS_(\w+)", fields[6])
        # A client root (internal class RmClientResource) is allocated by serverAllocClient, which has no
        # resource privilege check (ogkm-580 libraries/resserv/src/rs_server.c:728-766): any caller opens one.
        if fields[1].strip() == "RmClientResource":
            flags.append("CLIENT_ROOT")
        out.append((ext, sorted(set(flags)), depth > 0))
    return out


def _split_top(body):
    parts, lvl, cur = [], 0, []
    for c in body:
        if c == "(":
            lvl += 1
        elif c == ")":
            lvl -= 1
        if c == "," and lvl == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(c)
    parts.append("".join(cur))
    return parts


def _keep_smallest(sink, key, value):
    """Order-independent choice when one key is defined twice (the 0xffffffff "no override" macros):
    the smallest value wins, so the extraction does not depend on directory order of the checkout."""
    if key not in sink or value < sink[key]:
        sink[key] = value


def extract_tree(tree, tag):
    """-> facts for one ogkm tag: every exported control and every resource_list.h class."""
    src = os.path.join(tree, "src")
    vals = _flag_values(os.path.join(src, "nvidia/inc/kernel/rmapi/control.h"))
    sdk = os.path.join(src, "common/sdk/nvidia/inc")
    ctrl_names, class_ids, finn_names = {}, {}, {}
    for sub, sink in (("ctrl", ctrl_names), ("class", class_ids)):
        for dp, _dn, fn in os.walk(os.path.join(sdk, sub)):
            for f in sorted(fn):
                if not f.endswith(".h"):
                    continue
                raw = _read(os.path.join(dp, f))
                if sub == "ctrl":  # a control id NVIDIA's generator annotated (not every one says _CMD_)
                    for m in _FINN_CTRL.finditer(raw):
                        if "_CTRL_CMD_" not in m.group(1):
                            _keep_smallest(finn_names, int(m.group(2), 16), m.group(1))
                for m in _DEFINE_HEX.finditer(_strip(raw)):
                    name, val = m.group(1), int(m.group(2), 16)
                    if sub == "ctrl" and "_CTRL_CMD_" in name and val > 0xffff:
                        _keep_smallest(sink, val, name)
                    elif sub == "class" and val <= 0xffff and not name.endswith("_H_"):
                        _keep_smallest(sink, name, val)
    for cid, n in finn_names.items():
        if cid not in ctrl_names:
            ctrl_names[cid] = n
    # A few classes (the NV01_EVENT_* family) are defined in nvos.h rather than a class header.
    for m in _DEFINE_HEX.finditer(_strip(_read(os.path.join(sdk, "nvos.h")))):
        if int(m.group(2), 16) <= 0xffff:
            class_ids.setdefault(m.group(1), int(m.group(2), 16))
    controls = {}
    gen = os.path.join(src, "nvidia/generated")
    for f in sorted(os.listdir(gen)):
        if not (f.startswith("g_") and f.endswith("_nvoc.c")):
            continue
        for m in _EXPORT.finditer(_read(os.path.join(gen, f))):
            cid = int(m.group(2), 16)
            e = controls.setdefault("0x%08x" % cid, {"name": ctrl_names.get(cid, ""), "exports": []})
            e["exports"].append({"cls": m.group(3), "flags": _flag_names(int(m.group(1), 16), vals)})
    classes = {}
    for ext, flags, guarded in _rs_entries(os.path.join(src, "nvidia/src/kernel/rmapi/resource_list.h")):
        if ext not in class_ids:
            continue
        cid = "0x%04x" % class_ids[ext]
        c = classes.setdefault(cid, {"name": ext, "flags": [], "guarded": guarded})
        c["flags"] = sorted(set(c["flags"]) | set(flags))
    names = {c["name"]: cid for cid, c in classes.items()}
    return {"tag": tag, "controls": controls, "classes": classes, "class_names": names,
            "ctrl_names": {n: "0x%08x" % i for i, n in ctrl_names.items()}}


def cmd_extract(a):
    os.makedirs(a.out, exist_ok=True)
    jobs = []
    if a.tree:
        for t in a.tree:
            tag, path = t.split("=", 1)
            jobs.append((tag, path, False))
    else:
        tags = a.tags or read_tags(a.repo)
        jobs = [(t, a.clone, True) for t in tags]
    for tag, path, checkout in jobs:
        out = os.path.join(a.out, tag + ".json")
        if os.path.exists(out) and not a.force:
            continue
        if checkout:
            subprocess.check_call(["git", "-C", path, "checkout", "-q", "--detach", "refs/tags/" + tag])
        facts = extract_tree(path, tag)
        with open(out + ".tmp", "w") as f:
            json.dump(facts, f, sort_keys=True, indent=0)
        os.replace(out + ".tmp", out)
        print(tag, len(facts["controls"]), "controls", len(facts["classes"]), "classes", flush=True)
    if a.digest:
        _, per = load_facts(a.repo, a.out)
        write_facts(a.repo, per)
        print("wrote", FACTS)


# --------------------------------------------------------------------------------------- policy
def read_tags(repo):
    for ln in open(os.path.join(repo, RANGES)):
        if ln.startswith("# tags"):
            return sorted(ln.split()[2:], key=vkey)
    raise SystemExit("no '# tags' line in " + RANGES)


def read_rules(repo):
    """ingress_rules.tsv -> [dict].  Columns: kind id name verdict basis from until why evidence."""
    rows = []
    for ln in open(os.path.join(repo, RULES)):
        if not ln.strip() or ln.startswith("#"):
            continue
        f = ln.rstrip("\n").split("\t")
        if len(f) != 9:
            raise SystemExit("ingress_rules.tsv: need 9 tab-separated columns: " + ln)
        kind, ident, name, verdict, basis, frm, until, why, evidence = f
        if kind not in ("control", "class") or verdict not in ("allow", "refuse"):
            raise SystemExit("ingress_rules.tsv: bad kind/verdict: " + ln)
        if (verdict == "allow") != (basis in ("Mode2Rpc", "Empirical")):
            raise SystemExit("ingress_rules.tsv: allow needs basis Mode2Rpc/Empirical, refuse a judgement: " + ln)
        rows.append({"kind": kind, "id": int(ident, 16), "name": name, "verdict": verdict, "basis": basis,
                     "from": None if frm == "-" else vkey(frm), "until": None if until == "-" else vkey(until),
                     "why": why, "evidence": evidence})
    return rows


def served_sets(served, facts_by_tag):
    """-> (control ids, class ids) kayfabe serves: what its code names, resolved against every tag's names."""
    ctrl = {int(x, 16) for x in served["ctrl_ids"]}
    names = set(served["ctrl_names"])
    cls = {int(x, 16) for x in served["class_id_literals"]} | set(served["classes"].values())
    idents = set(served["upper_idents"]) | set(served["classes"])
    for facts in facts_by_tag.values():
        for n, i in facts["ctrl_names"].items():
            if n in names:
                ctrl.add(int(i, 16))
        for n, i in facts["class_names"].items():
            if n in idents:
                cls.add(int(i, 16))
    return ctrl, cls


def control_verdict(e):
    """Derived verdict of one exported control at one tag -> (allowed, reason)."""
    if e is None:
        return False, "NotExported"
    flags = set()
    for x in e["exports"]:
        flags |= set(x["flags"])
    each = [set(x["flags"]) for x in e["exports"]]
    if any("INTERNAL" in f for f in each):
        return False, "Internal"
    if any("RM_TEST_ONLY_CODE" in f for f in each):
        return False, "TestOnly"
    if any("PRIVILEGED" in f for f in each):
        return False, "Privileged"
    if not all("NON_PRIVILEGED" in f for f in each):
        return False, "KernelOnly"
    return True, "OgkmNonPrivileged"


def class_verdict(c):
    if c is None:
        return False, "NotInResourceList"
    f = set(c["flags"])
    if "ALLOC_KERNEL_PRIVILEGED" in f:
        return False, "KernelOnly"
    if "ALLOC_PRIVILEGED" in f:
        return False, "Privileged"
    if "CLIENT_ROOT" in f:
        return True, "OgkmClientRoot"
    if "ALLOC_NON_PRIVILEGED" not in f:
        return False, "NoPrivilegeFlag"
    return True, "OgkmNonPrivileged"


FACTS = "tools/api_boundaries/ingress_facts.tsv"
# The flags that decide a verdict; every other RMCTRL / RS flag (locking, caching, routing) is not kept.
KEEP_CTRL = {"NON_PRIVILEGED", "PRIVILEGED", "INTERNAL", "RM_TEST_ONLY_CODE", "PRIVILEGED_IF_RS_ACCESS_DISABLED"}
KEEP_CLASS = {"ALLOC_NON_PRIVILEGED", "ALLOC_PRIVILEGED", "ALLOC_KERNEL_PRIVILEGED", "CLIENT_ROOT"}


def _fact_rows(f):
    """One tag's facts -> {(kind, id): (name, flags-string)}; flags '-' = named in a header, not exported."""
    rows = {}
    for cid, e in f["controls"].items():
        fl = ";".join(sorted("|".join(sorted(set(x["flags"]) & KEEP_CTRL)) or "KERNEL" for x in e["exports"]))
        rows[("control", int(cid, 16))] = (e["name"], fl)
    for n, cid in f["ctrl_names"].items():
        rows.setdefault(("control", int(cid, 16)), (n, "-"))
    for cid, c in f["classes"].items():
        rows[("class", int(cid, 16))] = (c["name"], "|".join(sorted(set(c["flags"]) & KEEP_CLASS)) or "NONE")
    return rows


def _rows_to_facts(tag, rows):
    f = {"tag": tag, "controls": {}, "classes": {}, "class_names": {}, "ctrl_names": {}}
    for (kind, i), (name, fl) in rows.items():
        if kind == "control":
            if name:
                f["ctrl_names"][name] = "0x%08x" % i
            if fl != "-":
                f["controls"]["0x%08x" % i] = {"name": name, "exports": [
                    {"flags": [] if x == "KERNEL" else x.split("|")} for x in fl.split(";")]}
        else:
            f["class_names"][name] = "0x%04x" % i
            f["classes"]["0x%04x" % i] = {"name": name, "flags": [] if fl == "NONE" else fl.split("|")}
    return f


def load_facts(repo, facts_dir=None):
    """Per-tag facts: the committed ingress_facts.tsv, with every <tag>.json of facts_dir (fresh extraction)
    taking precedence for its tag."""
    tags = read_tags(repo)
    per = {}
    p = os.path.join(repo, FACTS)
    if os.path.exists(p):
        for ln in open(p):
            if ln.startswith("#") or not ln.strip():
                continue
            kind, ident, first, last, name, fl = ln.rstrip("\n").split("\t")
            for t in tags:
                if vkey(first) <= vkey(t) <= vkey(last):
                    per.setdefault(t, {})[(kind, int(ident, 16))] = (name, fl)
    if facts_dir:
        for t in tags:
            j = os.path.join(facts_dir, t + ".json")
            if os.path.exists(j):
                per[t] = _fact_rows(json.load(open(j)))
    return {t: _rows_to_facts(t, rows) for t, rows in per.items()}, per


def facts_at(repo, tag):
    """The committed facts rows at one tag, read from ingress_facts.tsv alone (no tag list needed)."""
    rows = {}
    for ln in open(os.path.join(repo, FACTS)):
        if ln.startswith("#") or not ln.strip():
            continue
        kind, ident, first, last, name, fl = ln.rstrip("\n").split("\t")
        if vkey(first) <= vkey(tag) <= vkey(last):
            rows[(kind, int(ident, 16))] = (name, fl)
    return rows


def write_facts(repo, per):
    tags = [t for t in read_tags(repo) if t in per]
    keys = sorted(set().union(*[set(per[t]) for t in tags]))
    L = ["# @generated by %s extract from ogkm %s..%s (%d tags); do not edit." % (TOOL, tags[0], tags[-1], len(tags)),
         "# Facts of NVIDIA's open-gpu-kernel-modules (MIT): per control the privilege flags of each exporting",
         "# class (';' between exports, KERNEL = no visibility flag, '-' = named in a header but not exported);",
         "# per class its RS_FLAGS_ALLOC_* privilege. One row per run of equal facts over consecutive tags.",
         "# kind\tid\tfirst_tag\tlast_tag\tname\tflags"]
    for k in keys:
        run = None
        for t in tags + [None]:
            v = per[t].get(k) if t else None
            if run and v == run[2]:
                run[1] = t
                continue
            if run and run[2] is not None:
                L.append("%s\t0x%08x\t%s\t%s\t%s\t%s" % (k[0], k[1], run[0], run[1], run[2][0], run[2][1]))
            run = [t, t, v] if t else None
    os.makedirs(os.path.dirname(os.path.join(repo, FACTS)), exist_ok=True)
    with open(os.path.join(repo, FACTS), "w") as f:
        f.write("\n".join(L) + "\n")


def compute(repo, facts_dir):
    tags = read_tags(repo)
    facts, _ = load_facts(repo, facts_dir)
    missing = [t for t in tags if t not in facts]
    if missing:
        raise SystemExit("no ogkm facts for tag(s) %s: run '%s extract' first" % (" ".join(missing), TOOL))
    served = json.load(open(os.path.join(repo, SERVED)))
    rules = read_rules(repo)
    answered = set()
    for ln in open(os.path.join(repo, ANSWERED)):
        ln = ln.split("#")[0].strip()
        if ln:
            answered.add(int(ln, 16))
    s_ctrl, s_cls = served_sets(served, facts)
    for r in rules:  # a rule row must name its id the way some tag of ogkm does (or no tag knows the id)
        key = ("0x%08x" if r["kind"] == "control" else "0x%04x") % r["id"]
        seen = {(f["controls"] if r["kind"] == "control" else f["classes"]).get(key, {}).get("name")
                for f in facts.values()} | {n for f in facts.values()
                                             for n, i in f["ctrl_names" if r["kind"] == "control" else "class_names"].items()
                                             if int(i, 16) == r["id"]}
        seen.discard(None)
        seen.discard("")
        if seen and r["name"] not in seen:
            raise SystemExit("ingress_rules.tsv: %s %s is named %s by ogkm, not %s" % (r["kind"], key, sorted(seen),
                                                                                       r["name"]))
    for r in rules:
        (s_ctrl if r["kind"] == "control" else s_cls).add(r["id"])
    out = {"control": {}, "class": {}}
    report = []
    for kind, ids in (("control", s_ctrl), ("class", s_cls)):
        for i in sorted(ids):
            per_tag = []
            known_anywhere = False
            for t in tags:
                f = facts[t]
                if kind == "control":
                    e = f["controls"].get("0x%08x" % i)
                    name = (e or {}).get("name") or _name_of(f["ctrl_names"], "0x%08x" % i)
                    ok, why = control_verdict(e)
                else:
                    e = f["classes"].get("0x%04x" % i)
                    name = (e or {}).get("name") or _name_of(f["class_names"], "0x%04x" % i)
                    ok, why = class_verdict(e)
                known_anywhere |= e is not None
                basis = why if ok else None
                for r in rules:
                    if r["kind"] != kind or r["id"] != i:
                        continue
                    if r["from"] and vkey(t) < r["from"]:
                        continue
                    if r["until"] and vkey(t) > r["until"]:
                        continue
                    name = name or r["name"]
                    if r["verdict"] == "allow":
                        ok, basis, why = True, r["basis"], r["basis"]
                    else:
                        ok, basis, why = False, None, "Judgement:" + r["basis"]
                per_tag.append((name or "", ok, why))
            # A tag where the id is absent still refuses it by a name: the nearest tag's.
            known = [n for n, _ok, _why in per_tag if n]
            if known:
                fill = [next((n for n, _o, _w in per_tag[k:] if n), known[-1]) for k in range(len(per_tag))]
                per_tag = [(n or fill[k], ok, why) for k, (n, ok, why) in enumerate(per_tag)]
            ruled = any(r["kind"] == kind and r["id"] == i for r in rules)
            if not known_anywhere and kind == "class" and not ruled:
                continue  # a class candidate literal that is no ogkm class: not a class at all
            if not known_anywhere:
                report.append((kind, i, "unknown-to-ogkm", per_tag[-1][2], "no tag of %s..%s exports it"
                               % (tags[0], tags[-1])))
                if not ruled:
                    continue  # refused by the default; there is no name to refuse it by
            out[kind][i] = per_tag
    return tags, out, report, answered, rules


def _name_of(names, ident):
    for n, i in names.items():
        if i == ident:
            return n
    return ""


# --------------------------------------------------------------------------------------- emit
def runs(per_tag):
    """Collapse equal consecutive (name, ok, why) triples into [first, last] index runs."""
    rs = []
    for k, v in enumerate(per_tag):
        if rs and rs[-1][2] == v:
            rs[-1][1] = k
        else:
            rs.append([k, k, v])
    return rs


REASONS = {
    "OgkmNonPrivileged": "Verdict::Allow(Basis::OgkmNonPrivileged)",
    "OgkmClientRoot": "Verdict::Allow(Basis::OgkmClientRoot)",
    "Mode2Rpc": "Verdict::Allow(Basis::Mode2Rpc)",
    "Empirical": "Verdict::Allow(Basis::Empirical)",
    "Internal": "Verdict::Refuse(Refusal::Internal)",
    "TestOnly": "Verdict::Refuse(Refusal::TestOnly)",
    "Privileged": "Verdict::Refuse(Refusal::Privileged)",
    "KernelOnly": "Verdict::Refuse(Refusal::KernelOnly)",
    "NoPrivilegeFlag": "Verdict::Refuse(Refusal::NoPrivilegeFlag)",
    "NotExported": "Verdict::Refuse(Refusal::AbsentAtTag)",
    "NotInResourceList": "Verdict::Refuse(Refusal::AbsentAtTag)",
}


def rust_verdict(why):
    if why.startswith("Judgement:"):
        return "Verdict::Refuse(Refusal::Judgement(Judgement::%s))" % why.split(":", 1)[1]
    return REASONS[why]


def emit_rust(tags, out, answered):
    L = []
    w = L.append
    w("// @generated from ogkm %s..%s (%d tags) + served.json by %s; do not edit."
      % (tags[0], tags[-1], len(tags), TOOL))
    w("// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later")
    w("// The ogkm facts are NVIDIA's (MIT, see THIRD_PARTY_NOTICES.md); the policy and the rule rows are")
    w("// kayfabe's (tools/api_boundaries/ingress_rules.tsv, docs/design/V3_GUEST_INGRESS_TABLE.md).")
    w("// Regenerate: tools/api_boundaries/ingress_policy.py extract ... && ingress_policy.py emit ...")
    w("")
    w("use crate::DriverVersion;")
    w("use crate::capability::{Basis, IngressRow, Judgement, Refusal, Run, Verdict};")
    w("")
    w("/// The guest driver tags this table covers (the driver matrix's tags), ascending.")
    w("pub const TAGS: &[DriverVersion] = &[")
    for t in tags:
        a, b, c = (vkey(t) + (0,))[:3]
        w("    DriverVersion { major: %d, minor: %d, patch: %d }," % (a, b, c))
    w("];")
    for kind, title in (("control", "CONTROLS"), ("class", "CLASSES")):
        w("")
        w("/// Every %s kayfabe serves (or a rule row names), sorted by id; one run per span of equal verdicts."
          % kind)
        w("pub static %s: &[IngressRow] = &[" % title)
        for i in sorted(out[kind]):
            fwd = "false" if (kind == "control" and i in answered) or kind == "class" else "true"
            if kind == "class":
                fwd = "false"
            w("    IngressRow {")
            w("        id: 0x%08x," % i)
            w("        forwarded: %s," % fwd)
            w("        runs: &[")
            for first, last, (name, _ok, why) in runs(out[kind][i]):
                w("            Run { first: %d, last: %d, name: %s, verdict: %s }," % (
                    first, last, json.dumps(name), rust_verdict(why)))
            w("        ],")
            w("    },")
        w("];")
    return "\n".join(L) + "\n"


def emit_report(tags, out, report, rules):
    L = ["# @generated by %s from ogkm %s..%s + served.json; do not edit." % (TOOL, tags[0], tags[-1]),
         "# STATUS: LIVE, 2026-11-20 (docs/design/V3_GUEST_INGRESS_TABLE.md).",
         "# kind\tid\tfinding\tverdict_at_newest\tdetail"]
    for kind, i, what, why, detail in report:
        L.append("%s\t0x%08x\t%s\t%s\t%s" % (kind, i, what, why, detail))
    for r in rules:
        L.append("%s\t0x%08x\trule-%s\t%s\t%s: %s" % (r["kind"], r["id"], r["verdict"], r["basis"], r["name"],
                                                     r["why"]))
    for kind in ("control", "class"):
        for i in sorted(out[kind]):
            for first, last, (name, ok, why) in runs(out[kind][i]):
                if not ok and not why.startswith("Judgement:"):
                    L.append("%s\t0x%08x\trefused-by-name\t%s\t%s %s..%s" % (kind, i, why, name or "?",
                                                                            tags[first], tags[last]))
    return "\n".join(L) + "\n"


def render(repo, facts_dir):
    tags, out, report, answered, rules = compute(repo, facts_dir)
    return emit_rust(tags, out, answered), emit_report(tags, out, report, rules)


def cmd_emit(a):
    rs, rep = render(a.repo, a.facts)
    for rel, text in ((OUT_RS, rs), (OUT_REPORT, rep)):
        os.makedirs(os.path.dirname(os.path.join(a.repo, rel)), exist_ok=True)
        with open(os.path.join(a.repo, rel), "w") as f:
            f.write(text)
        print("wrote", rel)


def cmd_check(a):
    rs, rep = render(a.repo, a.facts)
    bad = [rel for rel, text in ((OUT_RS, rs), (OUT_REPORT, rep)) if _read(os.path.join(a.repo, rel)) != text]
    if bad:
        print("INGRESS_TABLE_STALE " + " ".join(bad) + ": rerun %s emit" % TOOL, file=sys.stderr)
        return 1
    print("INGRESS_TABLE_CURRENT")
    return 0


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("extract")
    e.add_argument("--clone")
    e.add_argument("--tree", action="append")
    e.add_argument("--out", required=True)
    e.add_argument("--repo", default=os.path.normpath(os.path.join(HERE, "../..")))
    e.add_argument("--force", action="store_true")
    e.add_argument("--digest", action="store_true", help="merge the extracted tags into " + "ingress_facts.tsv")
    e.add_argument("tags", nargs="*")
    for name in ("emit", "check"):
        s = sub.add_parser(name)
        s.add_argument("--facts", help="a directory of freshly extracted <tag>.json (else the committed TSV)")
        s.add_argument("--repo", default=os.path.normpath(os.path.join(HERE, "../..")))
    a = p.parse_args(argv)
    if a.cmd == "extract":
        if not (a.clone or a.tree):
            p.error("extract needs --clone or --tree")
        return cmd_extract(a) or 0
    return {"emit": cmd_emit, "check": cmd_check}[a.cmd](a) or 0


if __name__ == "__main__":
    sys.exit(main())
