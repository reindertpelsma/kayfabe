#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Per-group counts and the differing cases of host-versus-guest Vulkan CTS runs given as several logs (chunks) per side (stdlib only; GPU-free tested).

STATUS: RESEARCH, 2026-11-18 (docs/design/V3_GPUPV_GAP_MATRIX.md section 3.5). The inputs are `vkcts.py run` logs with a VKCTS_RESULTS block; the comparison is diffrun's.

    cts_report.py --host LOG [LOG..] --guest LOG [LOG..] [--limits FILE]
Prints one `AREA` line per group (host and guest class counts), then the differing cases as diffrun prints them, then `CTS_REPORT cases=.. differing=.. candidates=..`.
A case in the list but in no log of one side is not_run on that side. Exit status 1 when a candidate (an unclassified guest-only failure) remains.
"""
import argparse
import collections
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import diffrun as D  # noqa: E402


def read_side(paths):
    rows = {}
    for p in paths:
        rows.update(D.parse_vkcts_results(Path(p).read_text(errors="replace")))
    return rows


def area_table(host, guest):
    """-> {area: {'host': Counter, 'guest': Counter}} over the union of cases."""
    out = collections.OrderedDict()
    for case in list(host) + [c for c in guest if c not in host]:
        h, g = host.get(case), guest.get(case)
        area = (h or g)["area"]
        t = out.setdefault(area, {"host": collections.Counter(), "guest": collections.Counter()})
        t["host"][h["status"] if h else "not_run"] += 1
        t["guest"][g["status"] if g else "not_run"] += 1
    return out


def fmt(c):
    return " ".join("%s=%d" % (k, c[k]) for k in D.CLASSES if c.get(k))


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--host", nargs="+", required=True)
    ap.add_argument("--guest", nargs="+", required=True)
    ap.add_argument("--limits")
    ap.add_argument("--xid")
    a = ap.parse_args(argv)
    host, guest = read_side(a.host), read_side(a.guest)
    limits = D.parse_limits(Path(a.limits).read_text()) if a.limits else []
    xids = D.parse_xids(Path(a.xid).read_text()) if a.xid else []
    for area, t in area_table(host, guest).items():
        print("AREA %s host: %s | guest: %s" % (area, fmt(t["host"]), fmt(t["guest"])))
    diffs = D.compare(host, guest, limits, xids)
    print("\n".join(D.diff_lines(diffs)))
    cand = [d for d in diffs if d["bucket"] == "kayfabe_gap_candidate"]
    print("CTS_REPORT cases=%d differing=%d candidates=%d" % (len(set(host) | set(guest)), len(diffs), len(cand)))
    return 1 if cand else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
