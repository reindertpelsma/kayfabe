#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""A fixed-seed, breadth-first sample of Vulkan CTS groups as a vkcts case list (stdlib only; GPU-free tested).

STATUS: RESEARCH, 2026-11-18 (docs/design/V3_GPUPV_GAP_MATRIX.md section 4). The full groups of the main mustpass hold 10^4 to 10^5 cases each (counted on 2026-11-18 from
vulkan-cts-1.4.7.1: sparse_resources 19402, ray_tracing_pipeline 22677, ray_query 49359, mesh_shader 28061, synchronization2 81617, synchronization 64872, transform_feedback
133719, memory 6364, memory_model 18530, api 267509, compute 67553, subgroups 48621, video 9471), far more than a box run can take, so a group is SAMPLED: its cases are
split by the next two name components (`dEQP-VK.<group>.<a>.<b>`), each split is shuffled with Random("<seed>:<group>:<split>"), and one case at a time is taken from each split in
turn until the budget is used. The same seed gives the same list on every machine; the list is a trusted `vkcts.py run --caselist` file and its sample is stated, never called a full run.

    cts_groups.py --mustpass DIR --groups sparse_resources,ray_query --per-group 100 [--seed 20261118] > caselist.tsv
    DIR = external/vulkancts/mustpass/main of a checkout at the pinned commit (it holds vk-default.txt and vk-default/**).
Output: `area TAB case`, the two info cases first (the run reads the device from them), then the groups in the order given. A group with no case is reported on stderr.
"""
import argparse
import collections
from pathlib import Path
import random
import re
import sys

# the names `vkcts.py run --caselist` accepts (vkcts_lib.CASE_RE): a mustpass name with a component over 120 characters cannot be run by that row and is left out of the sample
CASE_OK = re.compile(r"^dEQP-VK(?:\.[A-Za-z0-9_-]{1,120}){1,24}$")
INFO = ("dEQP-VK.info.device_properties", "dEQP-VK.info.device_extensions")
# group name -> mustpass file stem(s) under vk-default/ (a stem names vk-default/<stem>.txt and everything under vk-default/<stem>/)
FILES = {"sparse_resources": "sparse-resources", "ray_tracing_pipeline": "ray-tracing-pipeline", "ray_query": "ray-query", "mesh_shader": "mesh-shader",
         "synchronization2": "synchronization2", "synchronization": "synchronization", "transform_feedback": "transform-feedback", "device_group": "device-group",
         "memory": "memory", "memory_model": "memory-model", "api": "api", "pipeline": "pipeline", "compute": "compute", "subgroups": "subgroups", "video": "video"}


def group_files(listing, stem):
    """The mustpass files of a group, from the text of vk-default.txt."""
    return [p for p in listing.split() if p == "vk-default/%s.txt" % stem or p.startswith("vk-default/%s/" % stem)]


def read_group(mustpass, group):
    listing = (Path(mustpass) / "vk-default.txt").read_text()
    names = set()
    for rel in group_files(listing, FILES[group]):
        for line in (Path(mustpass) / rel).read_text().splitlines():
            line = line.strip()
            if line.startswith("dEQP-VK.") and CASE_OK.match(line):
                names.add(line)
    return sorted(names)


def split_key(name):
    return ".".join(name.split(".")[:4])


def sample(names, n, seed, group):
    """Breadth first: one case per split in turn. Deterministic for (names, n, seed, group)."""
    splits = collections.OrderedDict()
    for nm in names:
        splits.setdefault(split_key(nm), []).append(nm)
    for key, lst in splits.items():
        random.Random("%s:%s:%s" % (seed, group, key)).shuffle(lst)
    order = list(splits)
    random.Random("%s:%s:order" % (seed, group)).shuffle(order)
    out, i = [], 0
    while len(out) < n and any(splits[k] for k in order):
        k = order[i % len(order)]
        if splits[k]:
            out.append(splits[k].pop())
        i += 1
    return sorted(out)


def build(mustpass, groups, per_group, seed):
    lines, notes = [], []
    for nm in INFO:
        lines.append("info\t%s" % nm)
    seen = set(INFO)
    for g in groups:
        if g not in FILES:
            notes.append("unknown group %s" % g)
            continue
        names = read_group(mustpass, g)
        if not names:
            notes.append("group %s has no case in this mustpass" % g)
            continue
        pick = [c for c in sample(names, per_group, seed, g) if c not in seen]
        seen.update(pick)
        lines += ["%s\t%s" % (g, c) for c in pick]
        notes.append("group %s: %d of %d cases sampled" % (g, len(pick), len(names)))
    return lines, notes


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--mustpass", required=True)
    ap.add_argument("--groups", required=True)
    ap.add_argument("--per-group", type=int, default=100)
    ap.add_argument("--seed", default="20261118")
    a = ap.parse_args(argv)
    lines, notes = build(a.mustpass, [g for g in a.groups.split(",") if g], a.per_group, a.seed)
    sys.stdout.write("\n".join(lines) + "\n")
    for n in notes:
        print(n, file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
