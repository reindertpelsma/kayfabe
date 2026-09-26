#!/usr/bin/env python3
"""compare.py <resdir> — grade the headless-graphics test set: guest vs bare metal, by CONTENT.

Inputs in <resdir>: host.res/host.dig (bare metal, run 1), host2.res/host2.dig (bare metal, run 2 —
the NOISE FLOOR: a digest that differs between two bare-metal runs is NONDET and is not graded by
equality), guest.res/guest.dig (batched boots) and iso/guest.res/iso/guest.dig (each failing item
re-run alone in a fresh boot — the verdict of record for a failure).

Final verdict per item:
  NOTRUN(host)  bare metal itself did not PASS (or faulted: host_xid>0) — a fact about the workload
  PASS          guest self-check PASS, no host Xid during the item, every graded digest == bare metal
  FAIL(DIFF)    guest self-check PASS but a graded digest differs from bare metal  (a silent wrong answer)
  FAIL(XID)     guest self-check PASS but the host GPU faulted during the item    (a false pass)
  FAIL/TIMEOUT/HANG/GUEST_DEAD/WEDGED   the guest run's own verdict
The LAST row per (side, item) wins within a file; the isolated re-run overrides the batched one."""
import collections, os, re, sys

KV = re.compile(r"(\w+)=((?:(?! \w+=).)*)")

def res(path):
    out = collections.OrderedDict()
    if not os.path.exists(path):
        return out
    for line in open(path, errors="replace"):
        if line.startswith("GSET_RES "):
            kv = dict(KV.findall(line[9:].strip()))
            out[kv.get("item")] = kv
        elif line.startswith("GSET_WEDGE "):
            kv = dict(KV.findall(line[11:].strip()))
            out.setdefault("_wedges", []).append(kv)
    return out

def digs(path):
    d = collections.OrderedDict()
    if not os.path.exists(path):
        return d
    for line in open(path, errors="replace"):
        m = re.match(r"GSET_DIG side=\S+ item=(\S+) key=(\S+) val=(\S+)", line)
        if m:
            d[(m.group(1), m.group(2))] = m.group(3)
    return d

def main(R):
    h1, h2 = res(f"{R}/host.res"), res(f"{R}/host2.res")
    gb, gi = res(f"{R}/guest.res"), res(f"{R}/iso/guest.res")
    hd1, hd2 = digs(f"{R}/host.dig"), digs(f"{R}/host2.dig")
    gd = digs(f"{R}/guest.dig"); gd.update(digs(f"{R}/iso/guest.dig"))
    items = [i for i in list(h1) + [x for x in gb if x not in h1] if i and not i.startswith("_")]
    rows, cnt = [], collections.Counter()
    for it in items:
        h = h1.get(it, {}); g = gi.get(it) or gb.get(it, {})
        hv, gv = h.get("verdict", "-"), g.get("verdict", "-")
        keys = [k for (i, k) in hd1 if i == it]
        nondet = [k for k in keys if (it, k) in hd2 and hd2[(it, k)] != hd1[(it, k)]]
        graded = [k for k in keys if k not in nondet]
        match = [k for k in graded if gd.get((it, k)) == hd1[(it, k)]]
        diff = [k for k in graded if (it, k) in gd and gd[(it, k)] != hd1[(it, k)]]
        absent = [k for k in graded if (it, k) not in gd]
        hx = int(h.get("host_xid", "0") or 0); gx = g.get("host_xid", "-")
        if hv != "PASS" or hx > 0:
            final = "NOTRUN(host)"
        elif gv == "PASS" and gx not in ("-", "0"):
            final = "FAIL(XID)"
        elif gv == "PASS" and diff:
            final = "FAIL(DIFF)"
        elif gv == "PASS" and absent:
            final = "FAIL(ABSENT)"
        else:
            final = gv
        cnt[final] += 1
        dig = f"{len(match)}/{len(graded)} match" + (f", {len(nondet)} nondet" if nondet else "")
        if diff:
            dig += " DIFF:" + ",".join(diff[:4])
        if absent and gv == "PASS":
            dig += " ABSENT:" + ",".join(absent[:4])
        det = ""
        if final != "PASS":
            det = (f"rc={g.get('rc')} {g.get('secs')}s host_xid={gx} guest_xid={g.get('guest_xid','-')} "
                   f"kf3_rc={g.get('kf3_rc','-')} kf3_refusals={g.get('kf3_refusals','-')} — {g.get('note','')[:100]}")
        elif g.get("kf3_refusals", "0") not in ("0", "-"):
            det = f"kf3_refusals={g.get('kf3_refusals')} (see triage)"
        iso = "alone" if it in gi else "batched"
        rows.append(f"| {it} | {hv} | {gv} ({iso}) | {dig} | **{final}** | {det.strip()} |")
    print(f"### {R}\n")
    print("| item | bare metal | guest | digests vs bare metal | verdict | detail |")
    print("|---|---|---|---|---|---|")
    print("\n".join(rows))
    for w in gb.get("_wedges", []) + gi.get("_wedges", []):
        print(f"\n⊘ boot {w.get('boot')} WEDGED after {w.get('after')}")
    print("\n" + ", ".join(f"{k}: {v}" for k, v in sorted(cnt.items())))
    npass = cnt["PASS"]; nrun = len(items) - cnt["NOTRUN(host)"]
    print(f"GSET_SUITE_SUMMARY pass={npass}/{nrun} notrun_host={cnt['NOTRUN(host)']}")

if __name__ == "__main__":
    for R in sys.argv[1:]:
        main(R)
