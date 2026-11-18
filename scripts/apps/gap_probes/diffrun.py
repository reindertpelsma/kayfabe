#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""One differential runner's GPU-free core: the same case list on the host and in the guest, one verdict per case, only differences reported (stdlib only).

STATUS: RESEARCH, 2026-11-18 (design: docs/design/V3_GPUPV_GAP_MATRIX.md section 4, FULL-SUITE; GPU-free tested by scripts/ci/test_gap_probes.py; not yet run on a
box). It generalises what vkcts.py diff and nvcodec_matrix.py compare each do for their own suite, so that a full suite (Vulkan CTS groups, CUDA samples, GL CTS,
the FFmpeg matrix) is one adapter away. A row is a dict(case, status, area, reason, secs, t0, t1, kv). Status is normalised to the six classes of
CLASSES; `timeout` and `crash` stay separate from `fail`, because a timeout is a different kind of defect than a wrong answer.

    diffrun.py diff HOST.tsv GUEST.tsv [--format gap|vkcts|nvcodec] [--limits FILE] [--xid FILE]   print only the cases that differ, then one summary line
    The exit status is 1 when any CANDIDATE remains (a difference nobody has classified), else 0.
"""
import argparse
import collections
from pathlib import Path
import re
import sys

CLASSES = ("pass", "not_supported", "fail", "timeout", "crash", "not_run")
OK = ("pass",)
BAD = ("fail", "timeout", "crash")
CANDIDATES = ("GUEST_ONLY_FAIL", "GUEST_LOSES_SUPPORT", "GUEST_FAIL_HOST_NS", "GUEST_GAINS_SUPPORT", "VALUE_DIFF")
BUCKETS = ("kayfabe_limitation", "app_or_harness_bug", "config_error", "kayfabe_gap_candidate")
# raw statuses of the suites this runner reads -> class
RAW = {"pass": "pass", "pass_with_warning": "pass", "qualitywarning": "pass", "compatibilitywarning": "pass", "waiver": "pass", "warn": "pass",
       "notsupported": "not_supported", "not_supported": "not_supported", "fail": "fail", "mismatch": "fail", "resourceerror": "fail", "internalerror": "fail",
       "devicelost": "fail", "timeout": "timeout", "crash": "crash", "notrun": "not_run", "not_run": "not_run"}


def normalise(raw, reason=""):
    key = re.sub(r"[^a-z_]", "", str(raw).lower())
    cls = RAW.get(key, "fail")
    if cls == "fail" and re.search(r"time ?out", str(reason), re.I):
        return "timeout"
    if cls == "fail" and re.match(r"crash", str(reason), re.I):
        return "crash"
    return cls


def row(case, status, area="-", reason="-", secs="-", t0=None, t1=None, kv=None):
    return {"case": case, "status": status, "area": area, "reason": reason, "secs": secs, "t0": t0, "t1": t1, "kv": dict(kv or {})}


# ------------------------------------------------------------------------------------------------------------------ adapters
def parse_gap_results(text):
    """results.tsv of gap_probes.py: id group status rc secs t0 t1 reason canary. kv.tsv (case key value) is read by parse_gap_kv."""
    rows = {}
    for line in text.splitlines():
        c = line.split("\t")
        if len(c) < 9 or c[0] in ("id", "#id") or line.startswith("#"):
            continue
        rows[c[0]] = row(c[0], normalise(c[2], c[7]), c[1], c[7], c[4], _f(c[5]), _f(c[6]), {"canary": c[8]})
    return rows


def parse_gap_kv(text, rows):
    for line in text.splitlines():
        c = line.split("\t", 2)
        if len(c) == 3 and c[0] in rows:
            rows[c[0]]["kv"][c[1]] = c[2]
    return rows


def parse_vkcts_results(text):
    """The VKCTS_RESULTS block: #case status class area detail."""
    rows = {}
    for line in text.splitlines():
        c = line.split("\t")
        if len(c) < 5 or line.startswith("#") or line.startswith("VKCTS_"):
            continue
        rows[c[0]] = row(c[0], normalise(c[1], c[4]), c[3], c[4])
    return rows


