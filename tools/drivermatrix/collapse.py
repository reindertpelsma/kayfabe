#!/usr/bin/env python3
"""collapse.py — turn a `dm.py sweep` (one directory per ogkm tag) into VERSION RANGES.

Every measured item — a struct's whole layout, one field of it, a macro value, an enumerator —
is reduced to the runs of consecutive tags (in version order) over which it is identical. A run
boundary is a MEASURED change between two adjacent tags; nothing is interpolated.

  collapse.py ranges  --sweep DIR --out ranges.tsv [--only consumed.txt]
      one row per (item, run): kind item first_tag last_tag value
      `--only` keeps the items a consumption list names (struct names, `struct.field` paths,
      value names, or `entry:*`); this is what gets committed.
  collapse.py report  --sweep DIR --out report.md [--only consumed.txt]
      per struct: every boundary, with the fields added / removed / moved there.
  collapse.py boundaries --sweep DIR
      per tag transition: how many measured items change — the one-screen gap picture.
  collapse.py gaps --sweep DIR --ref TAG [--only consumed.txt]
      per tag: which CONSUMED structs and values differ from the reference tag (the version
      kayfabe's hand encoders were written against) — the per-version work list.

Tags are ordered numerically (535.309.01 < 545.23.08 < … < 610.57.04), never lexically.
"""
import argparse
import collections
import os
import sys


def vkey(tag):
    return tuple(int(x) for x in tag.split("."))


def load(sweep):
    tags = sorted((t for t in os.listdir(sweep) if os.path.isfile(os.path.join(sweep, t, ".done"))),
                  key=vkey)
    lay = {}   # tag -> struct -> [(path, off, size, type)]
    val = {}   # tag -> name -> value
    mis = {}   # tag -> set(names)
    for t in tags:
        lay[t], val[t], mis[t] = {}, {}, set()
        for axis in sorted(os.listdir(os.path.join(sweep, t))):
            d = os.path.join(sweep, t, axis)
            if not os.path.isdir(d):
                continue
            with open(os.path.join(d, "layouts.tsv")) as f:
                for line in f:
                    s, p, off, size, tn = line.rstrip("\n").split("\t")
                    lay[t].setdefault(s, []).append((p, int(off), int(size), tn))
            with open(os.path.join(d, "values.tsv")) as f:
                for line in f:
                    e, n, v = line.rstrip("\n").split("\t")
                    val[t][f"{e}:{n}"] = v
            with open(os.path.join(d, "missing.tsv")) as f:
                for line in f:
                    mis[t].add(line.split("\t", 1)[0])
    return tags, lay, val, mis


def runs(tags, get):
    """[(first, last, value)] over `tags` for the value function `get(tag)` (None = absent)."""
    out = []
    for t in tags:
        v = get(t)
        if out and out[-1][2] == v:
            out[-1][1] = t
        else:
            out.append([t, t, v])
    return out


def struct_fp(rows):
    return ";".join(f"{p}@{o}+{s}" for (p, o, s, _t) in rows)


def fields(rows):
    """path -> (off, size) — the first row per path (the struct's own `.` row is its sizeof)."""
    d = {}
    for (p, o, s, _t) in rows:
        d.setdefault(p, (o, s))
    return d


def elems(rows):
    """path -> array element size (1-D arrays only; 0 = not an array, -1 = multi-dimensional)."""
    d = {}
    for (p, _o, _s, t) in rows:
        if p in d:
            continue
        if t.startswith("array/"):
            spec = t[len("array/"):]
            d[p] = -1 if "x" in spec else int(spec or 0)
        else:
            d[p] = 0
    return d


def read_only(path):
    if not path:
        return None
    keep = set()
    with open(path) as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if line:
                keep.add(line)
    return keep


def wanted(keep, name):
    if keep is None:
        return True
    if name in keep:
        return True
    head = name.split(".", 1)[0].split(":", 1)[0]
    return f"{head}:*" in keep or f"{head}.*" in keep


def cmd_ranges(a):
    tags, lay, val, _mis = load(a.sweep)
    keep = read_only(a.only)
    structs = sorted({s for t in tags for s in lay[t]})
    names = sorted({n for t in tags for n in val[t]})
    with open(a.out, "w") as o:
        o.write("# kind\titem\tfirst_tag\tlast_tag\tvalue    (measured by tools/drivermatrix/dm.py; "
                f"tags {tags[0]}..{tags[-1]}, {len(tags)} of them)\n")
        o.write("# tags\t" + " ".join(tags) + "\n")
        for s in structs:
            per = {t: fields(lay[t][s]) if s in lay[t] else None for t in tags}
            el = {t: elems(lay[t][s]) if s in lay[t] else None for t in tags}
            paths = sorted({p for t in tags if per[t] for p in per[t]})
            for p in paths:
                item = s if p == "." else f"{s}.{p}"
                if not wanted(keep, item) and not wanted(keep, s):
                    continue

                def val(t, p=p):
                    if not per[t] or p not in per[t]:
                        return None
                    e = el[t].get(p, 0)
                    return per[t][p] + ((e,) if e else ())

                for (f0, f1, v) in runs(tags, val):
                    if v is None:
                        vv = "ABSENT"
                    elif len(v) == 3:
                        # `@ELEM` marks a 1-D array of ELEM-byte elements; `@-1` a multi-dim one.
                        vv = f"{v[0]}+{v[1]}@{v[2]}"
                    else:
                        vv = f"{v[0]}+{v[1]}"
                    o.write(f"{'sizeof' if p == '.' else 'field'}\t{item}\t{f0}\t{f1}\t{vv}\n")
        for n in names:
            if not wanted(keep, n):
                continue
            for (f0, f1, v) in runs(tags, lambda t: val[t].get(n)):
                o.write(f"value\t{n}\t{f0}\t{f1}\t{'ABSENT' if v is None else v}\n")


