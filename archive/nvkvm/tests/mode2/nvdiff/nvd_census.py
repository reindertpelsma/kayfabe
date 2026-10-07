#!/usr/bin/env python3
"""
nvd_census.py — every non-`NV_OK` status in a capture, with its index and its id.

    usage: nvd_census.py <capture.jsonl[.zst]> [more...]

★ The instrument behind the differential's most useful single number. `[measured
2026-08-10]` a real GA106 issues **exactly one** non-OK status in a 613-record CUDA
program — `0x2080012f GPU_QUERY_ECC_STATUS`, in `cuInit` — so on this path "0x56 is
the forgiven status" stops being a heuristic: any OTHER `0x56` our side emits is a
measured divergence from hardware, whatever the caller does next.

⊘ It is a USERSPACE census and says nothing about the RM<->GSP RPC plane, where the
same guest's kernel emits hundreds of `status=0x56` lines this cannot see. "Three"
is true of the ioctl boundary and false of the driver.

⊘ And an id here is the id USERSPACE issued, which is not always the id our device
receives: `0x2080200a PERF_BOOST` is answered by the guest kernel, which forwards
`0x20800a9a` to physical RM and returns that status unchanged. Rank by kind, and
check which boundary an id crosses before serving it.
"""
import sys, collections
sys.path.insert(0, __file__.rsplit("/", 1)[0])
from nvdiff import load
for path in sys.argv[1:]:
    recs = load(path)
    c = collections.Counter()
    idx = {}
    for i, r in enumerate(recs):
        if r.get("t") != "ioctl":
            continue
        st = (r.get("f_post") or {}).get("status")
        if st:
            c[(r["op"], st)] += 1
            idx.setdefault((r["op"], st), i)
    print("==", path)
    for (op, st), n in sorted(c.items(), key=lambda kv: idx[kv[0]]):
        print("   [%4d] %-50s status=0x%x  x%d" % (idx[(op, st)], op, st, n))
    if not c:
        print("   (no non-OK status)")
