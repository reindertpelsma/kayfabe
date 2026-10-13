#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Turn the output of ogkm_search_ids.py into a tier-1 table that `kf-census --tier1` loads.

STATUS: RESEARCH, 2026-10-13.

usage: python3 -I ogkm_tier1_tsv.py SEARCH_PREFIX IDS.txt TAGS.txt OUT.tsv CENSUS_CONTROLS.tsv...

SEARCH_PREFIX  the OUT_PREFIX given to ogkm_search_ids.py (reads .defs.tsv .hits.tsv .p2.tsv)
IDS.txt        the ids to write rows for (one 0x... per line)
TAGS.txt       the searched tags (tier_checks/ogkm_evidence/tags_searched.txt)
CENSUS_...     `kf-census --out` controls.tsv files of the captures (reply sizes only are used)

Verdicts (the census loads the first two columns; every verdict except NOT_IN_ANY_VERSION reads as "defined"):
  DEFINED_580X               a `#define NV*_CTRL_*  (0x<id>)` exists in at least one 580.x tag
  DEFINED_NOT_580X           defined in some tag(s) but in no 580.x tag
  NUMBER_ONLY                the number occurs in the tree but not as a control define (no row has this today)
  NOT_IN_ANY_VERSION         no occurrence of the number in any of the tags: P1 (any spelling, text files),
                             P2 (raw, binary files, decimal) and the define scan all found nothing
"""
import csv
import re
import sys


def vkey(t):
    return tuple(int(x) for x in t.split("."))


def expand(text, tags):
    out = set()
    ix = {t: i for i, t in enumerate(tags)}
    for part in text.split(", "):
        if ".." in part:
            a, b = part.split("..")
            out.update(tags[ix[a] : ix[b] + 1])
        elif part:
            out.add(part)
    return out


def main():
    prefix, ids_file, tags_file, out_path, *census = sys.argv[1:]
    tags = sorted((l.strip() for l in open(tags_file) if l.strip()), key=vkey)
    t580 = {t for t in tags if t.startswith("580.")}
    ids = [l.split("#")[0].strip() for l in open(ids_file)]
    ids = [i for i in ids if i]
    defs, hits = {}, {}
    for r in csv.DictReader(open(prefix + ".defs.tsv"), delimiter="\t"):
        defs.setdefault(r["id"], []).append(r)
    for r in csv.DictReader(open(prefix + ".hits.tsv"), delimiter="\t"):
        hits.setdefault(r["id"], []).append(r)
    p2 = [l for l in open(prefix + ".p2.tsv") if not l.startswith("#") and l.strip()]
    obs = {}
    for f in census:
        for r in csv.DictReader(open(f), delimiter="\t"):
            obs.setdefault(r["cmd"], set()).update(int(x.split(":")[0]) for x in r["reply_sizes"].split(";") if x != "-")
    rows = []
    for i in ids:
        d = defs.get(i, [])
        h = hits.get(i, [])
        names = []
        in580 = False
        alltags = set()
        size_notes = []
        match = "n/a"
        o = sorted(obs.get(i, []))
        for r in d:
            ts = expand(r["tags"], tags)
            alltags |= ts
            names.append(f"{r['name']} @{r['tags']} ({r['n_tags']} tags)")
            if ts & t580:
                in580 = True
                # sizes the name has at the 580.x tags
                s580 = set()
                for ent in r["size_by_tag_range"].split("; "):
                    m = re.match(r"(\?|\d+)B@(.*)$", ent)
                    if m and expand(m.group(2), tags) & t580:
                        s580.add(m.group(1))
                size_notes.append(f"{r['name']} 580.x sizeof={'/'.join(sorted(s580))}")
                if "?" in s580 and len(s580) == 1:
                    match = "NO_PARAMS_STRUCT" if o == [0] or o == [] else "UNKNOWN_SIZE"
                elif {str(x) for x in o} <= s580 and o:
                    match = "MATCH"
                elif o:
                    match = "MISMATCH"
        if d and in580:
            verdict = "DEFINED_580X"
        elif d:
            verdict = "DEFINED_NOT_580X"
        elif h:
            verdict = "NUMBER_ONLY"
        else:
            verdict = "NOT_IN_ANY_VERSION"
        collision = len({r["name"] for r in d}) > 1
        if verdict == "NOT_IN_ANY_VERSION":
            ref = ""
            why = "absent by number in every tag (P1 + P2 + define scan: 0 lines in %d tags)" % len(tags)
            note = "M: ogkm_search_ids.py over all %d tags found no occurrence of %s in any spelling (text and binary, hex and decimal)." % (len(tags), i)
            if o:
                note += " Observed reply sizes in the five captures: %s." % ",".join(map(str, o))
        else:
            first = d[0]["first_header"] if d else (h[0]["path"] if h else "")
            ref = first
            why = ""
            note = "M: " + "; ".join(names) if names else "M: number occurs only in " + ", ".join(sorted({x["path"] for x in h}))
            if size_notes:
                note += ". sizes: " + "; ".join(size_notes)
            if o:
                note += f". Observed reply sizes: {','.join(map(str, o))}"
            if collision:
                note += ". NUMBER COLLISION: the number is a different control in different tags (names above)."
            note += ". Size equality only; field offsets and values not compared."
        rows.append(
            [
                "ctl-" + i[2:].lower(),
                verdict,
                "; ".join(names) if names else "none",
                match,
                ref,
                why,
                note,
            ]
        )
    with open(out_path, "w") as f:
        f.write("id\tverdict\tversions_defined\tlayout_matches_observed\tsource_ref\treason_if_not\tnotes\n")
        for r in rows:
            f.write("\t".join(x.replace("\t", " ").replace("\n", " ") for x in r) + "\n")
    c = {}
    for r in rows:
        c[r[1]] = c.get(r[1], 0) + 1
    print(len(rows), "rows;", c, "; P2 hits file lines:", len(p2), file=sys.stderr)


if __name__ == "__main__":
    main()
