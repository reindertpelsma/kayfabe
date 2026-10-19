#!/usr/bin/env python3
"""Run extract_tag over every release tag of a (blobless, sparse) ogkm clone.

usage: sweep.py <clone> <served.json> <outdir> [tag ...]      (default: all tags, version order)

The clone is first configured by sweep_setup.sh (sparse patterns, so a checkout of a tag fetches only
the headers this tool reads, in one batched request).  One JSON per tag is written to <outdir>;
tags already present are skipped.  Run with  python3 -I  from a directory that holds nothing else.
"""
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import extract_tag  # noqa: E402


def vkey(t):
    return [int(x) for x in re.findall(r"\d+", t)]


def main():
    clone, servedp, outdir = sys.argv[1:4]
    tags = sys.argv[4:]
    if not tags:
        tags = subprocess.check_output(["git", "-C", clone, "tag"], text=True).split()
        tags = [t for t in tags if re.fullmatch(r"\d+(\.\d+)*", t)]
    tags.sort(key=vkey)
    os.makedirs(outdir, exist_ok=True)
    served = json.load(open(servedp))
    cache = extract_tag.Cache()
    for t in tags:
        out = os.path.join(outdir, t + ".json")
        if os.path.exists(out):
            continue
        subprocess.check_call(["git", "-C", clone, "checkout", "-q", "--detach", t])
        res = extract_tag.extract(clone, t, served, cache)
        with open(out + ".tmp", "w") as f:
            json.dump(res, f, sort_keys=True)
        os.rename(out + ".tmp", out)
        print(t, len(res["items"]), flush=True)


if __name__ == "__main__":
    main()
