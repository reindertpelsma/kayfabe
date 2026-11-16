#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""VA-apply phase profile of a kf3 QEMU log, as tables (STATUS: LIVE, 2026-11-16).

Reads the lines `docs/design/V3_VA_PROFILE.md` defines and prints:

  1. intervals: one row per status line whose `vat[...]` window did VA work (walks, applies or
     split requests), with the request/apply percentiles, the phase sums and the host-call counts;
  2. phase shares: the sum of every phase over the whole log, as a share of the summed apply time;
  3. slow records: every `kf3: VA_SLOW` line (live or "(at teardown)"), slowest first, with the
     derived `rm_map_only_us` (RM map minus stitch, descriptor and reap) and `other_us` (apply wall
     minus the apply's five partitions);
  4. the `VA_SLOW SUMMARY`, the `VA_WORST` lines and any `capacity ceiling geometry` line.

Only these fields are printed: no GPU UUID, board name or guest byte leaves the log. Old logs (no
new fields) still give the old `vat` columns. Extraction is the one the 2026-11-16 analysis used:
`vat\\[(.*?)\\]` then `(\\w+)=(\\d+)`.

Usage: va_profile_table.py LOG [--all] [--json] [--top N]
       va_profile_table.py --self-test
"""

import json
import re
import sys

VAT = re.compile(r"vat\[(.*?)\]")
FIELD = re.compile(r"(\w+)=(\d+)")
SLOW = re.compile(r"kf3: VA_SLOW n=")
SLOW_FIELD = re.compile(r"(\w+)=(0x[0-9a-fA-F]+|\d+|[a-z]+)")
PHASES = ["prep", "unmapph", "mapph", "takedown", "inval", "rm_map", "rm_unmap", "rm_free",
          "rm_resv", "ledger", "stitch", "desc", "reap"]
PARTITION = ["prep", "unmapph", "mapph", "takedown", "inval"]
INTERVAL_COLS = [
    "line", "walks", "walk_avg_us", "apply_avg_us", "apply_max_ms", "split_max_ms",
    "req_n", "req_p50_us", "req_p95_us", "req_p99_us", "req_max_ms",
    "q_inbox_n", "q_inbox_avg_us", "q_inbox_max_us",
    "aps_n", "aps_avg_us", "aps_p50_us", "aps_p95_us", "aps_p99_us", "aps_max_ms",
    "submit_avg_us", "collect_avg_us", "collect_max_us", "cap_retries",
    "prep_us", "unmapph_us", "mapph_us", "takedown_us", "inval_us",
    "rm_map_n", "rm_map_us", "rm_map_max_us", "rm_unmap_n", "rm_unmap_us", "rm_unmap_max_us",
    "rm_free_n", "rm_free_us", "ledger_us", "stitch_us", "desc_us", "reap_us",
    "pub_max_us", "resume_avg_us", "resume_max_us",
    "e2e_n", "e2e_p50_us", "e2e_p95_us", "e2e_p99_us", "e2e_max_ms",
    "slow_reqs", "slow_aps", "slow_printed", "slow_suppressed", "ring_overwritten",
]


def parse_vat(line):
    """The `vat[...]` fields of one line as a dict of ints, or None."""
    m = VAT.search(line)
    if not m:
        return None
    return {k: int(v) for k, v in FIELD.findall(m.group(1))}


def parse_slow(line):
    """A `kf3: VA_SLOW` record as a dict (ints where numeric), or None."""
    i = line.find("kf3: VA_SLOW n=")
    if i < 0:
        return None
    out = {"teardown": "(at teardown)" in line}
    for k, v in SLOW_FIELD.findall(line[i + len("kf3: VA_SLOW "):]):
        if v.startswith("0x"):
            out[k] = int(v, 16)
        elif v.isdigit():
            out[k] = int(v)
        else:
            out[k] = v
    if out.get("kind") == "apply":
        part = sum(out.get(p + "_us", 0) for p in PARTITION)
        out["other_us"] = max(0, out.get("apply_us", 0) - part)
        out["rm_map_only_us"] = max(0, out.get("rm_map_us", 0) - out.get("stitch_us", 0)
                                    - out.get("desc_us", 0) - out.get("reap_us", 0))
    return out


def scan(lines, keep_all=False):
    res = {"intervals": [], "slow": [], "summary": [], "worst": [], "geometry": [], "totals": {}}
    totals = {p: 0 for p in PHASES}
    totals.update({p + "_n": 0 for p in PHASES})
    totals["apply_us"] = 0
    totals["aps_n"] = 0
    for no, raw in enumerate(lines, 1):
        line = raw.rstrip("\n")
        if "vat[" in line:
            f = parse_vat(line)
            if f is not None:
                busy = f.get("walks", 0) or f.get("req_n", 0) or f.get("aps_n", 0)
                if busy or keep_all:
                    row = {"line": no}
                    row.update(f)
                    res["intervals"].append(row)
                for p in PHASES:
                    totals[p] += f.get(p + "_us", 0)
                    totals[p + "_n"] += f.get(p + "_n", 0)
                totals["apply_us"] += f.get("aps_avg_us", 0) * f.get("aps_n", 0)
                totals["aps_n"] += f.get("aps_n", 0)
            continue
        if "kf3: VA_SLOW SUMMARY" in line:
            res["summary"].append({k: int(v) for k, v in FIELD.findall(line)})
        elif SLOW.search(line):
            r = parse_slow(line)
            if r is not None:
                r["line"] = no
                res["slow"].append(r)
        elif "kf3: VA_WORST" in line:
            res["worst"].append({"line": no, **{k: int(v) for k, v in FIELD.findall(line.split("hostmaps")[0])}})
        elif "capacity ceiling geometry:" in line:
            res["geometry"].append({"line": no, "text": line.split("capacity ceiling geometry:", 1)[1].strip()})
    res["totals"] = totals
    return res


def table(rows, cols):
    out = ["\t".join(cols)]
    for r in rows:
        out.append("\t".join(str(r.get(c, "")) for c in cols))
    return "\n".join(out)


def render(res, top):
    out = ["## intervals (vat windows with VA work; *_max_* and slow_* are boot-wide)"]
    cols = [c for c in INTERVAL_COLS if any(c in r for r in res["intervals"])]
    out.append(table(res["intervals"], cols))
    t = res["totals"]
    out.append("\n## phase shares over the log (sum of window sums; partition shares of summed apply wall)")
    out.append("phase\tcount\tsum_us\tshare_of_apply")
    for p in PHASES:
        share = "%.3f" % (t[p] / t["apply_us"]) if t["apply_us"] else ""
        out.append("%s\t%d\t%d\t%s" % (p, t[p + "_n"], t[p], share))
    out.append("applies\t%d\t%d\t" % (t["aps_n"], t["apply_us"]))
    def key(r):
        return r.get("req_us", 0) if r.get("kind") == "ticket" else r.get("apply_us", 0)
    slow = sorted(res["slow"], key=key, reverse=True)[:top]
    out.append("\n## slow tickets (slowest first)")
    out.append(table([r for r in slow if r.get("kind") == "ticket"],
                     ["line", "n", "t_ms", "ticket", "walk", "pdb", "key", "windows", "req_us", "q_inbox_us", "mgr_us",
                      "submit_us", "walk_us", "collect_us", "apply_own_us", "apply_all_us", "cap_retries", "teardown"]))
    out.append("\n## slow applies (slowest first)")
    out.append(table([r for r in slow if r.get("kind") == "apply"],
                     ["line", "n", "t_ms", "ticket", "walk", "pdb", "key", "slot", "window", "apply_us", "maps", "unmaps",
                      "prep_us", "unmapph_us", "mapph_us", "takedown_us", "inval_us", "rm_map_n", "rm_map_us",
                      "rm_map_max_us", "rm_map_only_us", "stitch_us", "desc_us", "reap_us", "rm_unmap_n", "rm_unmap_us",
                      "rm_unmap_max_us", "ledger_us", "rm_free_us", "other_us", "teardown"]))
    out.append("\n## summary / worst / ceiling geometry")
    for s in res["summary"]:
        out.append("VA_SLOW SUMMARY " + " ".join("%s=%d" % kv for kv in s.items()))
    if res["worst"]:
        w = res["worst"][-1]
        out.append("last VA_WORST " + " ".join("%s=%s" % kv for kv in w.items()))
    for g in res["geometry"]:
        out.append("line %d: %s" % (g["line"], g["text"]))
    return "\n".join(out)


SAMPLE = [
    "kf3: status phase=Running mirrors=3 reused=1 prewarmed=2 mirror_avg_us=10 mirror_max_us=20 vat[invals=0 arrive->clear_avg_us=0 max_us=0 walks=0 walk_avg_us=0 gpu_avg_us=0 plan_avg_us=0 apply_avg_us=0 leaves=0 host_calls=0 apply_max_ms=0 split_max_ms=0 slow_applies=0 slow_splits=0] mem[inval=0]",
    "kf3: status phase=Running mirrors=3 reused=1 prewarmed=2 mirror_avg_us=10 mirror_max_us=20 vat[invals=0 arrive->clear_avg_us=0 max_us=0 walks=1 walk_avg_us=25094 gpu_avg_us=23115 plan_avg_us=0 apply_avg_us=1813176 leaves=99134 host_calls=4125 apply_max_ms=1813 split_max_ms=1838 slow_applies=83 slow_splits=14 q_inbox_n=2 q_inbox_avg_us=150 q_inbox_max_us=900 submit_n=1 submit_avg_us=40 submit_max_us=90 collect_n=1 collect_avg_us=300 collect_max_us=800 cap_retries=0 req_n=2 req_avg_us=1830000 req_p50_us=2097152 req_p95_us=2097152 req_p99_us=2097152 req_max_ms=1838 aps_n=1 aps_avg_us=1813176 aps_p50_us=2097152 aps_p95_us=2097152 aps_p99_us=2097152 aps_max_ms=1813 prep_n=1 prep_us=20000 prep_max_us=20000 unmapph_n=1 unmapph_us=600000 unmapph_max_us=600000 mapph_n=1 mapph_us=1190000 mapph_max_us=1190000 takedown_n=1 takedown_us=0 takedown_max_us=0 inval_n=1 inval_us=3000 inval_max_us=3000 rm_map_n=30 rm_map_us=1100000 rm_map_max_us=60000 rm_unmap_n=3 rm_unmap_us=550000 rm_unmap_max_us=400000 rm_free_n=2 rm_free_us=1000 rm_free_max_us=600 rm_resv_n=0 rm_resv_us=0 rm_resv_max_us=0 ledger_n=3 ledger_us=40000 ledger_max_us=20000 stitch_n=30 stitch_us=300000 stitch_max_us=20000 desc_n=30 desc_us=200000 desc_max_us=9000 reap_n=30 reap_us=1000 reap_max_us=100 slow_reqs=2 slow_aps=1 windows_max=1 slow_printed=3 slow_suppressed=0 ring_overwritten=0 pub_n=2 pub_avg_us=1830100 pub_max_us=1838100 resume_n=2 resume_avg_us=900 resume_max_us=1500 e2e_n=2 e2e_avg_us=1831000 e2e_p50_us=2097152 e2e_p95_us=2097152 e2e_p99_us=2097152 e2e_max_ms=1839] mem[inval=0]",
    "kf3: VA_SLOW n=1 kind=apply t_ms=1087569 ticket=48826 walk=19293 key=0x12 pdb=0x8000 slot=18 window=1 apply_us=1813176 maps=95266 unmaps=116018 prep_n=1 prep_us=20000 prep_max_us=20000 unmapph_n=1 unmapph_us=600000 unmapph_max_us=600000 mapph_n=1 mapph_us=1190000 mapph_max_us=1190000 takedown_n=1 takedown_us=0 takedown_max_us=0 inval_n=1 inval_us=3000 inval_max_us=3000 rm_map_n=30 rm_map_us=1100000 rm_map_max_us=60000 rm_unmap_n=3 rm_unmap_us=550000 rm_unmap_max_us=400000 rm_free_n=2 rm_free_us=1000 rm_free_max_us=600 rm_resv_n=0 rm_resv_us=0 rm_resv_max_us=0 ledger_n=3 ledger_us=40000 ledger_max_us=20000 stitch_n=30 stitch_us=300000 stitch_max_us=20000 desc_n=30 desc_us=200000 desc_max_us=9000 reap_n=30 reap_us=1000 reap_max_us=100",
    "kf3: VA_SLOW n=2 kind=ticket t_ms=1087600 ticket=48826 walk=19293 key=0x0 pdb=0x8000 windows=1 req_us=1838000 q_inbox_us=900 mgr_us=1837100 submit_us=40 walk_us=25094 collect_us=300 apply_own_us=1813176 apply_all_us=1813176 cap_retries=0 (at teardown)",
    "kf3: walk capacity ceiling geometry: slot 27 region 100+599040 required 620544 pool 2097152 free 1175808 in 3 region(s), largest [700000+566272]; adjacent free: head 0 tail 0 (in-place growth would not fit); neighbours left s3@0+100 right s9@599140+1000; slots held 30",
    "kf3: VA_SLOW SUMMARY recorded=2 held=2 overwritten=0 printed_live=1 suppressed_live=0 dumped_now=1 slow_reqs=2 slow_aps=1 req_max_ms=1838 aps_max_ms=1813 windows_max=1",
]


def self_test():
    res = scan(SAMPLE)
    assert len(res["intervals"]) == 1, res["intervals"]
    iv = res["intervals"][0]
    # the old fields keep their meaning under the old extraction
    old = dict(FIELD.findall(VAT.search(SAMPLE[1]).group(1)))
    for k, v in (("walks", "1"), ("walk_avg_us", "25094"), ("apply_avg_us", "1813176"), ("clear_avg_us", "0"),
                 ("max_us", "0"), ("apply_max_ms", "1813"), ("split_max_ms", "1838"), ("slow_applies", "83")):
        assert old[k] == v, (k, old[k])
    assert iv["req_p99_us"] == 2097152 and iv["rm_unmap_max_us"] == 400000
    a = [r for r in res["slow"] if r["kind"] == "apply"][0]
    assert a["other_us"] == 1813176 - (20000 + 600000 + 1190000 + 0 + 3000), a["other_us"]
    assert a["rm_map_only_us"] == 1100000 - 300000 - 200000 - 1000
    assert a["key"] == 0x12 and a["pdb"] == 0x8000 and a["slot"] == 18
    t = [r for r in res["slow"] if r["kind"] == "ticket"][0]
    assert t["teardown"] is True and t["q_inbox_us"] == 900
    assert res["summary"][0]["recorded"] == 2
    assert "largest [700000+566272]" in res["geometry"][0]["text"]
    assert res["totals"]["rm_map"] == 1100000 and res["totals"]["aps_n"] == 1
    text = render(res, 50)
    assert "rm_map_only_us" in text and "VA_SLOW SUMMARY recorded=2" in text
    # an old log (no new fields) still gives its old columns
    old_only = scan([SAMPLE[1].split(" q_inbox_n=")[0] + "] mem[x=1]"])
    assert old_only["intervals"][0]["walks"] == 1 and "req_n" not in old_only["intervals"][0]
    render(old_only, 5)
    print("va_profile_table self-test: ok")
    return 0


def main(argv):
    if "--self-test" in argv:
        return self_test()
    args = [a for a in argv if not a.startswith("--")]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    top = 50
    if "--top" in argv:
        i = argv.index("--top")
        top = int(argv[i + 1]) if i + 1 < len(argv) else top
        args = [a for a in args if a != argv[i + 1]] if i + 1 < len(argv) else args
    path = args[0] if args else argv[-1]
    with open(path, encoding="utf-8", errors="replace") as f:
        res = scan(f, keep_all="--all" in argv)
    if "--json" in argv:
        json.dump(res, sys.stdout, indent=1, sort_keys=True)
        print()
    else:
        print(render(res, top))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
