#!/usr/bin/env python3
"""Change points, intervals, the interpolation-rule test and the measurement plan.

usage: analyze.py <per-tag-dir> <served.json> <idents.txt> <measured_points.tsv> <windows_controls.tsv> <outdir>

Inputs are the per-tag JSONs written by sweep.py.  Writes into <outdir>:
  tags.tsv                    tag, branch, item count
  item_runs.json.gz           lossless: item -> [[first_tag_idx, last_tag_idx, sig], ...] (+ tags list)
  changepoints_<area>.tsv     per consecutive tag pair: the exact items whose signature changed
  intervals_<area>.tsv        maximal runs of tags with an identical area signature
  rule_violations.tsv         items for which the rule (A<B<C, sig(A)==sig(C) => sig(B)==sig(A)) fails
  rule_summary.json           counts per area
  plan_<axis>.tsv             interval -> covered by (measured) / needs; boundaries from monotone items only
  plan_<axis>_all_items.tsv   same, boundaries from every item including the ones that fail the rule
  plan_<axis>[_all_items]_material.tsv   strict intervals merged across ADD-only / class-header-only boundaries
  f4.json                     the six F-4 controls and the 580 -> 595 differences, from source
  size_crosscheck.tsv         Windows 580.88 vs 596.51 request sizes against the ogkm signature change
  summary.json                everything the doc quotes
"""
import csv
import gzip
import hashlib
import json
import os
import re
import sys
from collections import Counter, defaultdict

MISSING = "-"  # item not present in the tag at all
# The supported range of kayfabe is [535.309.01, 615.71.09] (docs/design/V3_DRIVER_MATRIX.md); the plan is clipped to it.
WINDOW_START = "535.309.01"


def vkey(t):
    return [int(x) for x in re.findall(r"\d+", t)]


def h12(s):
    return hashlib.sha1(s.encode()).hexdigest()[:12]


# GSP items on the boot / message-queue path (what kayfabe, as the GSP, must lay out exactly)
BOOT_RE = re.compile(r"(GspSystemInfo|GspStaticConfigInfo|GspFwWprMeta|GspFwSRMeta|GSP_FMC_BOOT_PARAMS|GSP_ARGUMENTS_CACHED|"
                     r"MESSAGE_QUEUE|GSP_MSG_QUEUE|rpc_message_header|LibosMemoryRegion|rpc_gsp_set_system_info|"
                     r"rpc_set_guest_system_info|rpc_run_cpu_sequencer|rpc_gsp_rm_(alloc|control)|GSP_VF_INFO)")


def norm_name(n):
    return re.sub(r"_", "", n).lower()


