#!/usr/bin/env python3
"""Cross-check the driver-matrix sweep against nvkvm-pv's committed 216-tag ABI sweep.

usage: crosscheck.py <sweep-dir> <nvkvm-pv ogkm_abi_sweep.tsv>  > crosscheck.tsv
Every (tag, field) both measured is compared; a disagreement is a finding for both projects.
"""
import os, sys, collections

FIELDS = [  # nvkvm-pv column → (struct, path) in our DWARF layouts ('.' = sizeof)
    ("uvm_map_ext_size", "UVM_MAP_EXTERNAL_ALLOCATION_PARAMS", "."),
    ("uvm_map_ext_fd_off", "UVM_MAP_EXTERNAL_ALLOCATION_PARAMS", "rmCtrlFd"),
    ("uvm_sem_pool_size", "UVM_ALLOC_SEMAPHORE_POOL_PARAMS", "."),
    ("uvm_register_gpu_size", "UVM_REGISTER_GPU_PARAMS", "."),
    ("uvm_register_gpu_fd_off", "UVM_REGISTER_GPU_PARAMS", "rmCtrlFd"),
    ("uvm_register_gpu_status_off", "UVM_REGISTER_GPU_PARAMS", "rmStatus"),
    ("uvm_register_gpu_hclient_off", "UVM_REGISTER_GPU_PARAMS", "hClient"),
    ("uvm_register_gpu_hsmcpart_off", "UVM_REGISTER_GPU_PARAMS", "hSmcPartRef"),
    ("chan_alloc_size", "NV_CHANNEL_ALLOC_PARAMS", "."),
    ("vaspace_alloc_size", "NV_VASPACE_ALLOCATION_PARAMETERS", "."),
    ("mem_alloc_size", "NV_MEMORY_ALLOCATION_PARAMS", "."),
    ("nv00de_alloc_size", "NV00DE_ALLOC_PARAMETERS", "."),
    ("nvos46_size", "NVOS46_PARAMETERS", "."),
    ("nvos46_status_off", "NVOS46_PARAMETERS", "status"),
]

sweep, theirs = sys.argv[1], sys.argv[2]
pv = {}
for line in open(theirs):
    if line.startswith("#") or not line.strip():
        continue
    cols = line.rstrip("\n").split("\t")
    if not cols[0][0].isdigit():
        continue
    pv[cols[0]] = cols[1:]

def ours(tag):
    d = collections.defaultdict(dict)
    for axis in ("gsp", "sdk", "os"):
        f = os.path.join(sweep, tag, axis, "layouts.tsv")
        if not os.path.exists(f):
            continue
        for line in open(f):
            s, p, off, size, _ = line.rstrip("\n").split("\t")
            d[s].setdefault(p, (int(off), int(size)))
    return d

print("# tag\tfield\tnvkvm-pv\tdriver-matrix\tverdict")
agree = disagree = only = 0
for tag in sorted(set(pv) & set(os.listdir(sweep)), key=lambda t: tuple(int(x) for x in t.split("."))):
    if not os.path.exists(os.path.join(sweep, tag, ".done")):
        continue
    mine = ours(tag)
    for i, (name, s, p) in enumerate(FIELDS):
        theirs_v = pv[tag][i] if i < len(pv[tag]) else "?"
        m = mine.get(s, {}).get(p)
        mv = "MISSING" if m is None else str(m[1] if p == "." else m[0])
        tv = theirs_v.rstrip("*")
        verdict = "agree" if tv == mv else "DISAGREE"
        if verdict == "agree":
            agree += 1
        else:
            disagree += 1
        print(f"{tag}\t{name}\t{theirs_v}\t{mv}\t{verdict}")
print(f"# cells compared {agree + disagree}: agree {agree}, DISAGREE {disagree}")
