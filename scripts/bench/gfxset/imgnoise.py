#!/usr/bin/env python3
"""imgnoise.py --bare A.png B.png [C.png ...] --guest G.png — grade a NONDETERMINISTIC image against bare
metal's own MEASURED spread (the Cycles items: GPU atomics change the sample accumulation order, so no two
bare-metal runs give the same PNG).

For every pair of images: PSNR over the R, G and B values (8-bit; alpha ignored), the number of values that
differ, and the largest difference. The bare-metal pairs give the spread; the guest is compared with EVERY
bare-metal image.
  floor  = the LOWEST PSNR among the bare-metal pairs — how far apart bare metal's two farthest runs are
  guest  = the MEDIAN of the guest's PSNR to each bare-metal image
  MATCH iff guest >= floor: the guest is, typically, no farther from bare metal than bare metal is from
  itself. A real defect (a wrong page, a lost write) moves PSNR by tens of dB, not by the spread.
With two bare images the floor is ONE sample of the spread — printed as n_bare=2, and it is thin (measured:
gs2's Cycles CUDA sat 0.3 dB outside a 2-image rule that also allowed a 3 dB margin). More bare-metal runs
(suite.sh's GSET_NOISE_DIRS) make it a distribution.
Prints one line: floor=<dB> guest=<dB> verdict=MATCH|DIFF, then the detail fields."""
import math
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pngdiff import load  # noqa: E402  (nvkvm-pv's verbatim PNG decoder)


def rgb(path):
    w, h, n, px = load(path)
    if n < 3:
        raise SystemExit(f"{path}: {n} channel(s), not RGB")
    return (w, h), bytes(px[i] for i in range(len(px)) if i % n < 3)


def dist(a, b):
    if a[0] != b[0]:
        return (0.0, len(a[1]), 255)
    A, B = a[1], b[1]
    se = 0
    nd = 0
    mx = 0
    for x, y in zip(A, B):
        if x != y:
            d = x - y if x > y else y - x
            se += d * d
            nd += 1
            if d > mx:
                mx = d
    if se == 0:
        return (math.inf, 0, 0)
    return (10 * math.log10(255 * 255 * len(A) / se), nd, mx)


def fmt(v):
    return "inf" if v == math.inf else f"{v:.2f}"


def matrix(paths):
    """--matrix a.png b.png ...: every pair's number of differing values (and the largest difference) —
    the evidence behind a verdict, and the test for a SYSTEMATIC guest difference (guest images that agree
    with each other but not with bare metal)."""
    imgs = [rgb(p) for p in paths]
    names = [os.path.basename(p) for p in paths]
    print("ndiff/maxdiff " + " ".join(f"{n[:14]:>14}" for n in names))
    for i, a in enumerate(imgs):
        row = []
        for j, b in enumerate(imgs):
            _, nd, mx = dist(a, b) if i != j else (0, 0, 0)
            row.append(f"{nd:>11}/{mx:<2}")
        print(f"{names[i][:14]:>14} " + " ".join(row))


def main():
    a = sys.argv[1:]
    if a[:1] == ["--matrix"]:
        return matrix(a[1:])
    if "--bare" not in a or "--guest" not in a:
        raise SystemExit(__doc__)
    bare = a[a.index("--bare") + 1:a.index("--guest")]
    guest = a[a.index("--guest") + 1]
    if len(bare) < 2:
        raise SystemExit("need >= 2 bare-metal images")
    B = [rgb(p) for p in bare]
    G = rgb(guest)
    bb = [dist(B[i], B[j]) for i in range(len(B)) for j in range(i + 1, len(B))]
    gb = [dist(G, b) for b in B]
    floor = min(d[0] for d in bb)
    gmed = statistics.median(d[0] for d in gb)
    v = "MATCH" if gmed >= floor else "DIFF"
    print(f"floor={fmt(floor)}dB guest={fmt(gmed)}dB verdict={v} n_bare={len(B)} "
          f"bare_psnr={fmt(min(d[0] for d in bb))}..{fmt(max(d[0] for d in bb))} "
          f"guest_psnr={fmt(min(d[0] for d in gb))}..{fmt(max(d[0] for d in gb))} "
          f"bare_ndiff={min(d[1] for d in bb)}..{max(d[1] for d in bb)} guest_ndiff={min(d[1] for d in gb)}..{max(d[1] for d in gb)} "
          f"bare_maxdiff={max(d[2] for d in bb)} guest_maxdiff={max(d[2] for d in gb)} values={len(G[1])}")


if __name__ == "__main__":
    main()