def main():
    perdir, servedp, idp, mpp, wcp, outdir = sys.argv[1:7]
    os.makedirs(outdir, exist_ok=True)
    served = json.load(open(servedp))
    idents = set(norm_name(x.strip()) for x in open(idp) if len(x.strip()) >= 6)
    tags = sorted((f[:-5] for f in os.listdir(perdir) if f.endswith(".json")), key=vkey)
    T = len(tags)
    tidx = {t: i for i, t in enumerate(tags)}
    data = {}
    for t in tags:
        data[t] = json.load(open(os.path.join(perdir, t + ".json")))
    details = defaultdict(dict)  # item -> {tag: detail}
    allitems = set()
    for t in tags:
        allitems.update(data[t]["items"])
        for k, v in data[t]["detail"].items():
            details[k][t] = v
    seq = {k: [data[t]["items"].get(k, MISSING) for t in tags] for k in sorted(allitems)}
    branch = lambda t: t.split(".")[0]

    # ------------------------------------------------------------------ runs
    runs = {}
    for k, s in seq.items():
        r, st = [], 0
        for i in range(1, T + 1):
            if i == T or s[i] != s[st]:
                r.append([st, i - 1, s[st]])
                st = i
        runs[k] = r
    with gzip.open(os.path.join(outdir, "item_runs.json.gz"), "wt") as f:
        json.dump({"tags": tags, "runs": runs}, f, separators=(",", ":"), sort_keys=True)
    with open(os.path.join(outdir, "tags.tsv"), "w") as f:
        f.write("idx\ttag\tbranch\titems\n")
        for i, t in enumerate(tags):
            f.write("%d\t%s\t%s\t%d\n" % (i, t, branch(t), len(data[t]["items"])))

    # ------------------------------------------------------------------ item classification
    ctrl_names_code = set(served["ctrl_names"])
    src = served["ctrl_id_sources"]

    def ctrl_sets(item):
        i = item.split(":", 1)[1]
        s = set()
        srcs = set(src.get(i, []))
        names = set()
        for d in details.get("ctrl:" + i, {}).values():
            names.update(d.get("names", []))
        if "code" in srcs or names & ctrl_names_code:
            s.add("code")
        if "extra_ids.txt" in srcs:
            s.add("matrix")
        if "windows_ids.txt" in srcs:
            s.add("windows")
        return s

    def const_everywhere(k):
        return len(runs[k]) == 1

    # classes: class names kayfabe derives sets for (Kind patterns of kf-chip/classes.rs) or names it cites
    kind_re = re.compile(r"(GPFIFO|COMPUTE|DMA_COPY|USERMODE|TWOD|INLINE_TO_MEMORY|VIDEO_(ENCODER|DECODER|OFA)|"
                         r"CHANNEL_GROUP|VASPACE|CONTEXT_SHARE|DEVICE_0|SUBDEVICE_0|SUBDEVICE_MASTER|"
                         r"^(TURING|AMPERE|ADA|HOPPER|BLACKWELL)_[A-Z]$|UVM_SW|DEFERRED_API)")
    class_id_name = defaultdict(set)
    for k, s in seq.items():
        if k.startswith("classdef:"):
            for v in set(s):
                if v.startswith("0x"):
                    class_id_name[int(v, 16)].add(k.split(":", 1)[1])

    def is_core_class(name):
        return bool(kind_re.search(name)) or norm_name(name) in idents

    def class_core(k):
        if k.startswith("classdef:"):
            return is_core_class(k.split(":", 1)[1])
        if k.startswith("classhdr:"):
            return any(is_core_class(n) for n in class_id_name.get(int(k.split(":", 1)[1], 16), []))
        return False

    def gsp_ref(k):
        if k.startswith("rpcfn:"):
            return norm_name(k.split(":", 2)[2]) in idents
        if k.startswith("gsp:"):
            nm = k.split(":", 3)[3]
            return norm_name(nm) in idents
        return False

    def family_of_chip(c):
        m = re.match(r"(TU|GA|AD|GH|GB|GR|GP|GV|GM)", c)
        return {"TU": "Turing", "GA": "Ampere", "AD": "Ada", "GH": "Hopper", "GB": "Blackwell", "GR": "Blackwell",
                "GP": "Pascal", "GV": "Volta", "GM": "Maxwell"}.get(m.group(1)) if m else None

    areas = {}  # name -> list of items

    def add(name, pred):
        areas[name] = [k for k in seq if pred(k) and not const_everywhere_all(k)]

    def const_everywhere_all(k):
        return len(set(seq[k])) == 1

    add("controls_served", lambda k: k.startswith("ctrl:") and bool(ctrl_sets(k) & {"code", "matrix"}))
    add("controls_windows", lambda k: k.startswith("ctrl:") and "windows" in ctrl_sets(k))
    add("controls_all_union", lambda k: k.startswith("ctrl:"))
    add("controls_all_flags", lambda k: k.startswith("ctrlall:"))
    add("alloc_structs", lambda k: k.startswith("allocstruct:"))
    # every class named in a chip list is in kayfabe's generated class tables, so "core" == all (checked: identical sets)
    add("classes", lambda k: k.split(":")[0] in ("classdef", "classhdr"))
    add("chip_lists_arch", lambda k: k.split(":")[0] in ("chipclasses", "mcarch"))
    add("gsp_rpc_enum", lambda k: k.startswith("rpcfn:"))
    add("gsp_interface_all", lambda k: k.startswith("gsp:") or k.startswith("rpcfn:"))
    add("gsp_interface_referenced", lambda k: (k.startswith("gsp:") or k.startswith("rpcfn:")) and gsp_ref(k))
    # served-but-not-in-ogkm (constant ABSENT) controls, reported separately
    absent_all = sorted(k[5:] for k in seq if k.startswith("ctrl:") and set(seq[k]) <= {"ABSENT", MISSING})
    fam_chip = defaultdict(list)
    for k in seq:
        if k.startswith("chipclasses:"):
            f = family_of_chip(k.split(":")[1].replace("hal", ""))
            if f:
                fam_chip[f].append(k)

    # ------------------------------------------------------------------ change kinds
    def ex_view(k, ti):
        """(present, struct-ish signature, vis) of a ctrl item at tag index ti, from the stored detail."""
        d = details.get(k, {}).get(tags[ti])
        if not d:
            return None
        ex = d.get("exports") or []
        if ex:
            return (tuple(sorted((e["cls"], e["size"], e["shash"]) for e in ex)),
                    tuple(sorted((e["cls"], e["priv"]) for e in ex)))
        h = d.get("header_only") or {}
        return (("HDR", h.get("shash")),), ()

    def change_kind(k, st):
        a, b = seq[k][st - 1], seq[k][st]
        if k.startswith("ctrl:"):
            va, vb = ex_view(k, st - 1), ex_view(k, st)
            if va is None and vb is not None:
                return "ADD"
            if va is not None and vb is None:
                return "REMOVE"
            if va and vb:
                if va[0] != vb[0]:
                    return "STRUCT"
                if va[1] != vb[1]:
                    return "VIS"
            return "OTHER"
        if a in (MISSING, "ABSENT", "NOHDR", "NODEF") and b not in (MISSING, "ABSENT", "NOHDR", "NODEF"):
            return "ADD"
        if b in (MISSING, "ABSENT", "NOHDR", "NODEF") and a not in (MISSING, "ABSENT", "NOHDR", "NODEF"):
            return "REMOVE"
        return "CHANGE"

    # ------------------------------------------------------------------ change points and intervals
    def sig_at(items, i):
        return h12("|".join(seq[k][i] for k in items))

    summary = {"tags": tags, "n_tags": T, "areas": {}}
    branches = []
    for t in tags:
        if not branches or branches[-1] != branch(t):
            branches.append(branch(t))
    summary["branches"] = branches
    area_bounds = {}
    for a, items in areas.items():
        change = defaultdict(list)  # tag idx -> items changed
        for k in items:
            for st, _en, _s in runs[k][1:]:
                change[st].append(k)
        cps = sorted(change)
        area_bounds[a] = cps
        # intervals
        starts = [0] + cps
        ivs = []
        for n, st in enumerate(starts):
            en = (starts[n + 1] - 1) if n + 1 < len(starts) else T - 1
            ivs.append((st, en))
        with open(os.path.join(outdir, "changepoints_%s.tsv" % a), "w") as f:
            f.write("tag_idx\tprev_tag\ttag\tn_changed\tchanged_items\n")
            for st in cps:
                its = sorted(change[st])
                f.write("%d\t%s\t%s\t%d\t%s\n" % (st, tags[st - 1], tags[st], len(its), " ".join(i for i in its)))
        with open(os.path.join(outdir, "intervals_%s.tsv" % a), "w") as f:
            f.write("interval\tfirst_tag\tlast_tag\tn_tags\tbranches\tchanged_at_start\tstart_sample\n")
            for n, (st, en) in enumerate(ivs):
                ch = sorted(change.get(st, []))
                f.write("%d\t%s\t%s\t%d\t%s\t%d\t%s\n" % (
                    n, tags[st], tags[en], en - st + 1,
                    branch(tags[st]) if branch(tags[st]) == branch(tags[en]) else branch(tags[st]) + "-" + branch(tags[en]),
                    len(ch), " ".join(ch[:6])))
        kinds_at = {st: Counter(change_kind(k, st) for k in change[st]) for st in cps}
        material = [st for st in cps if any(kk != "ADD" for kk in kinds_at[st])]
        summary["areas"][a] = {"items_varying": len(items), "change_points": len(cps), "intervals": len(ivs),
                               "change_points_excluding_add_only": len(material),
                               "intervals_excluding_add_only": len(material) + 1,
                               "change_kinds_total": dict(sum(kinds_at.values(), Counter()))}
        area_bounds[a] = (cps, ivs, change, kinds_at)

    # ------------------------------------------------------------------ the rule
    viol_rows = []
    rule = {}
    for a, items in areas.items():
        bad = []
        events = 0
        btags = 0
        for k in items:
            r = runs[k]
            first_of = {}
            ev = []
            for n, (st, en, s) in enumerate(r):
                if s in first_of:
                    p = first_of[s]
                    ev.append((p, n))
                else:
                    first_of[s] = n
            # pairs of runs with equal sig, with at least one different run in between: every later run of a
            # sig already seen is a re-entry (the run after the first appearance is by construction different)
            if ev:
                last_seen = {}
                evs = []
                for n, (st, en, s) in enumerate(r):
                    if s in last_seen:
                        p = last_seen[s]
                        evs.append((p, n))
                    last_seen[s] = n
                bt = 0
                for p, n in evs:
                    bt += sum(rr[1] - rr[0] + 1 for rr in r[p + 1:n])
                events += len(evs)
                btags += bt
                p, n = evs[0]
                bad.append({
                    "area": a, "item": k, "runs": len(r), "distinct_sigs": len(set(x[2] for x in r)),
                    "reentries": len(evs), "violating_B_tags": bt,
                    "A": tags[r[p][1]], "B": tags[r[p + 1][0]], "C": tags[r[n][0]],
                    "sigA": r[p][2], "sigB": r[p + 1][2],
                })
        rule[a] = {"items": len(items), "items_with_violation": len(bad), "reentry_events": events,
                   "violating_B_tags_total": btags}
        viol_rows += bad
    with open(os.path.join(outdir, "rule_violations.tsv"), "w") as f:
        cols = ["area", "item", "runs", "distinct_sigs", "reentries", "violating_B_tags", "A", "B", "C", "sigA", "sigB"]
        f.write("\t".join(cols) + "\n")
        seen = set()
        for r in viol_rows:
            f.write("\t".join(str(r[c]) for c in cols) + "\n")
    json.dump(rule, open(os.path.join(outdir, "rule_summary.json"), "w"), indent=1, sort_keys=True)
    summary["rule"] = rule
    viol_items = defaultdict(set)
    for r in viol_rows:
        viol_items[r["area"]].add(r["item"])
    summary["violation_examples"] = {a: sorted(v)[:12] for a, v in viol_items.items()}

    # ------------------------------------------------------------------ F-4 and 580 -> 595
    f4ids = ["0x2080852e", "0x2080852f", "0x2080853a", "0x2080853b", "0x2080a618", "0x2080b201"]
    ref = {}
    for nm in ["0x20800a59", "0x20808162", "0x20800102", "0x20800a3f", "0x20800b03", "0x20802806", "0x20809009",
               "0x20809001", "0x20809064", "0x2080a084", "0x2080a001", "0x20802209"]:
        ref[nm] = {"sig_tags_sample": {t: seq["ctrl:" + nm][tidx[t]] if "ctrl:" + nm in seq else "?" for t in
                                       ("580.65.06", "580.159.04", "595.44.02", "595.84", "595.91.07", "610.43.02")}}
    f4 = {"f4_controls": {}, "ref_controls": ref}
    for c in f4ids:
        k = "ctrl:" + c
        f4["f4_controls"][c] = {"sig_by_tag_distinct": sorted(set(seq.get(k, ["?"]))),
                                "defined_in_any_tag": any(v not in ("ABSENT", MISSING) for v in seq.get(k, []))}
    # does the literal id appear in any tag's header/export text?  (detail only exists when defined)
    a580 = [t for t in tags if branch(t) == "580"]
    a595 = [t for t in tags if branch(t) == "595"]
    t580, t595 = "580.159.04", "595.91.07"
    diff_580_595 = defaultdict(list)
    for k, s in seq.items():
        if s[tidx[t580]] != s[tidx[t595]]:
            kind = k.split(":")[0]
            if kind in ("ctrl", "ctrlall", "allocstruct", "classdef", "classhdr", "chipclasses", "mcarch", "rpcfn", "gsp"):
                diff_580_595[kind].append(k)
    f4["diff_580_159_04_vs_595_91_07"] = {kd: len(v) for kd, v in diff_580_595.items()}
    f4["diff_items_ctrl"] = sorted(diff_580_595.get("ctrl", []))
    f4["diff_items_allocstruct"] = sorted(diff_580_595.get("allocstruct", []))
    f4["diff_items_rpcfn"] = sorted(diff_580_595.get("rpcfn", []))
    f4["diff_items_gsp_n"] = len(diff_580_595.get("gsp", []))
    f4["diff_items_gsp_referenced"] = sorted(k for k in diff_580_595.get("gsp", []) if gsp_ref(k))
    f4["diff_items_classdef_core"] = sorted(k for k in diff_580_595.get("classdef", []) if class_core(k))
    # whole-branch comparison: first and last tag of 580 vs first and last of 595
    def branch_changes(b1, b2):
        r = {}
        for k, s in seq.items():
            v1 = {s[tidx[t]] for t in tags if branch(t) == b1}
            v2 = {s[tidx[t]] for t in tags if branch(t) == b2}
            if not (v1 & v2):
                r[k] = (sorted(v1), sorted(v2))
        return r
    bc = branch_changes("580", "595")
    f4["branch580_vs_595_disjoint_sigs"] = {kd: sum(1 for k in bc if k.split(":")[0] == kd) for kd in
                                            sorted({k.split(":")[0] for k in bc})}
    f4["branch580_vs_595_ctrl_items"] = sorted(k for k in bc if k.startswith("ctrl:"))
    json.dump(f4, open(os.path.join(outdir, "f4.json"), "w"), indent=1, sort_keys=True)
    summary["f4"] = f4

    # ------------------------------------------------------------------ Windows request sizes vs ogkm
    def sizes(cell):
        if cell in ("-", ""):
            return None
        return frozenset(int(x.split(":")[0]) for x in cell.split(","))
    rows = list(csv.DictReader(open(wcp), delimiter="\t"))
    xs = []
    c580 = ("ga106_580.88_C", "ga106_580.88_E")
    c595 = ("ga104_596.51_C", "ga104_596.51_E")
    for r in rows:
        s580 = set()
        s595 = set()
        for c in c580:
            z = sizes(r[c])
            if z:
                s580 |= z
        for c in c595:
            z = sizes(r[c])
            if z:
                s595 |= z
        if not s580 or not s595:
            continue
        i = int(r["cmd"], 16)
        k = "ctrl:0x%08x" % i
        o580 = seq[k][tidx["580.65.06"]] if k in seq else "?"
        o595 = seq[k][tidx["595.84"]] if k in seq else "?"
        xs.append({"cmd": r["cmd"], "name": r["name"], "ogkm_census": r["ogkm"],
                   "sizes_580.88_GA106": sorted(s580), "sizes_596.51_GA104": sorted(s595),
                   "size_changed": s580 != s595, "ogkm_sig_580.65.06": o580, "ogkm_sig_595.84": o595,
                   "ogkm_changed": o580 != o595})
    with open(os.path.join(outdir, "size_crosscheck.tsv"), "w") as f:
        cols = list(xs[0].keys())
        f.write("\t".join(cols) + "\n")
        for x in xs:
            f.write("\t".join(str(x[c]).replace(" ", "") for c in cols) + "\n")
    cont = Counter()
    for x in xs:
        defined = x["ogkm_sig_595.84"] not in ("ABSENT", "?") and x["ogkm_sig_580.65.06"] not in ("ABSENT", "?")
        cont[("defined_in_ogkm" if defined else "not_in_ogkm", "size_changed" if x["size_changed"] else "size_same",
              "ogkm_changed" if x["ogkm_changed"] else "ogkm_same")] += 1
    summary["size_crosscheck"] = {"/".join(k): v for k, v in sorted(cont.items())}
    summary["size_crosscheck_changed_not_in_ogkm"] = sorted(x["cmd"] for x in xs if x["size_changed"] and
                                                           x["ogkm_sig_595.84"] == "ABSENT")
    summary["size_crosscheck_changed_defined"] = sorted(
        [(x["cmd"], x["name"], x["ogkm_changed"]) for x in xs if x["size_changed"] and x["ogkm_sig_595.84"] != "ABSENT"])

    # ------------------------------------------------------------------ measurement plan
    mp = list(csv.DictReader(open(mpp), delimiter="\t"))
    layout_tags = [l.strip() for l in open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "drivermatrix", "tags.txt"))
                   if l.strip() and not l.startswith("#")]
    plan_axes = {
        "host": ["controls_served", "alloc_structs", "classes"],
        "winguest": ["controls_windows", "alloc_structs", "classes", "gsp_interface_referenced"],
    }
    summary["plan"] = {}
    for variant, axis, alist in [(v, ax, al) for v in ("mono", "all") for ax, al in plan_axes.items()]:
        axn = axis if variant == "mono" else axis + "_all_items"
        # items that fail the rule (non-monotone) are NOT used to place boundaries: the rule cannot interpolate
        # them, so they need a per-tag measurement of their own (rule_violations.tsv)
        bad_items = {r["item"] for r in viol_rows} if variant == "mono" else set()
        change = defaultdict(list)
        for a in alist:
            for st, its in area_bounds[a][2].items():
                for k in its:
                    if k not in bad_items:
                        change[st].append(k)
        cps = sorted(change)
        starts = [0] + cps
        ivs_full = [(st, (starts[n + 1] - 1) if n + 1 < len(starts) else T - 1) for n, st in enumerate(starts)]
        lo = tidx[WINDOW_START]
        ivs = [(max(st, lo), en) for st, en in ivs_full if en >= lo]
        # family-specific boundaries: a chip class list / arch id of that family changes at the interval start
        fam_change = defaultdict(set)
        for k in areas["chip_lists_arch"]:
            if k.startswith("chipclasses:"):
                f_ = family_of_chip(k.split(":")[1].replace("hal", ""))
            else:
                f_ = family_of_chip(k.split(":")[1].split("_")[-1]) if k.startswith("mcarch:IMPLEMENTATION_") else None
            if f_:
                for st, _e, _s in runs[k][1:]:
                    fam_change[st].add(f_)
        pts = [p for p in mp if p["axis"] == axis]
        for p in pts:
            p["_idx"] = tidx[p["tag_key"]]
        rows_out = []
        for n, (st, en) in enumerate(ivs):
            cov = [p for p in pts if st <= p["_idx"] <= en]
            ch = sorted(change.get(st, []))
            cats = Counter(k.split(":")[0] if not k.startswith("gsp:") else "gsp" for k in ch)
            kinds = Counter()
            for a in alist:
                if st in area_bounds[a][3]:
                    kinds.update(area_bounds[a][3][st])
            lay = [t for t in layout_tags if t in tidx and st <= tidx[t] <= en]
            famc = sorted(fam_change.get(st, set()))
            rep_pool = [t for t in lay] or [tags[en]]
            rep = rep_pool[-1]
            rows_out.append({
                "interval": n, "first_tag": tags[st], "last_tag": tags[en], "n_tags": en - st + 1,
                "branches": branch(tags[st]) if branch(tags[st]) == branch(tags[en]) else branch(tags[st]) + "-" + branch(tags[en]),
                "changed_at_start": len(ch), "start_categories": ";".join("%s=%d" % kv for kv in sorted(cats.items())),
                "start_kinds": ";".join("%s=%d" % kv for kv in sorted(kinds.items())) or "-",
                "add_only": bool(kinds) and set(kinds) == {"ADD"},
                "covered_by": ";".join(sorted({"%s@%s(%s)" % (p["driver"], p["die"], p["id"]) for p in cov})) or "-",
                "layout_only_tags": ";".join(lay) or "-",
                "family_items_changed": ";".join(famc) or "-",
                "needs": "-" if cov else "rep=%s (interval %s..%s)" % (rep, tags[st], tags[en]),
                "rep": rep, "_st": st, "_en": en, "_pts": cov,
                "_tierA": any(change_kind(k, st) in ("STRUCT", "VIS", "REMOVE", "CHANGE") and (
                    k.startswith(("ctrl:", "allocstruct:", "rpcfn:")) or BOOT_RE.search(k)) for k in ch),
                "_material": (st == lo and n == 0) or any(
                    change_kind(k, st) != "ADD" and not k.startswith("classhdr:") for k in ch),
                "_cov": bool(cov), "_inferred_only": any(p["how"] != "exact" for p in cov) and all(p["how"] != "exact" for p in cov),
            })
        summary["plan"][axn] = {"variant": variant,
            "intervals_all_221_tags": len(ivs_full), "window_start": WINDOW_START, "intervals": len(ivs), "covered": sum(1 for r in rows_out if r["_cov"]),
            "uncovered": sum(1 for r in rows_out if not r["_cov"]),
            "covered_only_by_inferred_position": sum(1 for r in rows_out if r["_cov"] and r["_inferred_only"]),
            "intervals_starting_with_add_only_change": sum(1 for r in rows_out if r["add_only"]),
            "uncovered_excluding_add_only_starts": sum(1 for r in rows_out if not r["_cov"] and not r["add_only"]),
            "areas": alist,
        }
        with open(os.path.join(outdir, "plan_%s.tsv" % axn), "w") as f:
            cols = ["interval", "first_tag", "last_tag", "n_tags", "branches", "changed_at_start", "start_categories",
                    "start_kinds", "family_items_changed", "covered_by", "needs", "layout_only_tags"]
            f.write("\t".join(cols) + "\n")
            for r in rows_out:
                f.write("\t".join(str(r[c]) for c in cols) + "\n")
        # material groups: merge a strict interval into its predecessor when its start boundary is only
        # ADD changes and/or class-header text changes (nothing removed, no struct / privilege / id changed)
        groups = []
        for r in rows_out:
            if groups and not r["_material"]:
                groups[-1].append(r)
            else:
                groups.append([r])
        with open(os.path.join(outdir, "plan_%s_material.tsv" % axn), "w") as f:
            f.write("group\ttier\tfirst_tag\tlast_tag\tn_tags\tstrict_subintervals\tcovered_by\tneeds\tfamily_items_changed\n")
            gcov = 0
            tiers = Counter()
            for gi, g in enumerate(groups):
                st, en = g[0]["_st"], g[-1]["_en"]
                pts_ = [p_ for r in g for p_ in r["_pts"]]
                cov_ = ";".join(sorted({"%s@%s(%s)" % (p_["driver"], p_["die"], p_["id"]) for p_ in pts_})) or "-"
                gcov += bool(pts_)
                lay = [t for t in layout_tags if t in tidx and st <= tidx[t] <= en]
                rep = (lay or [tags[en]])[-1]
                fams = sorted({x for r in g for x in (r["family_items_changed"].split(";") if r["family_items_changed"] != "-" else [])})
                tier = "A" if any(r["_tierA"] for r in g) else "B"
                tiers[(tier, bool(pts_))] += 1
                f.write("%d\t%s\t%s\t%s\t%d\t%d\t%s\t%s\t%s\n" % (
                    gi, tier, tags[st], tags[en], en - st + 1, len(g), cov_,
                    "-" if pts_ else "rep=%s" % rep, ";".join(fams) or "-"))
        summary["plan"][axn]["tiers"] = {"%s_%s" % (k[0], "covered" if k[1] else "uncovered"): v for k, v in sorted(tiers.items())}
        summary["plan"][axn]["material_groups"] = len(groups)
        summary["plan"][axn]["material_groups_covered"] = gcov
        summary["plan"][axn]["material_groups_uncovered"] = len(groups) - gcov
        summary["plan"][axn]["rows"] = [{k: v for k, v in r.items() if not k.startswith("_")} | {"covered": r["_cov"], "material": r["_material"]}
                                         for r in rows_out]

    # ------------------------------------------------------------------ rule applied to the plan itself
    # Measured set = the tags of the covered measured points + the representative tag of every uncovered
    # group.  For every item of the axis' areas and every UNmeasured tag B of the window that has measured
    # neighbours A<B<C with sig(A)==sig(C), the rule infers sig(B)==sig(A).  Count the wrong inferences.
    loi = tidx[WINDOW_START]
    summary["rule_on_plan"] = {}
    for axis, alist in plan_axes.items():
        mrows = list(csv.DictReader(open(os.path.join(outdir, "plan_%s_material.tsv" % axis)), delimiter="\t"))
        pts = [p_ for p_ in mp if p_["axis"] == axis]
        meas = {tidx[p_["tag_key"]] for p_ in pts}
        for r in mrows:
            if r["needs"].startswith("rep="):
                meas.add(tidx[r["needs"][4:]])
        meas = sorted(m for m in meas if m >= loi)
        items_ax = sorted({k for a in alist for k in seq if k in set(areas[a])} |
                          {k for a in alist for k in areas[a]})
        ok = bad = unk = 0
        wrong = Counter()
        for k in items_ax:
            for b in range(loi, T):
                if b in meas:
                    continue
                lo_ = [m for m in meas if m < b]
                hi_ = [m for m in meas if m > b]
                if not lo_ or not hi_:
                    continue
                a_, c_ = lo_[-1], hi_[0]
                if seq[k][a_] != seq[k][c_]:
                    unk += 1
                elif seq[k][b] == seq[k][a_]:
                    ok += 1
                else:
                    bad += 1
                    wrong[k] += 1
        summary["rule_on_plan"][axis] = {
            "measured_tags": [tags[m] for m in meas], "items": len(items_ax), "inferences_correct": ok,
            "inferences_wrong": bad, "pairs_A_ne_C_need_measurement": unk,
            "wrong_items": len(wrong), "wrong_item_list": sorted(wrong)[:40],
            "wrong_items_in_violators": len([k for k in wrong if k in {r["item"] for r in viol_rows}])}
    # branch-level view: change points that fall INSIDE one R-branch (tag pair with the same major)
    bl = {}
    for b in branches:
        idx = [i for i, t in enumerate(tags) if branch(t) == b]
        row = {"tags": len(idx), "first": tags[idx[0]], "last": tags[idx[-1]]}
        for a in ("controls_served", "controls_windows", "alloc_structs", "classes", "gsp_interface_referenced"):
            cps = [c for c in area_bounds[a][0] if c in idx and c - 1 in idx]
            row[a] = len(cps)
        bl[b] = row
    summary["branch_internal_change_points"] = bl
    # per-family chip-list changes and support floor (first tag where a chip of the family appears)
    fam = {}
    for f_, keys in sorted(fam_chip.items()):
        first = None
        for k in keys:
            for i, v in enumerate(seq[k]):
                if v != MISSING:
                    first = i if first is None else min(first, i)
                    break
        chg = sorted({st for k in keys for st, _e, _s in runs[k][1:]})
        fam[f_] = {"chips": sorted(k.split(":")[1].replace("hal", "") for k in keys if k.endswith(":full")),
                   "first_tag_with_chip": tags[first] if first is not None else None,
                   "chip_list_change_tags": [tags[i] for i in chg]}
    summary["family_chip_lists"] = fam
    summary["absent_in_all_tags"] = absent_all
    summary["controls_resolved"] = sum(1 for k in seq if k.startswith("ctrl:") and not set(seq[k]) <= {"ABSENT", MISSING})
    summary["controls_total_ids"] = sum(1 for k in seq if k.startswith("ctrl:"))
    summary["subsets"] = {a: len(v) for a, v in areas.items()}
    json.dump(summary, open(os.path.join(outdir, "summary.json"), "w"), indent=1, sort_keys=True, default=list)
    print(json.dumps({a: summary["areas"][a] for a in summary["areas"]}, indent=0))
    print(json.dumps(rule, indent=0))
    print({ax: {k: v for k, v in summary["plan"][ax].items() if k != "rows"} for ax in summary["plan"]})


if __name__ == "__main__":
    main()