def parse_nvcodec_results(text):
    rows = {}
    for line in text.splitlines():
        c = line.split("\t")
        if len(c) < 12 or c[0] == "case_id" or line.startswith("#"):
            continue
        rows[c[0]] = row(c[0], normalise(c[3], c[4]), c[1], c[4], c[6])
    return rows


def _f(x):
    try:
        return float(x)
    except (TypeError, ValueError):
        return None


FORMATS = {"gap": parse_gap_results, "vkcts": parse_vkcts_results, "nvcodec": parse_nvcodec_results}


# ------------------------------------------------------------------------------------------------------------------ known limits
def parse_limits(text):
    """TSV: case_glob key_glob host_class guest_class bucket reason. A limit explains a difference; it never hides an unrelated one. `*` matches anything."""
    out = []
    for line in text.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        c = line.rstrip("\n").split("\t")
        if len(c) != 6:
            raise ValueError("limits: need 6 tab separated columns: %r" % line)
        if c[4] not in BUCKETS:
            raise ValueError("limits: bucket %r is not one of %s" % (c[4], ",".join(BUCKETS)))
        if not c[5].strip() or c[5].strip() == "-":
            raise ValueError("limits: every limit carries a reason: %r" % line)
        out.append(dict(zip(("case", "key", "host", "guest", "bucket", "reason"), c)))
    return out


def _glob(pat, s):
    return re.fullmatch(re.escape(pat).replace(r"\*", ".*"), s) is not None


def limit_for(limits, case, key, host_cls, guest_cls):
    for l in limits:
        if _glob(l["case"], case) and _glob(l["key"], key) and _glob(l["host"], host_cls) and _glob(l["guest"], guest_cls):
            return l
    return None


# ------------------------------------------------------------------------------------------------------------------ the comparison
def categorize(h, g):
    if h == "not_run" or g == "not_run":
        return "INCOMPLETE"
    if h in OK and g in OK:
        return "SAME_OK"
    if h == "not_supported" and g == "not_supported":
        return "SAME_NOT_SUPPORTED"
    if h in BAD and g in BAD:
        return "BOTH_FAIL" if h == g else "BOTH_FAIL_DIFFERENT"
    if h in OK:
        return "GUEST_ONLY_FAIL" if g in BAD else "GUEST_LOSES_SUPPORT"
    if h == "not_supported":
        return "GUEST_GAINS_SUPPORT" if g in OK else "GUEST_FAIL_HOST_NS"
    return "HOST_ONLY_FAIL" if g in OK else "HOST_FAIL_GUEST_NOT_SUPPORTED"


def kv_diffs(hk, gk):
    """Keys whose values differ, or exist on one side only (the canary key is the harness's, `perf.*` keys are measurements that vary between two runs of the same machine, and `info.*` keys are text kept in the evidence: none of them is a result)."""
    return [(k, hk.get(k, "-"), gk.get(k, "-")) for k in sorted(set(hk) | set(gk)) if k != "canary" and not k.startswith(("perf.", "info.")) and hk.get(k) != gk.get(k)]


def parse_xids(text):
    """Host kernel log lines -> [(epoch_seconds, xid_number, line)]. Accepts `[1700000000.123] NVRM: Xid (PCI:..): 109, ...` (dmesg -r / journalctl -o short-unix style)."""
    out = []
    for line in text.splitlines():
        m = re.search(r"(\d{9,11}(?:\.\d+)?).*NVRM: Xid \([^)]*\): (\d+)", line)
        if m:
            out.append((float(m[1]), int(m[2]), line.strip()))
    return out


def xids_in(xids, r, slack=5.0):
    if r is None or r["t0"] is None or r["t1"] is None:
        return []
    return sorted({x for t, x, _ in xids if r["t0"] - 1 <= t <= r["t1"] + slack})


