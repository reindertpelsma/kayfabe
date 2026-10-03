#!/usr/bin/env python3
"""summarize.py <results-dir>... — one markdown row per app from host.res / guest.res /
guest_isolated.res (+ APPDIG digests compared host vs guest). The LAST row per (side, app) wins.
The guest verdict is the ISOLATED re-run when there is one (a fresh boot with that app alone), and
the batched verdict is shown beside it when they differ.

★ 2026-10-03 (release §I): a managed-memory row that did not pass is scored by its `loud=` class
(`loud_verdict.sh`): EXPECTED_LOUD, KF3_DEFECT or SILENT replace FAIL/TIMEOUT in the guest column
and in the closing counts, so a sanctioned loud failure, a kayfabe defect and a release blocker are
never one bucket. Rows recorded before the hook wrote `loud=` print as before.

★ 2026-10-03 (review of 5af7e644): the BOOT GATE. Every guest boot's silent-twin gate
(`APPS_BOOT_GATE boot=… gate=PASS|FAIL|UNMEASURED …`, appended by apps_matrix.sh from boot_gate.sh)
is read from guest.res and iso/guest.res BY boot_gates.py (the last gate line per boot wins — the
rule apps_matrix.sh's exit status uses too), and a closing line scores the lane:
    BOOT_GATE boots=<n> pass=<p> fail=<f> unmeasured=<u> lane=PASS|FAIL|UNMEASURED
lane=FAIL when any boot is FAIL or UNMEASURED — including a boot that produced app rows but no gate
line at all (its gate never ran: not a pass). lane=UNMEASURED only for results that predate the gate
entirely (no gate line and no row carrying `rc_none=`). A row whose own kf3 slice holds a silent-twin
birth line (`rc_silent_births=` > 0) also shows `/RC_SILENT` after its guest verdict."""
import re, sys, os, collections
import boot_gates  # the ONE rule for APPS_BOOT_GATE lines (beside this script; sys.path[0])

def rows(path):
    out = collections.OrderedDict()
    if not os.path.exists(path): return out
    for line in open(path, errors="replace"):
        if not line.startswith("APPRES "): continue
        kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", line[7:].strip()))
        out[kv.get("app")] = kv
    return out

def gate_verdict(R):
    """The lane's BOOT_GATE line (and the non-PASS boots, one line each) — boot_gates.py's rule, the
    same code apps_matrix.sh exits on (⊘ review of 9390f51c: the two used to read guest.res apart)."""
    head, bad, _lane = boot_gates.verdict(boot_gates.dir_paths(R))
    return head, bad

def digs(path):
    d = {}
    if not os.path.exists(path): return d
    for line in open(path, errors="replace"):
        m = re.match(r"APPDIG side=(\S+) app=(\S+) (?:OUTSHA|DIGEST) (\S+) (\S+)", line)
        if m: d[(m.group(2), m.group(3))] = m.group(4)
    return d

for R in sys.argv[1:]:
    host = rows(f"{R}/host.res"); gb = rows(f"{R}/guest.res"); gi = rows(f"{R}/guest_isolated.res")
    hd = {}
    for line in open(f"{R}/host.res", errors="replace") if os.path.exists(f"{R}/host.res") else []:
        m = re.match(r"APPDIG side=host app=(\S+) (?:OUTSHA|DIGEST) (\S+) (\S+)", line)
        if m: hd[(m.group(1), m.group(2))] = m.group(3)
    gd = digs(f"{R}/guest.dig"); gd.update(digs(f"{R}/iso/guest.dig"))
    print(f"\n### {R}\n")
    print("| app | host | guest (batched) | guest (alone) | guest detail |")
    print("|---|---|---|---|---|")
    cnt = collections.Counter()
    for app in host.keys() | gb.keys():
        pass
    order = list(host.keys()) + [a for a in gb if a not in host]
    for app in order:
        h = host.get(app, {}); b = gb.get(app, {}); i = gi.get(app, {})
        hv = h.get("verdict", "-"); bv = b.get("verdict", "-"); iv = i.get("verdict", "")
        final = iv or bv
        det = i or b
        lc = det.get("loud", "")
        if final not in ("PASS", "-") and lc in ("EXPECTED_LOUD", "KF3_DEFECT", "SILENT"):
            final = lc
            if iv: iv = f"{iv}/{lc}"
            else: bv = f"{bv}/{lc}"
        if det.get("rc_silent_births", "0") not in ("0", "-", ""):
            final = f"{final}/RC_SILENT"
            if iv: iv = f"{iv}/RC_SILENT"
            else: bv = f"{bv}/RC_SILENT"
        detail = ""
        if final not in ("PASS", "-"):
            detail = f"rc={det.get('rc')} {det.get('secs')}s quiet={det.get('quiet')} xid={det.get('guest_xid')} kf3_refusals={det.get('kf3_refusals')} — {det.get('note','')[:90]}"
        for (a, k), v in hd.items():
            if a == app and (a, k) in gd:
                same = gd[(a, k)] == v
                detail += f" {k} digest {'==' if same else '!='} host"
                if not same and final == "PASS": final = "PASS*"
        cnt[(hv, final)] += 1
        print(f"| {app} | {hv} | {bv} | {iv or '-'} | {detail.strip()} |")
    print("\n" + ", ".join(f"host {k[0]} / guest {k[1]}: {v}" for k, v in sorted(cnt.items())))
    if gb or gi:
        head, bad = gate_verdict(R)
        print("\n" + head)
        for b in bad: print(b)
