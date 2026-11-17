# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Committed VK-GL-CTS caselists of the open rows (STATUS: DESIGN-ONLY, 2026-11-17; GPU-free).

A caselist is a plain text file, one fully qualified case per line, `#` comments allowed; deqp-vk and glcts read exactly that with
--deqp-caselist-file (CHECKED in framework/common/tcuCommandLine.cpp parseSimpleCaseList: comments skipped, a duplicate is an error).
The cases are SELECTED from the must-pass lists of the pinned CTS commit by caselist_spec.json (group prefix + how many, evenly spaced,
order kept), so a reviewer can regenerate and diff them:

  python3 cts_caselist.py generate --cts-tree PATH_TO_CHECKOUT_AT_THE_PINNED_COMMIT [--write]
  python3 cts_caselist.py check                    committed files against the spec and the recorded sha256 (CI; no tree needed)
"""
import argparse
import os
import re
import sys

from kfopen_common import HERE, OpenRowError, load_json, sha256_text, write_json

SPEC = os.path.join(HERE, "caselist_spec.json")
NAME_RX = re.compile(r"^[A-Za-z0-9_+\-]+(?:\.[A-Za-z0-9_+\-]+){2,}$")
ROOTS = {"gl46_core": ("KHR-GL46.", "GTF-GL46."), "vk_breadth": ("dEQP-VK.",)}
MIN_CASES, MAX_CASES = 150, 1000


def select(lines, prefix, take):
    """`take` is "all" or an int n: n entries of the prefix-matching lines at evenly spaced positions (order kept, no repeats)."""
    pool = [l for l in lines if l.startswith(prefix)]
    if not pool:
        raise OpenRowError("CASELIST_EMPTY_GROUP no case starts with %s" % prefix)
    if take == "all" or take >= len(pool):
        return pool
    if not isinstance(take, int) or take < 1:
        raise OpenRowError("CASELIST_BAD_TAKE %r for %s" % (take, prefix))
    idx = sorted({(i * len(pool)) // take for i in range(take)})
    return [pool[i] for i in idx]


def read_mustpass(tree, rel):
    path = os.path.join(tree, *rel.split("/"))
    with open(path, encoding="utf-8") as f:
        return [l.strip() for l in f if l.strip() and not l.startswith("#")]


def generate(spec, tree):
    """{list name: [cases]} from the must-pass files under `tree`."""
    out = {}
    for name, lst in spec["lists"].items():
        cases, seen = [], set()
        for g in lst["groups"]:
            for c in select(read_mustpass(tree, g["source"]), g["prefix"], g["take"]):
                if c not in seen:
                    seen.add(c)
                    cases.append(c)
        out[name] = cases
    return out


def render(name, commit, cases, groups):
    head = ["# kfopen caselist %s: %d cases selected from the must-pass lists of VK-GL-CTS %s" % (name, len(cases), commit),
            "# regenerate: python3 cts_caselist.py generate --cts-tree <checkout at that commit> --write; do not edit by hand"]
    head += ["# group %s take %s" % (g["prefix"], g["take"]) for g in groups]
    return "\n".join(head + cases) + "\n"


def parse(text):
    return [l.strip() for l in text.splitlines() if l.strip() and not l.lstrip().startswith("#")]


def validate(name, text):
    """Everything a runner relies on, without the CTS tree: non-empty, a few hundred cases, unique, well-formed, no wildcards, known roots."""
    cases = parse(text)
    problems = []
    if not MIN_CASES <= len(cases) <= MAX_CASES:
        problems.append("CASELIST_SIZE %d cases (want %d..%d)" % (len(cases), MIN_CASES, MAX_CASES))
    dups = sorted({c for c in cases if cases.count(c) > 1}) if len(cases) < 5000 else []
    if dups:
        problems.append("CASELIST_DUPLICATE %s" % dups[0])
    for c in cases:
        if "*" in c or " " in c or not NAME_RX.match(c):
            problems.append("CASELIST_BAD_NAME %s" % c)
            break
        if not c.startswith(ROOTS[name]):
            problems.append("CASELIST_BAD_ROOT %s" % c)
            break
    return problems


def check(spec=None, base=HERE):
    spec = spec or load_json(SPEC)
    problems = []
    for name, lst in spec["lists"].items():
        path = os.path.join(base, *lst["out"].split("/"))
        if not os.path.exists(path):
            problems.append("CASELIST_MISSING %s" % lst["out"])
            continue
        raw = open(path, encoding="utf-8", newline="").read()
        if "\r" in raw:
            problems.append("CASELIST_CRLF %s (the sha256 is over LF text)" % lst["out"])
        if sha256_text(raw) != lst.get("sha256"):
            problems.append("CASELIST_HASH %s differs from caselist_spec.json" % lst["out"])
        problems += ["%s: %s" % (name, p) for p in validate(name, raw)]
        head = [l for l in raw.splitlines() if l.startswith("# kfopen caselist")]
        if not head or spec["cts_commit"] not in head[0]:
            problems.append("CASELIST_COMMIT %s does not name the pinned CTS commit" % lst["out"])
    return problems


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("generate")
    g.add_argument("--cts-tree", required=True)
    g.add_argument("--write", action="store_true")
    sub.add_parser("check")
    a = ap.parse_args(argv)
    spec = load_json(SPEC)
    if a.cmd == "check":
        problems = check(spec, HERE)
        for p in problems:
            print("PROBLEM", p)
        print("CASELISTS_OK" if not problems else "CASELISTS_FAILED problems=%d" % len(problems))
        return 1 if problems else 0
    produced = generate(spec, a.cts_tree)
    for name, cases in produced.items():
        text = render(name, spec["cts_commit"], cases, spec["lists"][name]["groups"])
        bad = validate(name, text)
        print("%s: %d cases sha256 %s%s" % (name, len(cases), sha256_text(text)[:16], (" PROBLEMS " + "; ".join(bad)) if bad else ""))
        if a.write:
            with open(os.path.join(HERE, *spec["lists"][name]["out"].split("/")), "w", encoding="utf-8", newline="\n") as f:
                f.write(text)
            spec["lists"][name]["sha256"] = sha256_text(text)
            spec["lists"][name]["count"] = len(cases)
    if a.write:
        write_json(SPEC, spec)
    return 0


if __name__ == "__main__":
    sys.exit(main())