def cmd_report(a):
    tags, lay, val, mis = load(a.sweep)
    keep = read_only(a.only)
    structs = sorted({s for t in tags for s in lay[t]})
    out = [f"# ogkm layout boundaries, {tags[0]} … {tags[-1]} ({len(tags)} tags)\n",
           "Generated by `tools/drivermatrix/collapse.py report`. A boundary is a measured "
           "difference between two ADJACENT measured tags; the fields listed are the ones that "
           "appear, disappear or move there. `+N` is a size, `@N` an offset.\n"]
    for s in structs:
        if not wanted(keep, s):
            continue
        fp = runs(tags, lambda t: struct_fp(lay[t][s]) if s in lay[t] else None)
        if len(fp) == 1:
            continue
        out.append(f"\n## `{s}` — {len(fp)} distinct layouts\n")
        prev = None
        for (f0, f1, v) in fp:
            span = f0 if f0 == f1 else f"{f0} … {f1}"
            if v is None:
                out.append(f"- **{span}**: absent\n")
                prev = None
                continue
            cur = fields(lay[f0][s])
            size = cur["."][1]
            if prev is None:
                out.append(f"- **{span}**: sizeof {size}\n")
            else:
                added = sorted(p for p in cur if p not in prev)
                gone = sorted(p for p in prev if p not in cur)
                moved = sorted(p for p in cur if p in prev and p != "." and cur[p] != prev[p])
                bits = [f"sizeof {prev['.'][1]} → {size}"]
                if added:
                    bits.append("added " + ", ".join(f"`{p}`" for p in added[:12])
                                + (f" (+{len(added) - 12} more)" if len(added) > 12 else ""))
                if gone:
                    bits.append("removed " + ", ".join(f"`{p}`" for p in gone[:12])
                                + (f" (+{len(gone) - 12} more)" if len(gone) > 12 else ""))
                if moved:
                    bits.append(f"{len(moved)} field(s) moved, first `{moved[0]}` "
                                f"{prev[moved[0]][0]}→{cur[moved[0]][0]}")
                out.append(f"- **{span}**: " + "; ".join(bits) + "\n")
            prev = cur
    with open(a.out, "w") as o:
        o.write("".join(out))


def cmd_boundaries(a):
    tags, lay, val, _mis = load(a.sweep)
    keep = read_only(a.only)
    for i in range(1, len(tags)):
        t0, t1 = tags[i - 1], tags[i]
        ss = sorted(set(lay[t0]) | set(lay[t1]))
        ch = [s for s in ss if wanted(keep, s)
              and (struct_fp(lay[t0].get(s, [])) != struct_fp(lay[t1].get(s, [])))]
        vs = sorted(set(val[t0]) | set(val[t1]))
        vch = [n for n in vs if wanted(keep, n) and val[t0].get(n) != val[t1].get(n)]
        print(f"{t0:>12} → {t1:<12} structs changed {len(ch):5d}   values changed {len(vch):5d}   "
              + " ".join(ch[:6]) + (" …" if len(ch) > 6 else ""))


def cmd_gaps(a):
    tags, lay, val, _mis = load(a.sweep)
    keep = read_only(a.only)
    ref = a.ref
    if ref not in lay:
        raise SystemExit(f"reference tag {ref} was not measured in {a.sweep}")
    structs = sorted(s for s in {s for t in tags for s in lay[t]} if wanted(keep, s))
    names = sorted(n for n in {n for t in tags for n in val[t]} if wanted(keep, n))
    for t in tags:
        diff_s = [s for s in structs if struct_fp(lay[t].get(s, [])) != struct_fp(lay[ref].get(s, []))]
        diff_v = [n for n in names if val[t].get(n) != val[ref].get(n)]
        print(f"{t}\tstructs {len(diff_s)}\tvalues {len(diff_v)}\t" + " ".join(diff_s))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    for n in ("ranges", "report", "boundaries", "gaps"):
        p = sub.add_parser(n)
        p.add_argument("--sweep", required=True)
        p.add_argument("--only")
        if n in ("ranges", "report"):
            p.add_argument("--out", required=True)
        if n == "gaps":
            p.add_argument("--ref", required=True)
    a = ap.parse_args()
    {"ranges": cmd_ranges, "report": cmd_report, "boundaries": cmd_boundaries,
     "gaps": cmd_gaps}[a.cmd](a)


if __name__ == "__main__":
    main()