def compare(host, guest, limits=(), xids=(), slack=5.0):
    """-> list of dict(case, area, host, guest, category, bucket, why, xid, kvdiff), one per case that is not SAME_*. Bucket is set when a limit matches,
    for BOTH_FAIL (the host control fails too: not kayfabe's, `app_or_harness_bug`), and otherwise `kayfabe_gap_candidate` for the five CANDIDATES (triage still decides)."""
    out = []
    for case in list(host) + [c for c in guest if c not in host]:
        h, g = host.get(case), guest.get(case)
        hc, gc = (h["status"] if h else "not_run"), (g["status"] if g else "not_run")
        cat = categorize(hc, gc)
        kd = kv_diffs(h["kv"], g["kv"]) if h and g and hc == gc and hc in OK else []
        if kd:
            cat = "VALUE_DIFF"
        if cat.startswith("SAME_"):
            continue
        keys = [k for k, _, _ in kd] or ["*"]
        lims = [limit_for(limits, case, k, hc, gc) for k in keys]
        bucket, why = "-", "-"
        if all(lims):
            bucket, why = lims[0]["bucket"], lims[0]["reason"]
        elif cat in ("BOTH_FAIL", "BOTH_FAIL_DIFFERENT"):
            bucket, why = "app_or_harness_bug", "the host control fails the same case"
        elif cat in CANDIDATES:
            bucket, why = "kayfabe_gap_candidate", "unclassified"
        out.append({"case": case, "area": (h or g)["area"], "host": hc, "guest": gc, "category": cat, "bucket": bucket, "why": why,
                    "xid": ",".join(map(str, xids_in(xids, g, slack))) or "-", "host_xid": ",".join(map(str, xids_in(xids, h, slack))) or "-",
                    "kvdiff": ";".join("%s:%s->%s" % d for d in kd[:6]) or "-"})
    return out


def summary_line(host, guest, diffs):
    c = collections.Counter(d["category"] for d in diffs)
    cand = sum(1 for d in diffs if d["bucket"] == "kayfabe_gap_candidate" or (d["bucket"] == "-" and d["category"] in CANDIDATES))
    b = collections.Counter(d["bucket"] for d in diffs)
    return "DIFFRUN_SUMMARY cases=%d differing=%d candidates=%d limitation=%d harness=%d config=%d %s" % (
        len(set(host) | set(guest)), len(diffs), cand, b["kayfabe_limitation"], b["app_or_harness_bug"], b["config_error"],
        " ".join("%s=%d" % (k.lower(), v) for k, v in sorted(c.items())))


def diff_lines(diffs):
    out = ["#case\tarea\thost\tguest\tcategory\tbucket\tguest_xid\thost_xid\tkvdiff\twhy"]
    for d in diffs:
        out.append("\t".join(str(d[k]) for k in ("case", "area", "host", "guest", "category", "bucket", "xid", "host_xid", "kvdiff", "why")))
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("diff")
    d.add_argument("host")
    d.add_argument("guest")
    d.add_argument("--format", choices=sorted(FORMATS), default="gap")
    d.add_argument("--limits")
    d.add_argument("--xid")
    a = ap.parse_args(argv)
    host, guest = (FORMATS[a.format](Path(p).read_text()) for p in (a.host, a.guest))
    if a.format == "gap":
        for rows, p in ((host, a.host), (guest, a.guest)):
            kvp = Path(p).with_name("kv.tsv")
            if kvp.exists():
                parse_gap_kv(kvp.read_text(), rows)
    limits = parse_limits(Path(a.limits).read_text()) if a.limits else []
    xids = parse_xids(Path(a.xid).read_text()) if a.xid else []
    diffs = compare(host, guest, limits, xids)
    print("\n".join(diff_lines(diffs)))
    print(summary_line(host, guest, diffs))
    return 1 if any(x["bucket"] == "kayfabe_gap_candidate" for x in diffs) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
