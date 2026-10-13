#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tier-1 search of a list of RM control ids over EVERY ogkm tag (the method of tier1_ogkm.md, scripted).

STATUS: RESEARCH, 2026-10-13. Read-only: `git grep` / `git archive` on a bare clone of
https://github.com/NVIDIA/open-gpu-kernel-modules, `gcc -S` on extracted headers. No GPU, no network after the clone.

usage: ogkm_search_ids.py IDS.txt OGKM_BARE_REPO OUT_PREFIX [--jobs N]

IDS.txt: one control id per line, hex (0x00730101). Writes, for all tags (`git tag`, sorted by version):

  OUT_PREFIX.defs.tsv   every id with every `#define NV*_CTRL_*CMD* (0x<id>)` found (P3 define scan, all .h files),
                        as: id, name, first_tag, last_tag, n_tags, header (path at first_tag), struct, sizes
  OUT_PREFIX.hits.tsv   P1 numeric scan (any spelling, whole tree incl. generated code), non-define hits grouped by
                        (id, path, first_tag, last_tag, n_tags, example line)
  OUT_PREFIX.p2.tsv     P2 raw scan (-a, hex and decimal) for ids with no P1 hit anywhere
  OUT_PREFIX.summary.tsv  one row per id (the result)

Three independent searches are combined on purpose: the define scan (by name and value, so it also finds a number
defined with another spelling), P1 (by number, any file) and P2 (by number, binary files and decimal).
"""
import collections
import concurrent.futures as cf
import os
import re
import subprocess
import sys
import tempfile


def run(cmd, cwd=None, check=True):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, errors="replace")
    if check and r.returncode not in (0, 1):
        raise RuntimeError(f"{cmd}: {r.stderr[:300]}")
    return r.stdout


def vkey(t):
    return tuple(int(x) for x in t.split("."))


def ranges(tags_present, all_tags):
    """Collapse a set of tags into 'a..b' ranges over the ordered tag list."""
    idx = {t: i for i, t in enumerate(all_tags)}
    pos = sorted(idx[t] for t in tags_present)
    out, start, prev = [], None, None
    for p in pos:
        if start is None:
            start = prev = p
        elif p == prev + 1:
            prev = p
        else:
            out.append((start, prev))
            start = prev = p
    if start is not None:
        out.append((start, prev))
    return ", ".join(all_tags[a] if a == b else f"{all_tags[a]}..{all_tags[b]}" for a, b in out)


DEF_RE = re.compile(
    r"^(?P<tag>[^:]+):(?P<path>[^:]+):(?P<line>\d+):\s*#\s*define\s+(?P<name>NV[0-9A-Za-z]*_CTRL_[A-Za-z0-9_]*CMD[A-Za-z0-9_]*)"
    r"\s+\(?\s*\(?\s*(?P<val>0[xX][0-9a-fA-F]+)[uUlL]*\s*\)?"
)
FINN_RE = re.compile(r'finn: Evaluated from "[^"]*?\|\s*(?P<msg>[A-Z0-9_]+)_MESSAGE_ID"')


def main():
    ids_file, repo, prefix = sys.argv[1:4]
    jobs = 4
    if "--jobs" in sys.argv:
        jobs = int(sys.argv[sys.argv.index("--jobs") + 1])
    ids = []
    for l in open(ids_file):
        l = l.split("#")[0].strip()
        if l:
            ids.append(int(l, 16))
    ids = sorted(set(ids))
    tags = sorted(run(["git", "-C", repo, "tag"]).split(), key=vkey)
    print(f"{len(ids)} ids, {len(tags)} tags ({tags[0]}..{tags[-1]})", file=sys.stderr)
    hexes = "|".join(f"{i:x}" for i in ids)
    p1 = f"0x0*({hexes})([^0-9a-fA-F]|$)"

    def scan(tag):
        # P3: all define lines naming a CTRL command, any header, any directory
        d = run(["git", "-C", repo, "grep", "-n", "-I", "-E", r"^\s*#\s*define\s+NV[0-9A-Za-z]*_CTRL_[A-Za-z0-9_]*CMD", tag, "--", "*.h"])
        defs = []
        non_literal = 0
        for line in d.splitlines():
            m = DEF_RE.match(line)
            if m:
                v = int(m["val"], 16)
                if v in ids_set:
                    defs.append((v, m["name"], m["path"], int(m["line"]), line))
            else:
                non_literal += 1
        # P1: numeric, any spelling, whole tree
        h = run(["git", "-C", repo, "grep", "-n", "-I", "-i", "-E", p1, tag])
        hits = []
        for line in h.splitlines():
            t, path, ln, text = line.split(":", 3)
            mm = re.search(p1, text, re.I)
            hits.append((int(mm.group(1), 16) if mm else -1, path, int(ln), text.strip()[:160]))
        return tag, defs, hits, non_literal

    ids_set = set(ids)
    results = {}
    with cf.ThreadPoolExecutor(jobs) as ex:
        for i, (tag, defs, hits, nl) in enumerate(ex.map(scan, tags)):
            results[tag] = (defs, hits, nl)
            if i % 20 == 0:
                print(f"  scanned {i + 1}/{len(tags)}", file=sys.stderr)

    # ---- definitions
    by_def = collections.defaultdict(lambda: collections.defaultdict(set))  # id -> name -> tags
    def_loc = {}
    for tag in tags:
        for v, name, path, ln, _ in results[tag][0]:
            by_def[v][name].add(tag)
            def_loc.setdefault((v, name), (tag, path, ln))
    # ---- non-define numeric hits
    by_hit = collections.defaultdict(lambda: collections.defaultdict(set))  # id -> path -> tags
    ex_line = {}
    for tag in tags:
        for v, path, ln, text in results[tag][1]:
            if DEF_RE.match(f"{tag}:{path}:{ln}:{text}"):
                continue
            by_hit[v][path].add(tag)
            ex_line.setdefault((v, path), (tag, ln, text))

    # ---- struct names and sizes
    struct_of = {}   # (id, name) -> struct name at first tag (finn comment or naming convention)
    for (v, name), (tag, path, ln) in def_loc.items():
        src = run(["git", "-C", repo, "show", f"{tag}:{path}"])
        lines = src.splitlines()
        text = lines[ln - 1]
        m = FINN_RE.search(text)
        if m:
            struct_of[(v, name)] = m["msg"]
        else:
            struct_of[(v, name)] = name.replace("_CTRL_CMD_", "_CTRL_", 1) + "_PARAMS"
    sizes = collections.defaultdict(dict)   # (id,name) -> tag -> size|None
    todo = collections.defaultdict(list)    # tag -> [(id,name)]
    for (v, name), ts in ((k, by_def[k[0]][k[1]]) for k in def_loc):
        for t in ts:
            todo[t].append((v, name))

    def size_tag(tag):
        out = {}
        with tempfile.TemporaryDirectory(dir="/dev/shm") as td:
            tar = subprocess.run(["git", "-C", repo, "archive", tag, "src/common/sdk/nvidia/inc"], capture_output=True)
            subprocess.run(["tar", "-x", "-C", td], input=tar.stdout, check=True)
            inc = os.path.join(td, "src/common/sdk/nvidia/inc")
            for (v, name) in todo[tag]:
                _, path, _ = def_loc[(v, name)]
                # the header the define lives in at THIS tag (the path can move): find by basename
                base = os.path.basename(path)
                hdr = None
                for root, _, files in os.walk(inc):
                    if base in files:
                        hdr = os.path.relpath(os.path.join(root, base), inc)
                        break
                st = struct_of[(v, name)]
                if hdr is None:
                    out[(v, name)] = None
                    continue
                c = f'#include "{hdr}"\nchar kf_sz[sizeof(struct {st})];\n'
                alt = f'#include "{hdr}"\nchar kf_sz[sizeof({st})];\n'
                size = None
                for src in (alt, c):
                    r = subprocess.run(["gcc", "-S", "-o", "-", "-x", "c", "-I", inc, "-w", "-"], input=src, capture_output=True, text=True)
                    if r.returncode == 0:
                        m = re.search(r"\.(?:comm|lcomm)\s+kf_sz,(\d+)", r.stdout) or re.search(r"\.size\s+kf_sz,\s*(\d+)", r.stdout)
                        if m:
                            size = int(m.group(1))
                            break
                out[(v, name)] = size
        return tag, out

    with cf.ThreadPoolExecutor(jobs) as ex:
        for i, (tag, out) in enumerate(ex.map(size_tag, [t for t in tags if todo.get(t)])):
            for k, s in out.items():
                sizes[k][tag] = s
            if i % 20 == 0:
                print(f"  sized {i + 1}", file=sys.stderr)

    # ---- P2 for ids with no P1 hit and no define
    nohit = [v for v in ids if v not in by_def and v not in by_hit]
    p2rows = []
    if nohit:
        pats = "|".join(f"{v:x}|{v}" for v in nohit)
        p2 = f"(^|[^0-9a-fA-F])0*({pats})([^0-9a-fA-F]|$)"

        def scan2(tag):
            return tag, run(["git", "-C", repo, "grep", "-n", "-a", "-i", "-E", p2, tag]).splitlines()

        with cf.ThreadPoolExecutor(jobs) as ex:
            for tag, lines in ex.map(scan2, tags):
                for l in lines:
                    p2rows.append(l[:200])

    # ---- write
    with open(prefix + ".defs.tsv", "w") as f:
        f.write("id\tname\ttags\tn_tags\tfirst_header\tstruct\tsize_by_tag_range\n")
        for v in ids:
            for name in sorted(by_def.get(v, {})):
                ts = by_def[v][name]
                sz = sizes[(v, name)]
                grp = collections.OrderedDict()
                for t in sorted(sz, key=vkey):
                    grp.setdefault(sz[t], []).append(t)
                szs = "; ".join(f"{'?' if s is None else s}B@{ranges(set(g), tags)}" for s, g in grp.items())
                f.write(f"0x{v:08x}\t{name}\t{ranges(ts, tags)}\t{len(ts)}\t{def_loc[(v, name)][1]}\t{struct_of[(v, name)]}\t{szs}\n")
    with open(prefix + ".hits.tsv", "w") as f:
        f.write("id\tpath\ttags\tn_tags\texample\n")
        for v in ids:
            for path in sorted(by_hit.get(v, {})):
                ts = by_hit[v][path]
                t, ln, text = ex_line[(v, path)]
                f.write(f"0x{v:08x}\t{path}\t{ranges(ts, tags)}\t{len(ts)}\t{t}:{ln}: {text}\n")
    with open(prefix + ".p2.tsv", "w") as f:
        f.write("# P2 raw scan hits for ids with no define and no P1 hit in any tag (empty = none)\n")
        for r in p2rows:
            f.write(r + "\n")
    nl = {t: results[t][2] for t in tags}
    with open(prefix + ".summary.tsv", "w") as f:
        f.write("id\tn_define_tags\tdefine_names\tn_other_hit_tags\tother_hit_files\tp2_hits\n")
        for v in ids:
            names = "; ".join(sorted(by_def.get(v, {})))
            alltags = set().union(*by_def[v].values()) if v in by_def else set()
            other = set().union(*by_hit[v].values()) if v in by_hit else set()
            f.write(f"0x{v:08x}\t{len(alltags)}\t{names}\t{len(other)}\t{len(by_hit.get(v, {}))}\t{sum(1 for r in p2rows if re.search(rf'(^|[^0-9a-fA-F])0*({v:x}|{v})([^0-9a-fA-F]|$)', r.split(':', 3)[-1], re.I)) if v in nohit else 0}\n")
    print(f"non-literal CTRL*CMD define lines (not parsed as 0x literal), summed over tags: {sum(nl.values())}", file=sys.stderr)


if __name__ == "__main__":
    main()
