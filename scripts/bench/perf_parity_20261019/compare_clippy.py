#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Compare complete Clippy diagnostic multisets, ignoring shifted line numbers.

Both runs must include cargo's build-finished marker. Inherited denied diagnostics remain
visible; no lint is suppressed. A new site, changed diagnostic or occurrence count fails.
"""
import collections
import json
import pathlib
import sys


def census(path):
    sites = collections.Counter()
    counts = collections.Counter()
    finished = None
    for line in pathlib.Path(path).read_text().splitlines():
        row = json.loads(line)
        if row.get("reason") == "build-finished":
            finished = row["success"]
        if row.get("reason") != "compiler-message":
            continue
        message = row["message"]
        if message["level"] not in ("error", "warning"):
            continue
        primary = [s for s in message["spans"] if s["is_primary"]]
        for span in primary or [None]:
            path = span["file_name"] if span else "<unlocated>"
            source = " ".join(" ".join(t["text"] for t in span["text"]).split()) if span else message["message"]
            key = (message["level"], (message.get("code") or {}).get("code"), path, source)
            sites[key] += 1
            counts[message["level"]] += 1
    if finished is None:
        raise ValueError("missing build-finished marker: incomplete Clippy output")
    return sites, dict(counts), finished


if __name__ == "__main__":
    base, bc, bf = census(sys.argv[1])
    candidate, cc, cf = census(sys.argv[2])
    result = {"base": bc, "candidate": cc, "base_build_success": bf,
              "candidate_build_success": cf, "added": sum((candidate - base).values()),
              "removed": sum((base - candidate).values()), "same_multiset": base == candidate}
    print(json.dumps(result, sort_keys=True, indent=2))
    if base != candidate:
        for key, count in (candidate - base).items():
            print("ADDED", count, key)
        for key, count in (base - candidate).items():
            print("REMOVED", count, key)
        sys.exit(1)
