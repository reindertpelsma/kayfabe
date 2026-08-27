#!/usr/bin/env python3
"""
nvmemsnap_diff.py — diff NVIDIA mmap CONTENT snapshots host-vs-guest with a 3x3 noise filter.

Captured by nvmemsnap.so (SNAP records). Run the test 3x on host and 3x on guest, then:

    nvmemsnap_diff.py --host h1.txt h2.txt h3.txt --guest g1.txt g2.txt g3.txt [--tag post]

Noise filter (user-specified): for each (region, byte-offset), collect the set of byte values
seen across the host runs and across the guest runs. A byte is FLAGGED as a real divergence
ONLY if it is STABLE within each platform (one value across all host runs, one across all guest
runs) AND host != guest. Bytes that vary within a platform are benign (ASLR/handles/timing).

Consecutive flagged offsets are grouped into runs and printed with host vs guest bytes.
"""
import sys, argparse
from collections import defaultdict

def parse(path):
    """return dict: (tag, occ, region_key) -> bytes ; region_key=(rpath,off,length)"""
    out = {}
    occ = defaultdict(int)
    with open(path, errors="replace") as f:
        for line in f:
            if not line.startswith("SNAP "):
                continue
            fields = {}
            hexs = None
            for tok in line.split():
                if tok.startswith("h="):
                    hexs = tok[2:]
                elif "=" in tok:
                    k, v = tok.split("=", 1); fields[k] = v
            tag = line.split()[1]
            rk = (fields.get("path", "?"), fields.get("off", "?"), fields.get("len", "?"))
            key = (tag, rk)
            occ[key] += 1
            fullkey = (tag, occ[key], rk)
            try:
                out[fullkey] = bytes.fromhex(hexs) if hexs else b""
            except ValueError:
                out[fullkey] = b""
    return out

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", nargs="+", required=True)
    ap.add_argument("--guest", nargs="+", required=True)
    ap.add_argument("--tag", default=None, help="only this snapshot tag (e.g. post/pre/crash)")
    ap.add_argument("--context", type=int, default=4, help="bytes of context around a run")
    ap.add_argument("--breakdown", action="store_true", help="per-region raw/var/flagged stats")
    args = ap.parse_args()

    H = [parse(p) for p in args.host]
    G = [parse(p) for p in args.guest]

    keys = set()
    for d in H + G:
        keys.update(d.keys())

    flagged_total = 0
    skipped = 0
    raw_total = hvar_total = gvar_total = 0
    for key in sorted(keys):
        tag, occ, rk = key
        if args.tag and tag != args.tag:
            continue
        hvals = [d.get(key) for d in H]
        gvals = [d.get(key) for d in G]
        if any(v is None for v in hvals) or any(v is None for v in gvals):
            # region not present in every run -> can't apply the 3x3 filter reliably
            skipped += 1
            continue
        n = min(len(v) for v in hvals + gvals)
        if n == 0:
            continue
        flags = []
        raw = hvar = gvar = nz = 0
        for i in range(n):
            hs = {v[i] for v in hvals}
            gs = {v[i] for v in gvals}
            if any(v[i] for v in hvals) or any(v[i] for v in gvals):
                nz += 1
            if hvals[0][i] != gvals[0][i]:
                raw += 1
            hv1 = len(hs) == 1; gv1 = len(gs) == 1
            if not hv1: hvar += 1
            if not gv1: gvar += 1
            if hv1 and gv1 and hs != gs:
                flags.append(i)
        raw_total += raw; hvar_total += hvar; gvar_total += gvar
        if args.breakdown:
            path, off, length = rk
            print(f"[{tag}#{occ}] {path} len={length} cmp={n} nonzero={nz} "
                  f"raw_diff={raw} host_var={hvar} guest_var={gvar} FLAGGED={len(flags)}")
        if not flags:
            continue
        # group consecutive offsets
        runs = []
        s = flags[0]; p = flags[0]
        for x in flags[1:]:
            if x == p + 1:
                p = x
            else:
                runs.append((s, p)); s = x; p = x
        runs.append((s, p))
        path, off, length = rk
        print(f"\n=== region {path} off={off} len={length}  [{tag}#{occ}]  "
              f"{len(flags)} divergent bytes in {len(runs)} run(s) ===")
        for (a, b) in runs:
            c0 = max(0, a - args.context); c1 = min(n, b + 1 + args.context)
            hb = hvals[0][c0:c1].hex()
            gb = gvals[0][c0:c1].hex()
            # mark the divergent span
            print(f"  +0x{a:x}..0x{b:x} ({b-a+1}B)")
            print(f"    host : {hb}")
            print(f"    guest: {gb}")
            flagged_total += (b - a + 1)
    print(f"\n[summary] raw_diff={raw_total} (host[0] vs guest[0])  "
          f"host_var={hvar_total} guest_var={gvar_total} (benign, within-platform)  "
          f"regions_skipped={skipped} (not in every run)")
    print(f"[summary] {flagged_total} consistently-divergent bytes "
          f"(stable within platform, host!=guest)")

if __name__ == "__main__":
    main()
